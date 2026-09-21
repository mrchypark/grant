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
			if c.Resource.ResourceType != "organization" || c.Resource.ID.Kind != "pathSegment" || c.Resource.ID.Name != "organizationId" {
				t.Fatalf("org route %s %s resource = %+v, want organization/pathSegment/organizationId", c.Method, c.PathTemplate, c.Resource)
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
			if h != "content-type" && h != "x-organization-id" {
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
		"/api/v1/admin/backups":                                                        true,
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
		"POST /api/v1/admin/backups",
		"PUT /api/v1/admin/users/{userId}/active",
	} {
		if !seen[want] {
			t.Fatalf("expected route %s not in registry", want)
		}
	}
}
