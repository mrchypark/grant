package api

import (
	"context"
	"github.com/mrchypark/grant/internal/grant"
	"net/http/httptest"
	"testing"
)

func TestFirstPartyAdmissionMasterOnly(t *testing.T) {
	s, store := managementTestServerWithStore(t)
	if _, err := store.Exec(context.Background(), `UPDATE applications SET first_party=1,consent_policy='auto_approve' WHERE id='app-1'`); err != nil {
		t.Fatal(err)
	}
	const path = "/api/v1/admin/orgs/mgmt-org/apps/app-1/actions/grant-management.admin.provider.create"
	for _, tc := range []struct {
		name, master, bearer string
		status               int
	}{
		{"unauthenticated", "", "", 401}, {"bearer", "", "tok", 401}, {"invalid-master", "wrong", "tok", 401}, {"master", "master-secret", "", 204},
	} {
		t.Run(tc.name, func(t *testing.T) {
			r := httptest.NewRequest("PUT", path, nil)
			if tc.master != "" {
				r.Header.Set("X-Grant-Master-Key", tc.master)
			}
			if tc.bearer != "" {
				r.Header.Set("Authorization", "Bearer "+tc.bearer)
			}
			r.Header.Set("X-Organization-ID", "mgmt-org")
			w := httptest.NewRecorder()
			s.ServeHTTP(w, r)
			if w.Code != tc.status {
				t.Fatalf("status %d: %s", w.Code, w.Body.String())
			}
		})
	}
	if _, ok := grant.LookupManagementRoute("PUT", "/api/v1/admin/orgs/{organizationId}/apps/{applicationId}/actions/{actionId}"); ok {
		t.Fatal("operational endpoint entered management catalog")
	}
	if n := len(grant.ManagementRoutes()); n != 71 {
		t.Fatalf("management routes=%d", n)
	}
}
