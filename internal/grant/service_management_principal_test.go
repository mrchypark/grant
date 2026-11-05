package grant

import (
	"context"
	"strings"
	"testing"
)

func managementPrincipalFixture(t *testing.T) (context.Context, *Service, HttpCapabilityDecisionInput, *int) {
	t.Helper()
	ctx, s, identity, calls := principalLocalFixture(t)
	s.cfg.Security.ManagementOrganizationID = "owner"
	s.cfg.Security.HTTPCapabilityPEPClientID = "management-pep"
	if _, err := s.UpsertServiceProviderAdmin(ctx, ServiceProvider{ID: "grant-management", Name: "Grant Management", Active: true}); err != nil {
		t.Fatal(err)
	}
	stage, actions, err := ManagementCatalog(1, nil)
	if err != nil {
		t.Fatal(err)
	}
	if _, err = s.ReplaceProviderActionsAdmin(ctx, "grant-management", actions); err != nil {
		t.Fatal(err)
	}
	mode := "user"
	if _, err = s.SetProviderPepClientAdmin(ctx, "grant-management", "management-pep", UpsertProviderPepClient{Lifecycle: "active", AuthorizationMode: &mode}); err != nil {
		t.Fatal(err)
	}
	if _, err = s.StageHttpCapabilityGenerationAdmin(ctx, "grant-management", stage); err != nil {
		t.Fatal(err)
	}
	if _, err = s.ActivateHttpCapabilityGenerationAdmin(ctx, "grant-management", ActivateHttpCapabilityGeneration{TargetGeneration: 1, TargetCatalogDigest: stage.CatalogDigest}); err != nil {
		t.Fatal(err)
	}
	lifecycleExec(t, ctx, s, "INSERT INTO role_actions(role_id,action_id) VALUES('role-system-bootstrap-owner','grant-management.admin.provider.create')")
	lifecycleExec(t, ctx, s, "INSERT INTO role_bindings(id,subject_type,subject_id,subject_organization_id,provider_id,resource_type,resource_id,role_id,state,created_by_user_id,created_at,updated_at) VALUES('management-user','user','owner','owner','grant-management','platform','grant','role-system-bootstrap-owner','active','owner','now','now')")
	if lifecycleCount(t, ctx, s, "SELECT id FROM applications") != 0 {
		t.Fatal("fixture must have zero applications")
	}
	for _, cap := range stage.Capabilities {
		if cap.Method != "POST" || cap.PathTemplate != "/api/v1/admin/providers" {
			continue
		}
		input := batchChildInput(batchEvidenceInput(t, stage, cap, nil), batchEvidenceChild(cap, 0, "grant"))
		input.ProviderID, input.PEPClientID = "grant-management", "management-pep"
		input.WireVersion, input.PEPPolicyRevision = 2, 1
		input.ProviderCredentialDigest = CredentialDigest("Bearer opaque-token")
		input.ProviderCredential = HttpCapabilityCredentialEvidence{Subject: goauthySubject(identity.Issuer, identity.ExternalUserID), Audience: identity.TokenAudience}
		return ctx, s, input, calls
	}
	t.Fatal("canonical platform capability missing")
	return ctx, s, HttpCapabilityDecisionInput{}, calls
}

func TestManagementUserPEPWithoutApplications(t *testing.T) {
	ctx, s, input, _ := managementPrincipalFixture(t)
	actor, err := s.AuthorizePlatformManagement(ctx, "Bearer opaque-token", "owner", "admin.provider.create")
	if err != nil || actor != "owner" {
		t.Fatalf("native platform actor=%q err=%v", actor, err)
	}
	decision, err := s.DecideHttpCapability(ctx, "Bearer opaque-token", "owner", input)
	if err != nil || !decision.Allowed {
		t.Fatalf("platform PDP decision=%+v err=%v", decision, err)
	}
	if _, err = s.AuthorizePlatformManagement(ctx, "Bearer opaque-token", "owner", "admin.provider.update"); err == nil || ErrorStatus(err) != 403 {
		t.Fatalf("ungranted platform action accepted: %v", err)
	}
	if lifecycleCount(t, ctx, s, "SELECT id FROM applications") != 0 {
		t.Fatal("user authorization created an application")
	}
}

