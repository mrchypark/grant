package grant

import (
	"fmt"
	"sort"
	"strings"
)

// ManagementRoute describes a single management API endpoint for the
// grant-management provider catalog. Path stores the Go 1.22+ ServeMux
// path template (e.g. "/api/v1/orgs/{organizationId}/apps"). The registry
// is the explicit enumeration of every business management route exposed
// by the server; no implicit grants exist outside this list.
type ManagementRoute struct {
	Method     string `json:"method"`
	Path       string `json:"path"`
	ActionCode string `json:"actionCode"`
	Platform   bool   `json:"platform"`
}

// managementRoutes is the canonical sorted registry of management routes.
// Changes to server routes MUST be reflected here; the test inventory
// enforces completeness against an explicit exclusion list.
var managementRoutes = []ManagementRoute{
	// --- Service (me/providers) ---
	{Method: "GET", Path: "/api/v1/me", ActionCode: "user.read"},
	// The action code remains user.materialize for stored-permission compatibility.
	{Method: "POST", Path: "/api/v1/me/ensure", ActionCode: "user.materialize"},
	{Method: "GET", Path: "/api/v1/providers", ActionCode: "provider.list"},
	{Method: "GET", Path: "/api/v1/orgs", ActionCode: "organization.list"},
	{Method: "POST", Path: "/api/v1/orgs", ActionCode: "organization.create"},
	{Method: "GET", Path: "/api/v1/orgs/{organizationId}/owner", ActionCode: "organization.owner.read"},
	{Method: "PUT", Path: "/api/v1/orgs/{organizationId}/owner", ActionCode: "organization.owner.transfer"},
	{Method: "GET", Path: "/api/v1/orgs/{organizationId}", ActionCode: "organization.detail"},
	{Method: "PATCH", Path: "/api/v1/orgs/{organizationId}", ActionCode: "organization.update"},
	{Method: "POST", Path: "/api/v1/admin/orgs", ActionCode: "admin.organization.create", Platform: true},

	// --- Organization: applications ---
	{Method: "GET", Path: "/api/v1/orgs/{organizationId}/apps", ActionCode: "applications.read"},
	{Method: "POST", Path: "/api/v1/orgs/{organizationId}/apps", ActionCode: "applications.create"},
	{Method: "GET", Path: "/api/v1/orgs/{organizationId}/apps/{applicationId}", ActionCode: "applications.detail"},
	{Method: "PATCH", Path: "/api/v1/orgs/{organizationId}/apps/{applicationId}", ActionCode: "applications.update"},
	{Method: "PUT", Path: "/api/v1/orgs/{organizationId}/apps/{applicationId}/active", ActionCode: "applications.active"},
	{Method: "DELETE", Path: "/api/v1/orgs/{organizationId}/apps/{applicationId}", ActionCode: "applications.delete"},
	{Method: "GET", Path: "/api/v1/orgs/{organizationId}/apps/{applicationId}/actions", ActionCode: "applications.actions.read"},
	{Method: "PUT", Path: "/api/v1/orgs/{organizationId}/apps/{applicationId}/actions", ActionCode: "applications.actions.replace"},
	{Method: "PUT", Path: "/api/v1/orgs/{organizationId}/apps/{applicationId}/actions/{actionId}", ActionCode: "applications.actions.add"},
	{Method: "DELETE", Path: "/api/v1/orgs/{organizationId}/apps/{applicationId}/actions/{actionId}", ActionCode: "applications.actions.remove"},
	{Method: "GET", Path: "/api/v1/orgs/{organizationId}/app-grants", ActionCode: "applications.grants.read"},
	{Method: "DELETE", Path: "/api/v1/orgs/{organizationId}/app-grants/{grantId}", ActionCode: "applications.grants.revoke"},
	{Method: "GET", Path: "/api/v1/orgs/{organizationId}/apps/{applicationId}/consent", ActionCode: "applications.consent.read"},
	{Method: "PUT", Path: "/api/v1/orgs/{organizationId}/apps/{applicationId}/consent", ActionCode: "applications.consent.replace"},
	{Method: "DELETE", Path: "/api/v1/orgs/{organizationId}/apps/{applicationId}/consent", ActionCode: "applications.consent.delete"},

	// --- Organization: groups ---
	{Method: "GET", Path: "/api/v1/orgs/{organizationId}/groups", ActionCode: "groups.read"},
	{Method: "POST", Path: "/api/v1/orgs/{organizationId}/groups", ActionCode: "groups.create"},
	{Method: "GET", Path: "/api/v1/orgs/{organizationId}/groups/{groupId}", ActionCode: "groups.detail"},
	{Method: "PUT", Path: "/api/v1/orgs/{organizationId}/groups/{groupId}", ActionCode: "groups.update"},
	{Method: "PATCH", Path: "/api/v1/orgs/{organizationId}/groups/{groupId}", ActionCode: "groups.patch"},
	{Method: "DELETE", Path: "/api/v1/orgs/{organizationId}/groups/{groupId}", ActionCode: "groups.delete"},
	{Method: "GET", Path: "/api/v1/orgs/{organizationId}/groups/{groupId}/members", ActionCode: "groups.members.read"},
	{Method: "PUT", Path: "/api/v1/orgs/{organizationId}/groups/{groupId}/members/{userId}", ActionCode: "groups.members.add"},
	{Method: "DELETE", Path: "/api/v1/orgs/{organizationId}/groups/{groupId}/members/{userId}", ActionCode: "groups.members.remove"},

	// --- Organization: members ---
	{Method: "PUT", Path: "/api/v1/orgs/{organizationId}/members/{userId}", ActionCode: "members.enroll"},
	{Method: "GET", Path: "/api/v1/orgs/{organizationId}/members", ActionCode: "members.read"},
	{Method: "DELETE", Path: "/api/v1/orgs/{organizationId}/members/{userId}", ActionCode: "members.remove"},

	// --- Organization: agreements ---
	{Method: "GET", Path: "/api/v1/orgs/{organizationId}/agreements", ActionCode: "agreements.read"},
	{Method: "POST", Path: "/api/v1/orgs/{organizationId}/agreements", ActionCode: "agreements.create"},
	{Method: "GET", Path: "/api/v1/orgs/{organizationId}/agreements/{agreementId}", ActionCode: "agreements.detail"},
	{Method: "PUT", Path: "/api/v1/orgs/{organizationId}/agreements/{agreementId}", ActionCode: "agreements.update"},
	{Method: "PATCH", Path: "/api/v1/orgs/{organizationId}/agreements/{agreementId}", ActionCode: "agreements.patch"},

	// --- Organization: roles ---
	{Method: "GET", Path: "/api/v1/orgs/{organizationId}/roles", ActionCode: "roles.read"},
	{Method: "POST", Path: "/api/v1/orgs/{organizationId}/roles", ActionCode: "roles.create"},
	{Method: "PATCH", Path: "/api/v1/orgs/{organizationId}/roles/{roleId}", ActionCode: "role.update"},
	{Method: "DELETE", Path: "/api/v1/orgs/{organizationId}/roles/{roleId}", ActionCode: "role.delete"},
	{Method: "PUT", Path: "/api/v1/orgs/{organizationId}/roles/{roleId}/actions", ActionCode: "role.actions.replace"},
	{Method: "GET", Path: "/api/v1/orgs/{organizationId}/roles/{roleId}/actions", ActionCode: "role.actions.read"},
	{Method: "PUT", Path: "/api/v1/orgs/{organizationId}/roles/{roleId}/actions/{actionId}", ActionCode: "role.actions.add"},
	{Method: "DELETE", Path: "/api/v1/orgs/{organizationId}/roles/{roleId}/actions/{actionId}", ActionCode: "role.actions.remove"},

	// --- Organization: bindings ---
	{Method: "GET", Path: "/api/v1/orgs/{organizationId}/bindings", ActionCode: "bindings.read"},
	{Method: "POST", Path: "/api/v1/orgs/{organizationId}/bindings", ActionCode: "bindings.create"},
	{Method: "DELETE", Path: "/api/v1/orgs/{organizationId}/bindings/{bindingId}", ActionCode: "bindings.delete"},

	// --- Organization: lifecycle ---
	{Method: "POST", Path: "/api/v1/orgs/{organizationId}/deactivate", ActionCode: "organization.deactivate"},
	{Method: "POST", Path: "/api/v1/orgs/{organizationId}/reactivate", ActionCode: "organization.reactivate"},

	// --- Admin: providers ---
	{Method: "POST", Path: "/api/v1/admin/providers", ActionCode: "admin.provider.create", Platform: true},
	{Method: "PATCH", Path: "/api/v1/admin/providers/{providerId}", ActionCode: "admin.provider.update", Platform: true},
	{Method: "PUT", Path: "/api/v1/admin/providers/{providerId}/actions", ActionCode: "admin.provider.actions.replace", Platform: true},
	{Method: "GET", Path: "/api/v1/admin/providers/{providerId}/actions", ActionCode: "admin.provider.actions.read", Platform: true},
	{Method: "POST", Path: "/api/v1/admin/providers/{providerId}/actions", ActionCode: "admin.provider.actions.create", Platform: true},
	{Method: "PATCH", Path: "/api/v1/admin/providers/{providerId}/actions/{actionId}", ActionCode: "admin.provider.actions.update", Platform: true},
	{Method: "DELETE", Path: "/api/v1/admin/providers/{providerId}/actions/{actionId}", ActionCode: "admin.provider.actions.delete", Platform: true},

	// --- Admin: org providers (authorized user) ---
	{Method: "GET", Path: "/api/v1/orgs/{organizationId}/providers/{providerId}/actions", ActionCode: "admin.org.providers.actions.read"},

	// --- Admin: capability catalog ---
	{Method: "GET", Path: "/api/v1/admin/providers/{providerId}/http-capability-active", ActionCode: "admin.catalog.read", Platform: true},
	{Method: "POST", Path: "/api/v1/admin/providers/{providerId}/http-capability-generations", ActionCode: "admin.catalog.stage", Platform: true},
	{Method: "POST", Path: "/api/v1/admin/providers/{providerId}/http-capability-active", ActionCode: "admin.catalog.activate", Platform: true},

	// --- Admin: app trust/identity binding ---
	{Method: "PUT", Path: "/api/v1/admin/orgs/{organizationId}/apps/{applicationId}/trust", ActionCode: "admin.app.trust", Platform: true},
	{Method: "PUT", Path: "/api/v1/admin/orgs/{organizationId}/apps/{applicationId}/identity-binding", ActionCode: "admin.app.identity.bind", Platform: true},

	// --- Admin: user active ---
	{Method: "PUT", Path: "/api/v1/admin/users/{userId}/active", ActionCode: "admin.user.active", Platform: true},

	// Append new routes to preserve existing capability IDs, which include the index.
	{Method: "GET", Path: "/api/v1/admin/users", ActionCode: "admin.users.read", Platform: true},
	{Method: "GET", Path: "/api/v1/admin/providers/{providerId}/http-capability-generations/{generation}", ActionCode: "admin.catalog.read", Platform: true},
	{Method: "GET", Path: "/api/v1/orgs/{organizationId}/owners", ActionCode: "owners.read"},
	{Method: "PUT", Path: "/api/v1/orgs/{organizationId}/owners/{userId}", ActionCode: "owners.add"},
	{Method: "DELETE", Path: "/api/v1/orgs/{organizationId}/owners/{userId}", ActionCode: "owners.remove"},
	{Method: "POST", Path: "/api/v1/orgs/{organizationId}/providers/{providerId}/actions", ActionCode: "custom.actions.create"},
	{Method: "PATCH", Path: "/api/v1/orgs/{organizationId}/providers/{providerId}/actions/{actionId}", ActionCode: "custom.actions.update"},
	{Method: "DELETE", Path: "/api/v1/orgs/{organizationId}/providers/{providerId}/actions/{actionId}", ActionCode: "custom.actions.delete"},
}

