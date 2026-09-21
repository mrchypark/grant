package grant

import (
	"reflect"
	"strings"
	"testing"
)

func TestManagementCatalogDeterministicDigest(t *testing.T) {
	a, aActions, err := ManagementCatalog(1, nil)
	if err != nil {
		t.Fatal(err)
	}
	b, bActions, err := ManagementCatalog(1, nil)
	if err != nil {
		t.Fatal(err)
	}
	if !reflect.DeepEqual(a, b) {
		t.Fatal("nondeterministic stage output")
	}
	if !reflect.DeepEqual(aActions, bActions) {
		t.Fatal("nondeterministic action output")
	}
}

func TestManagementCatalogGenerationTransition(t *testing.T) {
	if _, _, err := ManagementCatalog(0, nil); err == nil {
		t.Fatal("zero generation accepted")
	}
	previous := uint64(1)
	for _, input := range []struct {
		generation uint64
		previous   *uint64
	}{
		{2, nil},
		{3, &previous},
	} {
		if _, _, err := ManagementCatalog(input.generation, input.previous); err == nil {
			t.Fatal("nonconsecutive generation accepted")
		}
	}
	if _, _, err := ManagementCatalog(2, &previous); err != nil {
		t.Fatal("consecutive generation rejected:", err)
	}
}

func TestManagementCatalogAllActionsGrantable(t *testing.T) {
	_, actions, err := ManagementCatalog(1, nil)
	if err != nil {
		t.Fatal(err)
	}
	for _, a := range actions {
		if !a.GrantableToApplications {
			t.Fatalf("action %s is not grantable to applications", a.ID)
		}
	}
}

func TestManagementCatalogLegacyManageRetained(t *testing.T) {
	_, actions, err := ManagementCatalog(1, nil)
	if err != nil {
		t.Fatal(err)
	}
	var found bool
	for _, a := range actions {
		if a.Code == "manage" {
			found = true
			if a.ID != "grant-management.manage" {
				t.Fatalf("legacy manage action ID = %q, want grant-management.manage", a.ID)
			}
			if a.ProviderID != "grant-management" {
				t.Fatalf("legacy manage provider = %q, want grant-management", a.ProviderID)
			}
			if !a.GrantableToApplications {
				t.Fatal("legacy manage action not grantable to applications")
			}
		}
	}
	if !found {
		t.Fatal("deprecated manage action not retained in actions slice")
	}
}

func TestManagementCatalogPlatformScoped(t *testing.T) {
	stage, _, err := ManagementCatalog(1, nil)
	if err != nil {
		t.Fatal(err)
	}
	for _, c := range stage.Capabilities {
		if c.Resource == nil {
			t.Fatalf("capability %s has nil resource", c.CapabilityID)
		}
		route, ok := LookupManagementRoute(c.Method, c.PathTemplate)
		if !ok {
			t.Fatalf("capability %s has no matching route", c.CapabilityID)
		}
		if route.Platform {
			if c.Resource.ResourceType != "platform" || c.Resource.ID.Kind != "constant" || c.Resource.ID.Value != "grant" {
				t.Fatalf("platform route %s %s resource = %+v, want platform/constant/grant", c.Method, c.PathTemplate, c.Resource)
			}
		} else if strings.Contains(c.PathTemplate, "{organizationId}") {
			if c.Resource.ResourceType != "organization" || c.Resource.ID.Kind != "pathSegment" || c.Resource.ID.Name != "organizationId" || c.Resource.ID.Format != "asciiToken" {
				t.Fatalf("org route %s %s resource = %+v, want organization/pathSegment/organizationId/asciiToken", c.Method, c.PathTemplate, c.Resource)
			}
		} else {
			if c.Resource.ResourceType != "service" || c.Resource.ID.Kind != "constant" || c.Resource.ID.Value != "grant" {
				t.Fatalf("service route %s %s resource = %+v, want service/constant/grant", c.Method, c.PathTemplate, c.Resource)
			}
		}
	}
}

