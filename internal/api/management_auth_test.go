package api

import (
	"context"
	"encoding/json"
	"net/http"
	"net/http/httptest"
	"strings"
	"testing"

	"github.com/mrchypark/grant/internal/grant"
)

func managementTestServer(t *testing.T) *Server {
	t.Helper()
	s, _ := managementTestServerWithStore(t)
	return s
}

func managementTestServerWithStore(t *testing.T) (*Server, *grant.Store) {
	t.Helper()
	up := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		json.NewEncoder(w).Encode(map[string]any{
			"token":  "tok",
			"record": map[string]any{"id": "pbuser1", "collectionId": "users", "collectionName": "users", "email": "u@t.co", "verified": true, "name": "U"},
		})
	}))
	t.Cleanup(up.Close)
	cfg := grant.Config{
		Security: grant.SecurityConfig{
			BootstrapUserID:          "owner",
			ManagementOrganizationID: "mgmt-org",
			MasterKey:                "master-secret",
			AppAuthKey:               "app-secret",
		},
		Auth:     grant.AuthConfig{ProviderMode: "pocketbase", PocketBase: &grant.PocketBaseConfig{BaseURL: up.URL, Collection: "users", ApplicationID: "app-1"}},
		Database: grant.DatabaseConfig{DataDir: t.TempDir(), DefaultReadConsistency: grant.ReadStrong},
	}
	store, err := grant.OpenStore(context.Background(), cfg)
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() { _ = store.Close() })
	ctx := context.Background()
	if err = store.EnsureBootstrapData(ctx); err != nil {
		t.Fatal(err)
	}
	svc := grant.NewService(cfg, store)
	now := "2025-01-01T00:00:00Z"

	exec := func(sql string, args ...any) {
		t.Helper()
		if _, e := store.Exec(ctx, sql, args...); e != nil {
			t.Fatal(e)
		}
	}
	// Identity for "tok" → owner
	exec("INSERT INTO user_identities(id,user_id,provider,issuer,collection_id,external_user_id,email_snapshot,display_name_snapshot,last_seen_at,created_at,updated_at) VALUES('id-tok','owner','pocketbase',?,'users','pbuser1',NULL,NULL,?,?,?)", up.URL, now, now, now)
	// Management org + membership
	exec("INSERT INTO organizations(id,name,created_at,updated_at) VALUES('mgmt-org','mgmt-org',?,?)", now, now)
	exec("INSERT INTO organization_memberships(id,organization_id,user_id,membership_role,status,created_at,updated_at) VALUES('mem-mgmt','mgmt-org','owner','owner','active',?,?)", now, now)
	// grant-management provider + action (grantable=1)
	exec("INSERT OR IGNORE INTO service_providers(id,name,active,created_at,updated_at) VALUES('grant-management','Grant Management',1,?,?)", now, now)
	exec("INSERT OR IGNORE INTO actions(id,provider_id,code,name,risk_level,visibility_granting,grantable_to_applications,created_at,updated_at) VALUES('grant-management.admin.provider.create','grant-management','admin.provider.create','Create Provider','medium',0,1,?,?)", now, now)
	// Role + binding (platform/grant resource)
	exec("INSERT OR IGNORE INTO roles(id,organization_id,code,name,created_at,updated_at) VALUES('role-mgmt',NULL,'mgmt','Management',?,?)", now, now)
	exec("INSERT OR IGNORE INTO role_actions(role_id,action_id) VALUES('role-mgmt','grant-management.admin.provider.create')")
	exec("INSERT OR IGNORE INTO role_bindings(id,subject_type,subject_id,subject_organization_id,provider_id,resource_type,resource_id,role_id,state,created_by_user_id,created_at,updated_at) VALUES('bind-mgmt','user','owner','mgmt-org','grant-management','platform','grant','role-mgmt','active','owner',?,?)", now, now)
	// App in mgmt-org + action + consent
	exec("INSERT OR IGNORE INTO applications(id,organization_id,slug,name,application_type,app_type,first_party,consent_policy,active,provisioning_status,created_by_user_id,created_at,updated_at) VALUES('app-1','mgmt-org','app','App','confidential','server',0,'explicit',1,'ready','owner',?,?)", now, now)
	exec("INSERT OR IGNORE INTO application_actions(application_id,action_id) VALUES('app-1','grant-management.admin.provider.create')")
	exec("INSERT OR IGNORE INTO application_grants(id,user_id,application_id,organization_id,status,consented_at,created_at,updated_at) VALUES('g1','owner','app-1','mgmt-org','active',?,?,?)", now, now, now)
	exec("INSERT OR IGNORE INTO application_grant_actions(application_grant_id,action_id) VALUES('g1','grant-management.admin.provider.create')")

	return New(svc, false), store
}

func TestManagementBearerFallbackSuccess(t *testing.T) {
	s := managementTestServer(t)
	body := `{"id":"test-provider","name":"Test Provider","active":true}`
	r := httptest.NewRequest("POST", "/api/v1/admin/providers", strings.NewReader(body))
	r.Header.Set("Content-Type", "application/json")
	r.Header.Set("Authorization", "Bearer tok")
	r.Header.Set("X-Organization-ID", "mgmt-org")
	w := httptest.NewRecorder()
	s.ServeHTTP(w, r)
	if w.Code != 201 {
		t.Fatalf("expected 201, got %d: %s", w.Code, w.Body.String())
	}
}

func TestManagementInvalidMasterCannotDowngrade(t *testing.T) {
	s := managementTestServer(t)
	body := `{"id":"test-provider","name":"Test Provider","active":true}`
	r := httptest.NewRequest("POST", "/api/v1/admin/providers", strings.NewReader(body))
	r.Header.Set("Content-Type", "application/json")
	r.Header.Set("X-Grant-Master-Key", "wrong-key")
	r.Header.Set("Authorization", "Bearer tok")
	r.Header.Set("X-Organization-ID", "mgmt-org")
	w := httptest.NewRecorder()
	s.ServeHTTP(w, r)
	if w.Code != 401 {
		t.Fatalf("expected 401, got %d: %s", w.Code, w.Body.String())
	}
}

func TestManagementBootstrapRouteMasterOnly(t *testing.T) {
	s := managementTestServer(t)
	r := httptest.NewRequest("POST", "/api/v1/admin/bootstrap/owner", nil)
	r.Header.Set("Authorization", "Bearer tok")
	r.Header.Set("X-Organization-ID", "mgmt-org")
	w := httptest.NewRecorder()
	s.ServeHTTP(w, r)
	if w.Code != 401 {
		t.Fatalf("expected 401 for bootstrap without master, got %d: %s", w.Code, w.Body.String())
	}
}
