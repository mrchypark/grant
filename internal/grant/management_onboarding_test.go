package grant

import (
	"context"
	"strings"
	"testing"
)

// managementMaterializeFixture extends managementFixture with PEP client and
// catalog for the grant-management materialize capability. It returns a
// pre-built HttpCapabilityDecisionInput for the materialize endpoint.
func managementMaterializeFixture(t *testing.T, managementOrgID string) (context.Context, *Service, HttpCapabilityDecisionInput) {
	t.Helper()
	ctx, s := managementFixture(t, managementOrgID)

	// Register the materialize action.
	now := nowString()
	if _, err := s.store.Exec(ctx, "INSERT OR IGNORE INTO actions(id,provider_id,code,name,risk_level,visibility_granting,grantable_to_applications,created_at,updated_at) VALUES('grant-management.user.materialize','grant-management','user.materialize','Materialize','high',0,1,?,?)", now, now); err != nil {
		t.Fatal(err)
	}

	// Generate the management catalog.
	stage, actions, err := ManagementCatalog(1, nil)
	if err != nil {
		t.Fatal(err)
	}
	if _, err = s.ReplaceProviderActionsAdmin(ctx, "grant-management", actions); err != nil {
		t.Fatal(err)
	}

	// Find the materialize capability and add it to the app and grant.
	var materializeCap HttpCapability
	for _, cap := range stage.Capabilities {
		if cap.CapabilityID == "management.user.materialize.1" {
			materializeCap = cap
			break
		}
	}
	if materializeCap.CapabilityID == "" {
		t.Fatal("materialize capability not found in catalog")
	}

	// Add materialize action to application and grant.
	if _, err := s.store.Exec(ctx, "INSERT OR IGNORE INTO application_actions(application_id,action_id) VALUES('app-1','grant-management.user.materialize')"); err != nil {
		t.Fatal(err)
	}
	if _, err := s.store.Exec(ctx, "INSERT OR IGNORE INTO application_grant_actions(application_grant_id,action_id) VALUES('g1','grant-management.user.materialize')"); err != nil {
		t.Fatal(err)
	}

	// Set up the PEP service identity independently of the management app.
	s.cfg.Security.HTTPCapabilityPEPClientID = "mgmt-pep"
	pepApp := s.cfg.Security.ManagementApplicationID
	if pepApp == "" {
		pepApp = "app-1"
		s.cfg.Security.ManagementApplicationID = pepApp
	}
	if _, err := s.store.UpsertProviderPepClient(ctx, "grant-management", "mgmt-pep", "owner", UpsertProviderPepClient{Lifecycle: "active"}); err != nil {
		t.Fatal(err)
	}

	// Stage and activate the catalog.
	if _, err := s.store.StageHttpCapabilityGeneration(ctx, "grant-management", stage); err != nil {
		t.Fatal(err)
	}
	if _, err := s.store.ActivateHttpCapabilityGeneration(ctx, "grant-management", "owner", ActivateHttpCapabilityGeneration{TargetGeneration: 1, TargetCatalogDigest: stage.CatalogDigest}); err != nil {
		t.Fatal(err)
	}

	// Build the decision input for materialize.
	emptyBodyDigest := sha256Text("")
	input := HttpCapabilityDecisionInput{
		ProviderID:               "grant-management",
		PEPClientID:              "mgmt-pep",
		CatalogGeneration:        stage.Generation,
		CatalogDigest:            stage.CatalogDigest,
		CapabilityID:             materializeCap.CapabilityID,
		CapabilityVersion:        materializeCap.Version,
		ContractDigest:           materializeCap.ContractDigest,
		Method:                   "POST",
		CanonicalPath:            "/api/v1/me/ensure",
		CanonicalQuery:           "",
		CanonicalQueryDigest:     sha256Text(""),
		ForwardedHeaderDigest:    sha256Text(""),
		BodyDigest:               emptyBodyDigest,
		BodyLength:               0,
		ProviderCredentialDigest: CredentialDigest("tok"),
		ProviderCredential:       HttpCapabilityCredentialEvidence{Subject: pocketBaseSubject(s.cfg.Auth.PocketBase.BaseURL, "users", "pbuser1"), Audience: "app-1"},
		ResourceEvidence:         HttpCapabilityResourceEvidence{Kind: "constant", CanonicalValue: "grant"},
		ResourceType:             "service",
		ResourceID:               "grant",
		RequestBinding:           strings.Repeat("a", 22),
	}

	decision, err := s.DecideHttpCapability(ctx, "tok", managementOrgID, input)
	if err != nil || !decision.Allowed {
		t.Fatalf("onboarding baseline: allowed=%v err=%v", decision.Allowed, err)
	}
	return ctx, s, input
}