// ManagementRoutes returns a copy of the complete sorted registry of
// management API routes. Callers must not modify the returned slice.
func ManagementRoutes() []ManagementRoute {
	out := make([]ManagementRoute, len(managementRoutes))
	copy(out, managementRoutes)
	return out
}

// LookupManagementRoute returns the ManagementRoute matching the given
// HTTP method and path template. Path values use Go 1.22+ ServeMux
// conventions (e.g. "/api/v1/orgs/{organizationId}/apps").
func LookupManagementRoute(method, path string) (ManagementRoute, bool) {
	for _, r := range managementRoutes {
		if r.Method == method && r.Path == path {
			return r, true
		}
	}
	return ManagementRoute{}, false
}

// ManagementCatalog produces an immutable StageHttpCapabilityGeneration and
// the corresponding action definitions for every registered management
// route. The generation parameter must be 1 without a previous generation,
// or previous+1 otherwise. The provider is "grant-management".
func ManagementCatalog(generation uint64, previous *uint64) (StageHttpCapabilityGeneration, []Action, error) {
	stage := StageHttpCapabilityGeneration{SchemaVersion: 1, Generation: generation, ExpectedLatestGeneration: previous}
	if generation == 0 || (previous == nil && generation != 1) || (previous != nil && (*previous == 0 || generation <= *previous || generation-*previous != 1)) {
		return stage, nil, fmt.Errorf("generation must be 1 without a previous generation, otherwise previous + 1")
	}
	const provider = "grant-management"
	routes := ManagementRoutes()
	actions := make([]Action, 0, len(routes)+1)
	seenActions := make(map[string]bool, len(routes))
	for i, route := range routes {
		actionID := provider + "." + route.ActionCode
		name := route.ActionCode
		var description *string
		if route.Method == "POST" && route.Path == "/api/v1/me/ensure" && route.ActionCode == "user.materialize" {
			name = "Ensure current user"
			detail := "Create or reuse the authenticated external identity's internal user and scoped identity mapping without changing profiles, memberships, or permissions."
			description = &detail
		}
		if !seenActions[actionID] {
			seenActions[actionID] = true
			actions = append(actions, Action{
				ID:                      actionID,
				ProviderID:              provider,
				Code:                    route.ActionCode,
				Name:                    name,
				Description:             description,
				RiskLevel:               ActionRiskLevelHigh,
				GrantableToApplications: true,
			})
		}

		resource := buildManagementResource(route)
		c := HttpCapability{
			CapabilityID: fmt.Sprintf("management.%s.%d", route.ActionCode, i),
			Version:      1,
			Enabled:      true,
			AccessMode:   "authorized",
			Method:       route.Method,
			PathTemplate: route.Path,
			ActionID:     &actionID,
			Resource:     &resource,
			Query:        getReadQuery(route.Method),
			Request:      buildManagementRequest(route),
			Response: HttpResponsePolicy{
				MaxBytes:       4 << 20,
				ForwardHeaders: []string{"content-type", "etag"},
				Redirects:      "deny",
				ErrorStatuses:  []int{400, 401, 403, 404, 409, 412, 422, 428, 500, 503},
			},
		}
		if route.Method == "GET" && route.Path == "/api/v1/admin/users" {
			c.Query.Parameters = append(c.Query.Parameters,
				HttpQueryParameter{Name: "cursor", Format: "text", MaxBytes: 1024},
				HttpQueryParameter{Name: "limit", Format: "text", MaxBytes: 20},
				HttpQueryParameter{Name: "q", Format: "text", MaxBytes: 1024})
		}
		var err error
		c.ContractDigest, err = HttpCapabilityContractDigest(c)
		if err != nil {
			return stage, nil, err
		}
		stage.Capabilities = append(stage.Capabilities, c)
	}

	// Deprecated legacy "manage" action for backwards catalog transition.
	// No routes use this action code.
	actions = append(actions, Action{
		ID:                      provider + ".manage",
		ProviderID:              provider,
		Code:                    "manage",
		Name:                    "Grant management groups",
		RiskLevel:               ActionRiskLevelHigh,
		GrantableToApplications: true,
	})

	var err error
	stage.CatalogDigest, err = HttpCapabilityCatalogDigest(provider, stage)
	if err == nil {
		err = validateGeneration(provider, stage)
	}
	return stage, actions, err
}

