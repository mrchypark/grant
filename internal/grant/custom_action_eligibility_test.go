package grant

import (
	"context"
	"testing"
	"time"
)

func eligibilityFixture(t *testing.T) (context.Context, *Service, Action) {
	t.Helper()
	ctx, s := serviceFixture(t)
	if _, err := s.UpsertServiceProviderAdmin(ctx, ServiceProvider{ID: "operable", Name: "Operable", Active: true}); err != nil {
		t.Fatal(err)
	}
	a, err := s.CreateOperableCustomActionAuthorized(ctx, "owner", "owner", CustomActionInput{Code: operableCustomNamespace("owner") + "calendar.event.create", Name: "Create", RiskLevel: "low"})
	if err != nil {
		t.Fatal(err)
	}
	return ctx, s, a
}

func TestCustomActionExplicitEligibility(t *testing.T) {
	ctx, s, a := eligibilityFixture(t)
	if a.GrantableToApplications {
		t.Fatal("create implicitly eligible")
	}
	yes, no := true, false
	patch := CustomActionPatch{GrantableToApplications: &yes}
	snap, err := s.ProviderActionsSnapshotAuthorized(ctx, "owner", "owner", "operable")
	if err != nil {
		t.Fatal(err)
	}
	if _, err = s.PatchOperableCustomActionConditional(ctx, "owner", "owner", a.ID, patch, ""); ErrorStatus(err) != 428 {
		t.Fatalf("missing CAS: %v", err)
	}
	if _, err = s.PatchOperableCustomActionConditional(ctx, "owner", "owner", a.ID, patch, `"stale"`); ErrorStatus(err) != 412 {
		t.Fatalf("stale CAS: %v", err)
	}
	a, err = s.PatchOperableCustomActionConditional(ctx, "owner", "owner", a.ID, patch, snap.ETag)
	if err != nil || !a.GrantableToApplications {
		t.Fatalf("enable: %+v %v", a, err)
	}
	for _, table := range []string{"application_actions", "application_grant_actions", "role_actions"} {
		if lifecycleCount(t, ctx, s, "SELECT * FROM "+table+" WHERE action_id=?", a.ID) != 0 {
			t.Fatal("implicit grant", table)
		}
	}
	if lifecycleCount(t, ctx, s, "SELECT * FROM application_grants") != 0 {
		t.Fatal("implicit consent")
	}
	a, err = s.PatchOperableCustomActionAuthorized(ctx, "owner", "owner", a.ID, CustomActionPatch{Name: stringPtr("Metadata")})
	if err != nil || !a.GrantableToApplications {
		t.Fatalf("omission: %+v %v", a, err)
	}
	if err = s.store.InsertApplicationWithRedirectURI(ctx, Application{ID: "eligible-app", OrganizationID: "owner", Slug: "eligible-app", Name: "App", ApplicationType: "confidential", AppType: "internal", FirstParty: false, ConsentPolicy: "explicit", Active: true, ProvisioningStatus: "ready", CreatedByUserID: "owner"}, nil); err != nil {
		t.Fatal(err)
	}
	if err = s.SetApplicationAction(ctx, "owner", "owner", "eligible-app", a.ID, true); err != nil {
		t.Fatal(err)
	}
	snap, err = s.ProviderActionsSnapshotAuthorized(ctx, "owner", "owner", "operable")
	if err != nil {
		t.Fatal(err)
	}
	if _, err = s.PatchOperableCustomActionConditional(ctx, "owner", "owner", a.ID, CustomActionPatch{GrantableToApplications: &no}, snap.ETag); ErrorStatus(err) != 409 {
		t.Fatalf("disabled admitted action: %v", err)
	}
	if lifecycleCount(t, ctx, s, "SELECT * FROM application_actions WHERE action_id=?", a.ID) != 1 {
		t.Fatal("cascaded admission")
	}
	if err = s.SetApplicationAction(ctx, "owner", "owner", "eligible-app", a.ID, false); err != nil {
		t.Fatal(err)
	}
	if _, err = s.PatchOperableCustomActionConditional(ctx, "owner", "owner", a.ID, CustomActionPatch{GrantableToApplications: &no}, snap.ETag); err != nil {
		t.Fatal(err)
	}
}