func TestManagementCatalogHeaders(t *testing.T) {
	stage, _, err := ManagementCatalog(1, nil)
	if err != nil {
		t.Fatal(err)
	}
	for _, c := range stage.Capabilities {
		for _, h := range c.Request.ForwardHeaders {
			if h != "content-type" && h != "x-organization-id" && h != "if-match" && h != "if-none-match" {
				t.Fatalf("capability %s forwards unexpected header %q", c.CapabilityID, h)
			}
		}
		found := false
		for _, h := range c.Request.ForwardHeaders {
			if h == "x-organization-id" {
				found = true
			}
		}
		if !found {
			t.Fatalf("capability %s missing x-organization-id in forwardHeaders", c.CapabilityID)
		}
	}
}

func TestManagementCatalogBodylessCalls(t *testing.T) {
	stage, _, err := ManagementCatalog(1, nil)
	if err != nil {
		t.Fatal(err)
	}
	bodyless := map[string]bool{
		"/api/v1/me/materialize":                                                       true,
		"/api/v1/orgs/{organizationId}/deactivate":                                     true,
		"/api/v1/orgs/{organizationId}/reactivate":                                     true,
		"/api/v1/admin/orgs/{organizationId}/apps/{applicationId}/provisioning/verify": true,
		"/api/v1/orgs/{organizationId}/members/{userId}":                               true,
		"/api/v1/orgs/{organizationId}/groups/{groupId}/members/{userId}":              true,
		"/api/v1/orgs/{organizationId}/roles/{roleId}/actions/{actionId}":              true,
		"/api/v1/orgs/{organizationId}/apps/{applicationId}/actions/{actionId}":        true,
	}
	for _, c := range stage.Capabilities {
		if bodyless[c.PathTemplate] {
			if c.Request.Body != "none" {
				t.Fatalf("bodyless route %s has body %q, want none", c.PathTemplate, c.Request.Body)
			}
			if c.Request.MaxBytes != 0 {
				t.Fatalf("bodyless route %s has maxBytes %d, want 0", c.PathTemplate, c.Request.MaxBytes)
			}
		}
	}
}

func TestManagementCatalogRouteInventory(t *testing.T) {
	routes := ManagementRoutes()
	if len(routes) == 0 {
		t.Fatal("empty route registry")
	}
	seen := map[string]bool{}
	for _, r := range routes {
		key := r.Method + " " + r.Path
		if seen[key] {
			t.Fatalf("duplicate route: %s", key)
		}
		seen[key] = true
	}
	// Verify a few critical routes exist.
	for _, want := range []string{
		"GET /api/v1/me",
		"POST /api/v1/me/materialize",
		"GET /api/v1/providers",
		"POST /api/v1/orgs/{organizationId}/deactivate",
		"POST /api/v1/orgs/{organizationId}/reactivate",
		"PATCH /api/v1/admin/providers/{providerId}/actions/{actionId}",
		"PUT /api/v1/admin/users/{userId}/active",
	} {
		if !seen[want] {
			t.Fatalf("expected route %s not in registry", want)
		}
	}
}

func TestManagementCatalogAsciiTokenOrgFormat(t *testing.T) {
	stage, _, err := ManagementCatalog(1, nil)
	if err != nil {
		t.Fatal(err)
	}
	for _, c := range stage.Capabilities {
		if c.Resource == nil || c.Resource.ResourceType != "organization" {
			continue
		}
		if c.Resource.ID.Format != "asciiToken" {
			t.Fatalf("capability %s organization format = %q, want asciiToken", c.CapabilityID, c.Resource.ID.Format)
		}
	}
	// Validate asciiToken accepts transport identifiers like Ops_1 and tenant.EU:42.
	for _, id := range []string{"Ops_1", "tenant.EU:42"} {
		if err := scalarFormat("asciiToken", id); err != nil {
			t.Fatalf("asciiToken scalarFormat(%q) rejected: %v", id, err)
		}
	}
	// Validate slug rejects these identifiers.
	for _, id := range []string{"Ops_1", "tenant.EU:42"} {
		if err := scalarFormat("slug", id); err == nil {
			t.Fatalf("slug scalarFormat(%q) accepted, expected rejection", id)
		}
	}
}

