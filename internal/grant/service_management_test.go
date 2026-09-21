package grant

import (
	"context"
	"encoding/json"
	"net/http"
	"net/http/httptest"
	"testing"
)

// managementFixture builds a Service with a PocketBase mock and the
// grant-management provider/action, role binding, and app consent.
// It creates the managementOrgID org, adds owner as member, and sets
// up all authorization data so AuthorizePlatformManagement succeeds
// for token "tok" with action "admin.provider.create".
func managementFixture(t *testing.T, managementOrgID string) (context.Context, *Service) {
	t.Helper()
	up := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		json.NewEncoder(w).Encode(map[string]any{
			"token":  "tok",
			"record": map[string]any{"id": "pbuser1", "collectionId": "users", "collectionName": "users", "email": "u@t.co", "verified": true, "name": "U"},
		})
	}))
	t.Cleanup(up.Close)
	cfg := Config{
		Security: SecurityConfig{
			BootstrapUserID:          "owner",
			ManagementOrganizationID: managementOrgID,
		},
		Auth:     AuthConfig{ProviderMode: "pocketbase", PocketBase: &PocketBaseConfig{BaseURL: up.URL, Collection: "users", ApplicationID: "app-1"}},
		Database: DatabaseConfig{DataDir: t.TempDir(), DefaultReadConsistency: ReadStrong},
	}
	store, err := OpenStore(context.Background(), cfg)
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() { _ = store.Close() })
	ctx := context.Background()
	if err = store.EnsureBootstrapData(ctx); err != nil {
		t.Fatal(err)
	}
	s := NewService(cfg, store)

	// Register PocketBase identity for "tok" → owner user.
	if _, err = s.store.Exec(ctx, "INSERT INTO user_identities(id,user_id,provider,issuer,collection_id,external_user_id,email_snapshot,display_name_snapshot,last_seen_at,created_at,updated_at) VALUES(?,?,'pocketbase',?,?,?,NULL,NULL,?,?,?)", requestID(), "owner", cfg.Auth.PocketBase.BaseURL, "users", "pbuser1", nowString(), nowString(), nowString()); err != nil {
		t.Fatal(err)
	}

	if managementOrgID != "" && managementOrgID != "owner" {
		if _, err = s.store.Exec(ctx, "INSERT INTO organizations(id,name,created_at,updated_at) VALUES(?,?,?,?)", managementOrgID, managementOrgID, nowString(), nowString()); err != nil {
			t.Fatal(err)
		}
		if _, err = s.store.Exec(ctx, "INSERT INTO organization_memberships(id,organization_id,user_id,membership_role,status,created_at,updated_at) VALUES(?,?,?,'owner','active',?,?)", requestID(), managementOrgID, "owner", nowString(), nowString()); err != nil {
			t.Fatal(err)
		}
	}

	if _, err = s.store.Exec(ctx, "INSERT OR IGNORE INTO service_providers(id,name,active,created_at,updated_at) VALUES('grant-management','Grant Management',1,?,?)", nowString(), nowString()); err != nil {
		t.Fatal(err)
	}
	if _, err = s.store.Exec(ctx, "INSERT OR IGNORE INTO actions(id,provider_id,code,name,risk_level,visibility_granting,grantable_to_applications,created_at,updated_at) VALUES('grant-management.admin.provider.create','grant-management','admin.provider.create','Create Provider','medium',0,1,?,?)", nowString(), nowString()); err != nil {
		t.Fatal(err)
	}
	if _, err = s.store.Exec(ctx, "INSERT OR IGNORE INTO roles(id,organization_id,code,name,created_at,updated_at) VALUES('role-mgmt',NULL,'mgmt','Management',?,?)", nowString(), nowString()); err != nil {
		t.Fatal(err)
	}
	if _, err = s.store.Exec(ctx, "INSERT OR IGNORE INTO role_actions(role_id,action_id) VALUES('role-mgmt','grant-management.admin.provider.create')"); err != nil {
		t.Fatal(err)
	}

	bindOrg := managementOrgID
	if bindOrg == "" {
		bindOrg = "owner"
	}
	if _, err = s.store.Exec(ctx, "INSERT OR IGNORE INTO role_bindings(id,subject_type,subject_id,subject_organization_id,provider_id,resource_type,resource_id,role_id,state,created_by_user_id,created_at,updated_at) VALUES('bind-mgmt','user','owner',?,'grant-management','platform','grant','role-mgmt','active','owner',?,?)", bindOrg, nowString(), nowString()); err != nil {
		t.Fatal(err)
	}

	appOrg := managementOrgID
	if appOrg == "" {
		appOrg = "owner"
	}
	if _, err = s.store.Exec(ctx, "INSERT OR IGNORE INTO applications(id,organization_id,slug,name,application_type,app_type,first_party,consent_policy,active,provisioning_status,created_by_user_id,created_at,updated_at) VALUES('app-1',?,?,'App','confidential','server',0,'explicit',1,'ready','owner',?,?)", appOrg, appOrg, nowString(), nowString()); err != nil {
		t.Fatal(err)
	}
	if _, err = s.store.Exec(ctx, "INSERT OR IGNORE INTO application_actions(application_id,action_id) VALUES('app-1','grant-management.admin.provider.create')"); err != nil {
		t.Fatal(err)
	}
	if _, err = s.store.Exec(ctx, "INSERT OR IGNORE INTO application_grants(id,user_id,application_id,organization_id,status,consented_at,created_at,updated_at) VALUES('g1','owner','app-1',?,'active',?,?,?)", appOrg, nowString(), nowString(), nowString()); err != nil {
		t.Fatal(err)
	}
	if _, err = s.store.Exec(ctx, "INSERT OR IGNORE INTO application_grant_actions(application_grant_id,action_id) VALUES('g1','grant-management.admin.provider.create')"); err != nil {
		t.Fatal(err)
	}
	return ctx, s
}