func TestMaterializeAllowNoMembership(t *testing.T) {
	ctx, s, input := managementMaterializeFixture(t, "mgmt-org")
	// Remove membership — materialize must still allow.
	if _, err := s.store.Exec(ctx, "DELETE FROM organization_memberships WHERE user_id='owner' AND organization_id='mgmt-org'"); err != nil {
		t.Fatal(err)
	}
	decision, err := s.DecideHttpCapability(ctx, "tok", "mgmt-org", input)
	if err != nil {
		t.Fatal(err)
	}
	if !decision.Allowed || decision.Reason != "management_materialize" {
		t.Fatalf("expected allow, got allowed=%v reason=%q", decision.Allowed, decision.Reason)
	}
}

func TestMaterializeNewIdentityDoesNotGrantManagementAccess(t *testing.T) {
	ctx, s, input := managementMaterializeFixture(t, "mgmt-org")
	if _, err := s.store.Exec(ctx, "UPDATE applications SET first_party=1,consent_policy='auto_approve' WHERE id='app-1'"); err != nil {
		t.Fatal(err)
	}
	if _, err := s.store.Exec(ctx, "DELETE FROM user_identities WHERE user_id='owner'"); err != nil {
		t.Fatal(err)
	}
	decision, err := s.DecideHttpCapability(ctx, "tok", "", input)
	if err != nil || !decision.Allowed {
		t.Fatalf("new identity materialization: allowed=%v err=%v", decision.Allowed, err)
	}
	view, err := s.EnsureCurrentUser(ctx, "tok")
	if err != nil || view.UserID == "" || view.UserID == "owner" {
		t.Fatalf("ensured view=%+v err=%v", view, err)
	}
	stage, _, err := ManagementCatalog(1, nil)
	if err != nil {
		t.Fatal(err)
	}
	for _, cap := range stage.Capabilities {
		if cap.Method == "GET" && cap.PathTemplate == "/api/v1/me" {
			input.CapabilityID, input.ContractDigest = cap.CapabilityID, cap.ContractDigest
			input.Method, input.CanonicalPath = cap.Method, cap.PathTemplate
			decision, err = s.DecideHttpCapability(ctx, "tok", "mgmt-org", input)
			if err != nil || decision.Allowed {
				t.Fatalf("ordinary management must remain denied: allowed=%v err=%v", decision.Allowed, err)
			}
			return
		}
	}
	t.Fatal("ordinary management capability missing")
}

func TestMaterializeAllowNoConsentNoRoles(t *testing.T) {
	ctx, s, input := managementMaterializeFixture(t, "mgmt-org")
	// Remove role binding, consent grant, and membership — materialize must still allow.
	if _, err := s.store.Exec(ctx, "DELETE FROM role_bindings WHERE id='bind-mgmt'"); err != nil {
		t.Fatal(err)
	}
	if _, err := s.store.Exec(ctx, "DELETE FROM application_grant_actions WHERE application_grant_id='g1'"); err != nil {
		t.Fatal(err)
	}
	if _, err := s.store.Exec(ctx, "DELETE FROM application_grants WHERE id='g1'"); err != nil {
		t.Fatal(err)
	}
	if _, err := s.store.Exec(ctx, "DELETE FROM organization_memberships WHERE user_id='owner' AND organization_id='mgmt-org'"); err != nil {
		t.Fatal(err)
	}
	decision, err := s.DecideHttpCapability(ctx, "tok", "mgmt-org", input)
	if err != nil {
		t.Fatal(err)
	}
	if !decision.Allowed || decision.Reason != "management_materialize" {
		t.Fatalf("expected allow, got allowed=%v reason=%q", decision.Allowed, decision.Reason)
	}
}

func TestMaterializeDenyRemovedAppAction(t *testing.T) {
	_, s, input := managementMaterializeFixture(t, "mgmt-org")
	// Remove the materialize action from application_actions.
	if _, err := s.store.Exec(context.Background(), "DELETE FROM application_actions WHERE application_id='app-1' AND action_id='grant-management.user.materialize'"); err != nil {
		t.Fatal(err)
	}
	decision, err := s.DecideHttpCapability(context.Background(), "tok", "mgmt-org", input)
	if err != nil {
		t.Fatal(err)
	}
	if decision.Allowed {
		t.Fatal("expected deny when app action removed")
	}
}

func TestMaterializeDenyInactiveOrg(t *testing.T) {
	_, s, input := managementMaterializeFixture(t, "mgmt-org")
	// Deactivate the organization.
	if _, err := s.store.Exec(context.Background(), "UPDATE organizations SET inactivated_at='2025-01-01T00:00:00Z' WHERE id='mgmt-org'"); err != nil {
		t.Fatal(err)
	}
	decision, err := s.DecideHttpCapability(context.Background(), "tok", "mgmt-org", input)
	if err != nil {
		t.Fatal(err)
	}
	if decision.Allowed {
		t.Fatal("expected deny for inactive org")
	}
}