func TestCustomActionEligibilityTransactionPreconditions(t *testing.T) {
	for _, tc := range []struct {
		name, sql string
		status    int
	}{
		{name: "catalog", sql: "UPDATE actions SET name='Changed' WHERE id=?", status: 412},
		{name: "demotion", sql: "UPDATE organization_memberships SET membership_role='member' WHERE user_id='owner'", status: 409},
		{name: "permission", sql: "DELETE FROM role_actions WHERE action_id='organization.permission.manage'", status: 409},
		{name: "provider", sql: "UPDATE service_providers SET active=0 WHERE id='operable'", status: 409},
	} {
		t.Run(tc.name, func(t *testing.T) {
			ctx, s, a := eligibilityFixture(t)
			snapshot, err := s.ProviderActionsSnapshot(ctx, "operable")
			if err != nil {
				t.Fatal(err)
			}
			checked, err := checkedActionContext(ctx, snapshot, snapshot.ETag, "")
			if err != nil {
				t.Fatal(err)
			}
			guards, err := s.ownerActionMutationGuard(ctx, "owner", "owner", "operable")
			if err != nil {
				t.Fatal(err)
			}
			// Pause after authorization and snapshot preparation, then commit revocation first.
			if tc.name == "catalog" {
				lifecycleExec(t, ctx, s, tc.sql, a.ID)
			} else {
				lifecycleExec(t, ctx, s, tc.sql)
			}
			guards = append(guards, Statement{SQL: "UPDATE actions SET grantable_to_applications=1 WHERE id=?", Args: []any{a.ID}, ExpectedRowsAffected: requireOne()})
			err = lifecycleError(s.store.actionTransaction(checked, guards))
			if ErrorStatus(err) != tc.status {
				t.Fatalf("accepted changed preconditions: %v", err)
			}
			current, err := serviceRow[Action](ctx, s, ReadStrong, "SELECT * FROM actions WHERE id=?", a.ID)
			if err != nil || current.GrantableToApplications {
				t.Fatalf("partial write: %+v %v", current, err)
			}
		})
	}
}

func TestCustomActionAdmissionOrganizationGuard(t *testing.T) {
	ctx, s, a := eligibilityFixture(t)
	if err := s.store.InsertApplicationWithRedirectURI(ctx, Application{ID: "scope-app", OrganizationID: "owner", Slug: "scope-app", Name: "Scope", ApplicationType: "confidential", AppType: "internal", FirstParty: false, ConsentPolicy: "explicit", Active: true, ProvisioningStatus: "ready", CreatedByUserID: "owner"}, nil); err != nil {
		t.Fatal(err)
	}
	// Simulate a globally eligible foreign custom action, including legacy/admin data.
	lifecycleExec(t, ctx, s, "UPDATE actions SET code=?,grantable_to_applications=1 WHERE id=?", operableCustomNamespace("foreign")+"calendar.event.create", a.ID)
	if err := s.SetApplicationAction(ctx, "owner", "owner", "scope-app", a.ID, true); ErrorStatus(err) != 409 {
		t.Fatalf("single foreign admission: %v", err)
	}
	if _, err := s.ReplaceApplicationActionsAuthorized(ctx, "owner", "owner", "scope-app", []string{a.ID}); ErrorStatus(err) != 400 {
		t.Fatalf("replacement foreign admission: %v", err)
	}
	target, err := serviceRow[applicationActionTarget](ctx, s, ReadStrong, "SELECT id,organization_id,created_at FROM applications WHERE id='scope-app'")
	if err != nil {
		t.Fatal(err)
	}
	if _, err = s.store.replaceApplicationActions(ctx, target, "owner", []string{a.ID}); err == nil {
		t.Fatal("transaction admitted foreign custom action")
	}
	if lifecycleCount(t, ctx, s, "SELECT * FROM application_actions WHERE application_id='scope-app'") != 0 {
		t.Fatal("partial foreign admission")
	}
	yes := true
	snap, err := s.ProviderActionsSnapshotAuthorized(ctx, "owner", "owner", "operable")
	if err != nil {
		t.Fatal(err)
	}
	if _, err = s.PatchOperableCustomActionConditional(ctx, "owner", "owner", a.ID, CustomActionPatch{GrantableToApplications: &yes}, snap.ETag); ErrorStatus(err) != 409 {
		t.Fatalf("mutated foreign action: %v", err)
	}
	if _, err = s.PatchOperableCustomActionConditional(ctx, "owner", "owner", "organization.profile.read", CustomActionPatch{GrantableToApplications: &yes}, snap.ETag); ErrorStatus(err) != 409 {
		t.Fatalf("mutated system action: %v", err)
	}
}