// buildManagementResource returns the resource extractor for a management
// route based on its path structure and platform flag.
func buildManagementResource(route ManagementRoute) HttpCapabilityResource {
	if route.Platform {
		return HttpCapabilityResource{ResourceType: "platform", ID: HttpResourceExtractor{Kind: "constant", Value: "grant"}}
	}
	if strings.Contains(route.Path, "{organizationId}") {
		return HttpCapabilityResource{ResourceType: "organization", ID: HttpResourceExtractor{Kind: "pathSegment", Name: "organizationId", Format: "asciiToken"}}
	}
	return HttpCapabilityResource{ResourceType: "service", ID: HttpResourceExtractor{Kind: "constant", Value: "grant"}}
}

// getReadQuery returns the query policy for GET routes that support the
// readConsistency parameter, or the zero value for non-GET routes.
func getReadQuery(method string) HttpQueryPolicy {
	if method != "GET" {
		return HttpQueryPolicy{Mode: "none"}
	}
	return HttpQueryPolicy{
		Mode: "exact",
		Parameters: []HttpQueryParameter{
			{Name: "consistency", Format: "text", MaxBytes: 32},
		},
	}
}

// buildManagementRequest returns the request policy for a management route.
// Routes that accept JSON request bodies (POST/PUT with body) use json with
// 2 MiB limit. Bodyless routes (GET, DELETE, bodyless POST/PUT) use none.
// Every route forwards x-organization-id; JSON routes also forward content-type.
func buildManagementRequest(route ManagementRoute) HttpRequestPolicy {
	headers := []string{"x-organization-id"}
	if route.Method == "PUT" && (strings.HasSuffix(route.Path, "/actions") || strings.HasSuffix(route.Path, "/consent") || strings.HasSuffix(route.Path, "/owner")) {
		headers = append(headers, "if-match")
		if strings.HasSuffix(route.Path, "/consent") {
			headers = append(headers, "if-none-match")
		}
	}
	sort.Strings(headers)
	hasBody := (route.Method == "POST" || route.Method == "PUT" || route.Method == "PATCH") && isBodyRoute(route.Path)
	if hasBody {
		return HttpRequestPolicy{
			Body:           "json",
			MaxBytes:       2 << 20,
			ForwardHeaders: append([]string{"content-type"}, headers...),
			Authorization:  "forward-user",
			ContentTypes:   []string{"application/json"},
		}
	}
	return HttpRequestPolicy{
		Body:           "none",
		ForwardHeaders: headers,
		Authorization:  "forward-user",
	}
}

// isBodyRoute returns true for POST/PUT paths that accept a JSON request body.
// Bodyless POST paths (ensure, deactivate, reactivate, verify, backups)
// return false.
func isBodyRoute(path string) bool {
	switch path {
	case "/api/v1/me/ensure",
		"/api/v1/orgs/{organizationId}/deactivate",
		"/api/v1/orgs/{organizationId}/reactivate",
		"/api/v1/orgs/{organizationId}/members/{userId}",
		"/api/v1/orgs/{organizationId}/owners/{userId}",
		"/api/v1/orgs/{organizationId}/groups/{groupId}/members/{userId}",
		"/api/v1/orgs/{organizationId}/roles/{roleId}/actions/{actionId}",
		"/api/v1/orgs/{organizationId}/apps/{applicationId}/actions/{actionId}":
		return false
	}
	return true
}