func TestAuthorizePlatformManagementSuccess(t *testing.T) {
	ctx, s := managementFixture(t, "mgmt-org")
	actor, err := s.AuthorizePlatformManagement(ctx, "tok", "mgmt-org", "admin.provider.create")
	if err != nil {
		t.Fatal(err)
	}
	if actor != "owner" {
		t.Fatal("unexpected actor:", actor)
	}
}

func TestAuthorizePlatformManagementAbsentConfig(t *testing.T) {
	ctx, s := managementFixture(t, "")
	_, err := s.AuthorizePlatformManagement(ctx, "tok", "", "admin.provider.create")
	if err == nil || ErrorStatus(err) != 503 {
		t.Fatal("expected 503, got", err)
	}
}

func TestAuthorizePlatformManagementWrongOrg(t *testing.T) {
	ctx, s := managementFixture(t, "mgmt-org")
	_, err := s.AuthorizePlatformManagement(ctx, "tok", "wrong-org", "admin.provider.create")
	if err == nil || ErrorStatus(err) != 403 {
		t.Fatal("expected 403, got", err)
	}
}

func TestAuthorizePlatformManagementInactiveUser(t *testing.T) {
	ctx, s := managementFixture(t, "mgmt-org")
	// Confirm success before mutation
	if _, err := s.AuthorizePlatformManagement(ctx, "tok", "mgmt-org", "admin.provider.create"); err != nil {
		t.Fatal("precondition:", err)
	}
	if _, err := s.store.Exec(ctx, "UPDATE users SET active=0 WHERE id='owner'"); err != nil {
		t.Fatal(err)
	}
	_, err := s.AuthorizePlatformManagement(ctx, "tok", "mgmt-org", "admin.provider.create")
	if err == nil || ErrorStatus(err) != 403 {
		t.Fatal("expected 403, got", err)
	}
}

func TestAuthorizePlatformManagementNonMember(t *testing.T) {
	ctx, s := managementFixture(t, "mgmt-org")
	// Confirm success before mutation
	if _, err := s.AuthorizePlatformManagement(ctx, "tok", "mgmt-org", "admin.provider.create"); err != nil {
		t.Fatal("precondition:", err)
	}
	// Remove owner's management org membership
	if _, err := s.store.Exec(ctx, "DELETE FROM organization_memberships WHERE user_id='owner' AND organization_id='mgmt-org'"); err != nil {
		t.Fatal(err)
	}
	_, err := s.AuthorizePlatformManagement(ctx, "tok", "mgmt-org", "admin.provider.create")
	if err == nil || ErrorStatus(err) != 403 {
		t.Fatal("expected 403, got", err)
	}
}

func TestAuthorizePlatformManagementRevokedBinding(t *testing.T) {
	ctx, s := managementFixture(t, "mgmt-org")
	// Confirm success before mutation
	if _, err := s.AuthorizePlatformManagement(ctx, "tok", "mgmt-org", "admin.provider.create"); err != nil {
		t.Fatal("precondition:", err)
	}
	if _, err := s.store.Exec(ctx, "UPDATE role_bindings SET state='disabled' WHERE id='bind-mgmt'"); err != nil {
		t.Fatal(err)
	}
	_, err := s.AuthorizePlatformManagement(ctx, "tok", "mgmt-org", "admin.provider.create")
	if err == nil || ErrorStatus(err) != 403 {
		t.Fatal("expected 403, got", err)
	}
}