func TestCustomActionEligibilityRequiresOwnerAndPermission(t *testing.T) {
	ctx, s, a := eligibilityFixture(t)
	ownershipMember(t, ctx, s, "delegate")
	if _, err := s.CreateRoleAuthorized(ctx, "owner", "owner", CreateRoleInput{ID: "permission-manager", Code: "permission-manager", Name: "Manager"}); err != nil {
		t.Fatal(err)
	}
	if _, err := s.ReplaceRoleActionsAuthorized(ctx, "owner", "owner", "permission-manager", []string{"organization.permission.manage", "organization.application.manage"}); err != nil {
		t.Fatal(err)
	}
	if _, err := s.CreateRoleBindingAuthorized(ctx, "owner", "owner", CreateRoleBindingInput{ID: "delegate-permissions", SubjectType: "user", SubjectID: "delegate", SubjectOrganizationID: "owner", ProviderID: "grant", ResourceType: "organization", ResourceID: "owner", RoleID: "permission-manager", State: "active"}); err != nil {
		t.Fatal(err)
	}
	yes := true
	snap, err := s.ProviderActionsSnapshotAuthorized(ctx, "owner", "owner", "operable")
	if err != nil {
		t.Fatal(err)
	}
	// Even the correct catalog validator cannot substitute for actual ownership.
	if _, err = s.PatchOperableCustomActionConditional(ctx, "delegate", "owner", a.ID, CustomActionPatch{GrantableToApplications: &yes}, snap.ETag); ErrorStatus(err) != 403 {
		t.Fatalf("permission-only actor changed eligibility: %v", err)
	}
	// Direct service entry also enforces actual owner authority before CAS.
	if _, err = s.PatchOperableCustomActionAuthorized(ctx, "delegate", "owner", a.ID, CustomActionPatch{GrantableToApplications: &yes}); ErrorStatus(err) != 403 {
		t.Fatalf("non-owner accepted: %v", err)
	}
	lifecycleExec(t, ctx, s, "DELETE FROM role_actions WHERE action_id='organization.permission.manage'")
	if _, err = s.PatchOperableCustomActionConditional(ctx, "owner", "owner", a.ID, CustomActionPatch{GrantableToApplications: &yes}, snap.ETag); ErrorStatus(err) != 403 {
		t.Fatalf("owner without permission accepted: %v", err)
	}
}

func TestCustomActionEligibilityServicePathRechecksAuthority(t *testing.T) {
	for _, provider := range []string{"operable", "independent-authz"} {
		t.Run(provider, func(t *testing.T) {
			for _, revocation := range []string{"demotion", "permission"} {
				t.Run(revocation, func(t *testing.T) {
					_, local, remote, gate := actorRecheckFixture(t)
					ctx, cancel := context.WithTimeout(context.Background(), 30*time.Second)
					defer cancel()
					lifecycleExec(t, ctx, local, "UPDATE organization_memberships SET membership_role='owner' WHERE organization_id='owner' AND user_id='owner'")
					if _, err := local.UpsertServiceProviderAdmin(ctx, ServiceProvider{ID: provider, Name: provider, Active: true}); err != nil {
						t.Fatal(err)
					}
					a, err := local.CreateCustomActionAuthorized(ctx, "owner", "owner", provider, CustomActionInput{Code: organizationCustomNamespace("owner") + "calendar.event.create", Name: "Original", RiskLevel: "low"})
					if err != nil {
						t.Fatal(err)
					}
					snapshot, err := local.ProviderActionsSnapshotAuthorized(ctx, "owner", "owner", provider)
					if err != nil {
						t.Fatal(err)
					}
					gate.arm()
					defer gate.release()
					done := make(chan error, 1)
					go func() {
						yes := true
						_, err := remote.PatchCustomActionConditional(ctx, "owner", "owner", provider, a.ID, CustomActionPatch{Name: stringPtr("Must not persist"), GrantableToApplications: &yes}, snapshot.ETag)
						done <- err
					}()
					select {
					case <-gate.reached:
					case <-ctx.Done():
						t.Fatal("service did not reach write gate", ctx.Err())
					}
					// CAS stages and cleans up payloads as separate writes; pause only once.
					gate.mu.Lock()
					gate.paused = false
					gate.mu.Unlock()
					if revocation == "demotion" {
						lifecycleExec(t, ctx, local, "UPDATE organization_memberships SET membership_role='member' WHERE organization_id='owner' AND user_id='owner'")
					} else {
						lifecycleExec(t, ctx, local, "DELETE FROM role_actions WHERE action_id='organization.permission.manage'")
					}
					gate.release()
					select {
					case err = <-done:
					case <-ctx.Done():
						t.Fatal("service failed to finish", ctx.Err())
					}
					if ErrorStatus(err) != 409 {
						t.Fatalf("actual service accepted revoked authority: %v", err)
					}
					current, err := serviceRow[Action](ctx, local, ReadStrong, "SELECT * FROM actions WHERE id=?", a.ID)
					if err != nil || current.GrantableToApplications || current.Name != "Original" {
						t.Fatalf("partial service write: %+v %v", current, err)
					}
				})
			}
		})
	}
}