func TestManagementUserPEPRequiresConfiguredOrganization(t *testing.T) {
	for _, configuredOrg := range []string{"", "operator-org"} {
		t.Run(configuredOrg, func(t *testing.T) {
			ctx, s, input, _ := managementPrincipalFixture(t)
			// The tenant owner really holds the exact platform role; it cannot
			// override the operator's configured organization boundary.
			s.cfg.Security.ManagementOrganizationID = configuredOrg
			actor, err := s.AuthorizePlatformManagement(ctx, "Bearer opaque-token", "owner", "admin.provider.create")
			status := 403
			if configuredOrg == "" {
				status = 503
			}
			if err == nil || ErrorStatus(err) != status || actor != "" {
				t.Fatalf("native platform boundary actor=%q err=%v", actor, err)
			}
			decision, err := s.DecideHttpCapability(ctx, "Bearer opaque-token", "owner", input)
			if err != nil || decision.Allowed || decision.Reason != "platform_management_organization_required" {
				t.Fatalf("platform PDP boundary decision=%+v err=%v", decision, err)
			}
		})
	}
}

func TestManagementUserPEPFailClosed(t *testing.T) {
	for _, tc := range []struct{ name, sql string }{
		{"application", "UPDATE provider_pep_clients SET authorization_mode='application',policy_revision=policy_revision+1"},
		{"disabled", "UPDATE provider_pep_clients SET lifecycle='disabled'"},
		{"deleted user policy", "UPDATE provider_pep_clients SET lifecycle='deleted'"},
		{"expired", "UPDATE provider_pep_clients SET valid_until='2000-01-01T00:00:00.000000000Z'"},
		{"inactive provider", "UPDATE service_providers SET active=0 WHERE id='grant-management'"},
		{"inactive user", "UPDATE users SET active=0 WHERE id='owner'"},
		{"inactive organization", "UPDATE organizations SET inactivated_at='now' WHERE id='owner'"},
		{"disabled membership", "UPDATE organization_memberships SET status='disabled' WHERE user_id='owner'"},
		{"revoked role", "UPDATE role_bindings SET state='revoked' WHERE id='management-user'"},
	} {
		t.Run(tc.name, func(t *testing.T) {
			ctx, s, input, _ := managementPrincipalFixture(t)
			lifecycleExec(t, ctx, s, tc.sql)
			actor, err := s.AuthorizePlatformManagement(ctx, "Bearer opaque-token", "owner", "admin.provider.create")
			if err == nil || ErrorStatus(err) != 403 || actor != "" {
				t.Fatalf("native platform authorization did not fail closed: actor=%q err=%v", actor, err)
			}
			decision, err := s.DecideHttpCapability(ctx, "Bearer opaque-token", "owner", input)
			if decision.Allowed || err != nil && ErrorStatus(err) != 403 {
				t.Fatalf("platform PDP did not fail closed: decision=%+v err=%v", decision, err)
			}
		})
	}
}

func TestManagementForeignPEPDoesNotSelectUserMode(t *testing.T) {
	ctx, s, _, _ := managementPrincipalFixture(t)
	mode := "user"
	if _, err := s.SetProviderPepClientAdmin(ctx, "grant", "foreign-pep", UpsertProviderPepClient{Lifecycle: "active", AuthorizationMode: &mode}); err != nil {
		t.Fatal(err)
	}
	s.cfg.Security.HTTPCapabilityPEPClientID = "foreign-pep"
	actor, err := s.AuthorizePlatformManagement(ctx, "Bearer opaque-token", "owner", "admin.provider.create")
	if err == nil || ErrorStatus(err) != 403 || actor != "" {
		t.Fatalf("foreign provider policy selected user authorization: actor=%q err=%v", actor, err)
	}
	s.cfg.Security.HTTPCapabilityPEPClientID = ""
	actor, err = s.AuthorizePlatformManagement(ctx, "Bearer opaque-token", "owner", "admin.provider.create")
	if err == nil || ErrorStatus(err) != 403 || actor != "" {
		t.Fatalf("unconfigured PEP bypassed legacy application admission: actor=%q err=%v", actor, err)
	}
}

func TestManagementApplicationPEPPreservesAdmission(t *testing.T) {
	ctx, s := managementFixture(t, "mgmt-org")
	s.cfg.Security.HTTPCapabilityPEPClientID = "management-pep"
	mode := "application"
	if _, err := s.SetProviderPepClientAdmin(ctx, "grant-management", "management-pep", UpsertProviderPepClient{Lifecycle: "active", AuthorizationMode: &mode}); err != nil {
		t.Fatal(err)
	}
	if actor, err := s.AuthorizePlatformManagement(ctx, "tok", "mgmt-org", "admin.provider.create"); err != nil || actor != "owner" {
		t.Fatalf("legacy application authorization changed: actor=%q err=%v", actor, err)
	}
	lifecycleExec(t, ctx, s, "DELETE FROM application_actions WHERE application_id='app-1'")
	if actor, err := s.AuthorizePlatformManagement(ctx, "tok", "mgmt-org", "admin.provider.create"); err == nil || ErrorStatus(err) != 403 || actor != "" {
		t.Fatalf("application admission bypassed: actor=%q err=%v", actor, err)
	}
}