func TestAuthorizePlatformManagementAppActionMissing(t *testing.T) {
	ctx, s := managementFixture(t, "mgmt-org")
	// Confirm success before mutation
	if _, err := s.AuthorizePlatformManagement(ctx, "tok", "mgmt-org", "admin.provider.create"); err != nil {
		t.Fatal("precondition:", err)
	}
	if _, err := s.store.Exec(ctx, "DELETE FROM application_actions WHERE application_id='app-1'"); err != nil {
		t.Fatal(err)
	}
	_, err := s.AuthorizePlatformManagement(ctx, "tok", "mgmt-org", "admin.provider.create")
	if err == nil || ErrorStatus(err) != 403 {
		t.Fatal("expected 403, got", err)
	}
}

func TestAuthorizePlatformManagementConsentRevoked(t *testing.T) {
	ctx, s := managementFixture(t, "mgmt-org")
	// Confirm success before mutation
	if _, err := s.AuthorizePlatformManagement(ctx, "tok", "mgmt-org", "admin.provider.create"); err != nil {
		t.Fatal("precondition:", err)
	}
	if _, err := s.store.Exec(ctx, "UPDATE application_grants SET status='revoked' WHERE id='g1'"); err != nil {
		t.Fatal(err)
	}
	_, err := s.AuthorizePlatformManagement(ctx, "tok", "mgmt-org", "admin.provider.create")
	if err == nil || ErrorStatus(err) != 403 {
		t.Fatal("expected 403, got", err)
	}
}

func TestAuthorizePlatformManagementConsentMissing(t *testing.T) {
	ctx, s := managementFixture(t, "mgmt-org")
	// Confirm success before mutation
	if _, err := s.AuthorizePlatformManagement(ctx, "tok", "mgmt-org", "admin.provider.create"); err != nil {
		t.Fatal("precondition:", err)
	}
	if _, err := s.store.Exec(ctx, "DELETE FROM application_grant_actions WHERE application_grant_id='g1'"); err != nil {
		t.Fatal(err)
	}
	if _, err := s.store.Exec(ctx, "DELETE FROM application_grants WHERE id='g1'"); err != nil {
		t.Fatal(err)
	}
	_, err := s.AuthorizePlatformManagement(ctx, "tok", "mgmt-org", "admin.provider.create")
	if err == nil || ErrorStatus(err) != 403 {
		t.Fatal("expected 403, got", err)
	}
}

func TestManagementActorFromContextDefault(t *testing.T) {
	if got := ManagementActorFromContext(context.Background()); got != "grant-master" {
		t.Fatal("expected grant-master, got", got)
	}
}

func TestManagementActorFromContextWithValue(t *testing.T) {
	ctx := WithManagementActor(context.Background(), "alice")
	if got := ManagementActorFromContext(ctx); got != "alice" {
		t.Fatal("expected alice, got", got)
	}
}

func TestManagementReactivationCapabilityMatch(t *testing.T) {
	actionID := "grant-management.organization.reactivate"
	cap := HttpCapability{
		Method:       "POST",
		PathTemplate: "/api/v1/orgs/{organizationId}/reactivate",
		ActionID:     &actionID,
		Resource:     &HttpCapabilityResource{ResourceType: "organization", ID: HttpResourceExtractor{Kind: "pathSegment", Name: "organizationId"}},
	}
	if !managementReactivationCapability("grant-management", cap, "org1", "org1", "/api/v1/orgs/org1/reactivate") {
		t.Fatal("expected match")
	}
}

func TestManagementReactivationCapabilityMismatch(t *testing.T) {
	actionID := "grant-management.organization.reactivate"
	cap := HttpCapability{
		Method:       "POST",
		PathTemplate: "/api/v1/orgs/{organizationId}/reactivate",
		ActionID:     &actionID,
		Resource:     &HttpCapabilityResource{ResourceType: "organization", ID: HttpResourceExtractor{Kind: "pathSegment", Name: "organizationId"}},
	}
	if managementReactivationCapability("grant-management", cap, "org1", "org2", "/api/v1/orgs/org1/reactivate") {
		t.Fatal("should not match on wrong resourceID")
	}
	if managementReactivationCapability("other", cap, "org1", "org1", "/api/v1/orgs/org1/reactivate") {
		t.Fatal("should not match on wrong provider")
	}
	cap2 := cap
	cap2.Method = "GET"
	if managementReactivationCapability("grant-management", cap2, "org1", "org1", "/api/v1/orgs/org1/reactivate") {
		t.Fatal("should not match on wrong method")
	}
	cap3 := cap
	cap3.Batch = &HttpBatchPolicy{}
	if managementReactivationCapability("grant-management", cap3, "org1", "org1", "/api/v1/orgs/org1/reactivate") {
		t.Fatal("should not match when batch is set")
	}
}
