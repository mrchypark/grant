package api

import (
	"context"
	"net/http/httptest"
	"strings"
	"testing"
)

func TestOwnershipHTTPRequiresOwnerAndMatchingETag(t *testing.T) {
	s := managementTestServer(t)
	// Bootstrap fixture user already belongs to mgmt-org. Create a separate
	// organization whose ordinary membership can be promoted later in service tests.
	before := lifecycleCall(t, s, "GET", "/api/v1/orgs/owner/owner", "", "owner", false, 200)
	tag := before.Header().Get("ETag")
	if tag == "" {
		t.Fatal("missing ETag")
	}
	call := func(payload, match string, want int) {
		t.Helper()
		r := httptest.NewRequest("PUT", "/api/v1/orgs/owner/owner", strings.NewReader(payload))
		r.Header.Set("Authorization", "Bearer tok")
		r.Header.Set("X-Organization-ID", "owner")
		r.Header.Set("Content-Type", "application/json")
		if match != "" {
			r.Header.Set("If-Match", match)
		}
		w := httptest.NewRecorder()
		s.ServeHTTP(w, r)
		if w.Code != want {
			t.Fatalf("%s: %d %s", payload, w.Code, w.Body.String())
		}
	}
	call(`{"ownerUserId":"owner"}`, "", 428)
	call(`{"ownerUserId":"owner"}`, `"stale"`, 412)
	call(`{"ownerUserId":"owner"}`, tag, 400)
	call(`{"ownerUserId":null}`, tag, 422)
	call(`{"ownerUserId":"owner","organizationId":"other"}`, tag, 422)
	if _, err := s.service.GetOrganizationOwnerAuthorized(context.Background(), "owner", "missing"); err == nil {
		t.Fatal("cross organization accepted")
	}
}