func TestMaterializeDenyInactiveUser(t *testing.T) {
	_, s, input := managementMaterializeFixture(t, "mgmt-org")
	// Deactivate the user.
	if _, err := s.store.Exec(context.Background(), "UPDATE users SET active=0 WHERE id='owner'"); err != nil {
		t.Fatal(err)
	}
	decision, err := s.DecideHttpCapability(context.Background(), "tok", "mgmt-org", input)
	if err == nil || ErrorStatus(err) != 403 || decision.Allowed {
		t.Fatal("expected deny for inactive user")
	}
}

func TestMaterializeDenyInactiveProvider(t *testing.T) {
	_, s, input := managementMaterializeFixture(t, "mgmt-org")
	// Deactivate the provider.
	if _, err := s.store.Exec(context.Background(), "UPDATE service_providers SET active=0 WHERE id='grant-management'"); err != nil {
		t.Fatal(err)
	}
	decision, err := s.DecideHttpCapability(context.Background(), "tok", "mgmt-org", input)
	if (err != nil && ErrorStatus(err) != 403) || decision.Allowed {
		t.Fatal("expected deny for inactive provider")
	}
}

func TestMaterializeDenyInactiveApp(t *testing.T) {
	_, s, input := managementMaterializeFixture(t, "mgmt-org")
	// Deactivate the application.
	if _, err := s.store.Exec(context.Background(), "UPDATE applications SET active=0 WHERE id='app-1'"); err != nil {
		t.Fatal(err)
	}
	decision, err := s.DecideHttpCapability(context.Background(), "tok", "mgmt-org", input)
	if err == nil || ErrorStatus(err) != 403 || decision.Allowed {
		t.Fatal("expected deny for inactive application")
	}
}

func TestManagementEnsureCapabilityMatch(t *testing.T) {
	actionID := "grant-management.user.materialize"
	cap := HttpCapability{
		Method:       "POST",
		PathTemplate: "/api/v1/me/ensure",
		ActionID:     &actionID,
		Resource:     &HttpCapabilityResource{ResourceType: "service", ID: HttpResourceExtractor{Kind: "constant", Value: "grant"}},
		Request:      HttpRequestPolicy{Body: "none", Authorization: "forward-user"},
	}
	if !managementEnsureCapability("grant-management", cap) {
		t.Fatal("expected match")
	}
}

func TestManagementEnsureCapabilityMismatch(t *testing.T) {
	actionID := "grant-management.user.materialize"
	cap := HttpCapability{
		Method:       "POST",
		PathTemplate: "/api/v1/me/ensure",
		ActionID:     &actionID,
		Resource:     &HttpCapabilityResource{ResourceType: "service", ID: HttpResourceExtractor{Kind: "constant", Value: "grant"}},
		Request:      HttpRequestPolicy{Body: "none", Authorization: "forward-user"},
	}
	// Wrong provider.
	if managementEnsureCapability("other", cap) {
		t.Fatal("should not match on wrong provider")
	}
	// Wrong method.
	cap2 := cap
	cap2.Method = "GET"
	if managementEnsureCapability("grant-management", cap2) {
		t.Fatal("should not match on wrong method")
	}
	// Wrong path.
	cap3 := cap
	cap3.PathTemplate = "/api/v1/me"
	if managementEnsureCapability("grant-management", cap3) {
		t.Fatal("should not match on wrong path")
	}
	// Wrong action.
	cap4 := cap
	cap4.ActionID = ptrString("grant-management.user.read")
	if managementEnsureCapability("grant-management", cap4) {
		t.Fatal("should not match on wrong action")
	}
	// Wrong resource type.
	cap5 := cap
	cap5.Resource = &HttpCapabilityResource{ResourceType: "organization", ID: HttpResourceExtractor{Kind: "constant", Value: "grant"}}
	if managementEnsureCapability("grant-management", cap5) {
		t.Fatal("should not match on wrong resource type")
	}
	// Has recordScope.
	cap6 := cap
	cap6.RecordScope = &HttpRecordScopePolicy{}
	if managementEnsureCapability("grant-management", cap6) {
		t.Fatal("should not match when recordScope is set")
	}
	// Has batch.
	cap7 := cap
	cap7.Batch = &HttpBatchPolicy{}
	if managementEnsureCapability("grant-management", cap7) {
		t.Fatal("should not match when batch is set")
	}
	// Has responseAuthorization.
	cap8 := cap
	cap8.ResponseAuthorization = &HttpResponseAuthorization{}
	if managementEnsureCapability("grant-management", cap8) {
		t.Fatal("should not match when responseAuthorization is set")
	}
	cap9 := cap
	cap9.Request.Body = "json"
	if managementEnsureCapability("grant-management", cap9) {
		t.Fatal("should not match a body-bearing capability")
	}
	cap10 := cap
	cap10.Request.Authorization = "service-token"
	if managementEnsureCapability("grant-management", cap10) {
		t.Fatal("should not match a service credential capability")
	}
}

func ptrString(s string) *string { return &s }