func TestManagementCatalogGetReadQueryConsistency(t *testing.T) {
	stage, _, err := ManagementCatalog(1, nil)
	if err != nil {
		t.Fatal(err)
	}
	for _, c := range stage.Capabilities {
		if c.Method == "GET" {
			if c.Query.Mode != "exact" {
				t.Fatalf("GET capability %s query mode = %q, want exact", c.CapabilityID, c.Query.Mode)
			}
			if len(c.Query.Parameters) != 1 || c.Query.Parameters[0].Name != "consistency" || c.Query.Parameters[0].Format != "text" || c.Query.Parameters[0].MaxBytes != 32 {
				t.Fatalf("GET capability %s query parameters = %+v, want [consistency text 32]", c.CapabilityID, c.Query.Parameters)
			}
		} else {
			if c.Query.Mode != "none" {
				t.Fatalf("non-GET capability %s query mode = %q, want none", c.CapabilityID, c.Query.Mode)
			}
		}
	}
}

func TestManagementCatalogAsciiTokenPathMatch(t *testing.T) {
	stage, _, err := ManagementCatalog(1, nil)
	if err != nil {
		t.Fatal(err)
	}
	for _, c := range stage.Capabilities {
		if c.Resource == nil || c.Resource.ResourceType != "organization" {
			continue
		}
		// Build a real path with Ops_1 as org and a dummy for other vars.
		path := c.PathTemplate
		path = strings.Replace(path, "{organizationId}", "Ops_1", 1)
		for strings.Contains(path, "{") {
			start := strings.Index(path, "{")
			end := strings.Index(path[start:], "}") + start
			path = path[:start] + "dummy" + path[end+1:]
		}
		values, err := matchPolicyPath(c.PathTemplate, path, nil)
		if err != nil {
			t.Fatalf("matchPolicyPath %s with Ops_1 rejected: %v", c.PathTemplate, err)
		}
		if err := scalarFormat(c.Resource.ID.Format, values["organizationId"]); err != nil {
			t.Fatalf("organization resource extraction rejected Ops_1: %v", err)
		}
	}
}

func TestManagementCatalogConsistencyQueryVariants(t *testing.T) {
	stage, _, err := ManagementCatalog(1, nil)
	if err != nil {
		t.Fatal(err)
	}
	// Pick the first GET capability (service-scoped, no path vars to fill).
	var getCap HttpCapability
	for _, c := range stage.Capabilities {
		if c.Method == "GET" && c.Resource.ResourceType == "service" {
			getCap = c
			break
		}
	}
	if getCap.CapabilityID == "" {
		t.Fatal("no GET service capability found")
	}
	for query, want := range map[string]string{"": "", "consistency=strong": "strong", "consistency=%20Strong%20": " Strong "} {
		values, err := canonicalQuery(getCap.Query, query)
		if err != nil || values["consistency"] != want {
			t.Fatalf("query %q: values=%v err=%v", query, values, err)
		}
	}
	for _, query := range []string{"unknown=x", "consistency=strong&consistency=strong", "consistency=" + strings.Repeat("s", 33)} {
		if _, err := canonicalQuery(getCap.Query, query); err == nil {
			t.Fatalf("invalid query accepted: %q", query)
		}
	}
	for _, c := range stage.Capabilities {
		if c.Method != "GET" {
			if _, err := canonicalQuery(c.Query, "consistency=strong"); err == nil {
				t.Fatalf("mutation %s accepted query", c.CapabilityID)
			}
		}
	}
}