func TestManagementApplicationPEPCompatibility(t *testing.T) {
	for _, tc := range []struct{ name, provider, lifecycle string }{
		{"foreign user policy", "grant", "active"},
		{"inactive canonical application policy", "grant-management", "disabled"},
		{"missing canonical policy", "", ""},
	} {
		t.Run(tc.name, func(t *testing.T) {
			ctx, s := managementFixture(t, "mgmt-org")
			s.cfg.Security.HTTPCapabilityPEPClientID = "management-pep"
			if tc.provider != "" {
				mode := "application"
				if tc.provider != "grant-management" {
					mode = "user"
				}
				if _, err := s.SetProviderPepClientAdmin(ctx, tc.provider, "management-pep", UpsertProviderPepClient{Lifecycle: tc.lifecycle, AuthorizationMode: &mode}); err != nil {
					t.Fatal(err)
				}
			}
			if actor, err := s.AuthorizePlatformManagement(ctx, "tok", "mgmt-org", "admin.provider.create"); err != nil || actor != "owner" {
				t.Fatalf("legacy application authorization changed: actor=%q err=%v", actor, err)
			}
			lifecycleExec(t, ctx, s, "DELETE FROM application_actions WHERE application_id='app-1'")
			if actor, err := s.AuthorizePlatformManagement(ctx, "tok", "mgmt-org", "admin.provider.create"); err == nil || ErrorStatus(err) != 403 || actor != "" {
				t.Fatalf("legacy application admission bypassed: actor=%q err=%v", actor, err)
			}
		})
	}
}

func TestManagementUserPEPModeChangeIsFenced(t *testing.T) {
	for _, native := range []bool{true, false} {
		name := "PDP"
		if native {
			name = "native"
		}
		t.Run(name, func(t *testing.T) {
			ctx, s, input, calls := managementPrincipalFixture(t)
			before := *calls
			policyReads := 0
			s.store.afterReplicaQuery = func(_ context.Context, sql string) {
				if strings.Contains(sql, "provider_pep_clients") {
					policyReads++
					if policyReads == 1 {
						lifecycleExec(t, context.Background(), s, "UPDATE provider_pep_clients SET authorization_mode='application',policy_revision=policy_revision+1 WHERE pep_client_id='management-pep'")
					}
				}
			}
			defer func() { s.store.afterReplicaQuery = nil }()
			var err error
			if native {
				var actor string
				actor, err = s.AuthorizePlatformManagement(ctx, "Bearer opaque-token", "owner", "admin.provider.create")
				if actor != "" {
					t.Fatal("stale native actor returned")
				}
			} else {
				var decision HttpCapabilityDecision
				decision, err = s.DecideHttpCapability(ctx, "Bearer opaque-token", "owner", input)
				if decision.Allowed {
					t.Fatal("stale PDP allow returned")
				}
			}
			if err == nil || ErrorStatus(err) != 403 || policyReads < 2 || *calls-before != 1 {
				t.Fatalf("mode change escaped snapshot retry: err=%v policyReads=%d validations=%d", err, policyReads, *calls-before)
			}
		})
	}
}

func TestManagementUserPEPUnstableSnapshotDenied(t *testing.T) {
	ctx, s, _, calls := managementPrincipalFixture(t)
	before := *calls
	queries := 0
	s.store.afterReplicaQuery = func(_ context.Context, _ string) {
		queries++
		lifecycleExec(t, context.Background(), s, "UPDATE users SET display_name=? WHERE id='owner'", requestID())
	}
	defer func() { s.store.afterReplicaQuery = nil }()
	actor, err := s.AuthorizePlatformManagement(ctx, "Bearer opaque-token", "owner", "admin.provider.create")
	if err == nil || ErrorStatus(err) != 503 || actor != "" || queries != 3 || *calls-before != 1 {
		t.Fatalf("unstable snapshot authorized: actor=%q err=%v queries=%d validations=%d", actor, err, queries, *calls-before)
	}
}
