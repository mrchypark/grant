package api

import (
	"testing"

	"github.com/Conalog/grant/internal/grant"
)

// Every business route must have a catalog contract. Adding an internal route
// requires an explicit exclusion instead of silently omitting management APIs.
func TestManagementCatalogCoversRegisteredRoutes(t *testing.T) {
	excluded := map[string]bool{
		"POST /api/v1/admin/bootstrap/rauthy-owner":          true,
		"POST /api/v1/admin/bootstrap/external-owner":        true,
		"POST /api/v1/authz/check":                           true,
		"POST /api/v1/authz/batch-check":                     true,
		"POST /api/v1/authz/filter":                          true,
		"POST /api/v1/application-authz/check":               true,
		"POST /api/v1/application-authz/batch-check":         true,
		"POST /api/v1/application-authz/filter":              true,
		"GET /api/v2/http-capabilities/catalog/{providerId}": true,
		"POST /api/v2/http-capabilities/decide":              true,
	}
	catalog := make(map[string]bool)
	for _, route := range grant.ManagementRoutes() {
		key := route.Method + " " + route.Path
		if catalog[key] || excluded[key] {
			t.Errorf("duplicate or internal management route: %s", key)
		}
		catalog[key] = true
	}
	for _, pattern := range New(nil, false).patterns {
		if excluded[pattern] {
			delete(excluded, pattern)
			continue
		}
		if !catalog[pattern] {
			t.Errorf("business API missing from management catalog: %s", pattern)
		}
		delete(catalog, pattern)
	}
	for pattern := range catalog {
		t.Errorf("catalog exposes an unregistered route: %s", pattern)
	}
	for pattern := range excluded {
		t.Errorf("stale internal route exclusion: %s", pattern)
	}
}
