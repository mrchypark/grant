package grant

import (
	"context"
	"sync"
	"testing"
)

func ownershipMember(t *testing.T, ctx context.Context, s *Service, id string) {
	t.Helper()
	lifecycleExec(t, ctx, s, "INSERT INTO users(id,display_name,active,created_at) VALUES(?,?,1,?)", id, id, nowString())
	if _, err := s.EnrollOrganizationMemberAuthorized(ctx, "owner", "owner", id); err != nil {
		t.Fatal(err)
	}
}
func TestOwnershipTransferRevokesOnlyOwnerAuthority(t *testing.T) {
	ctx, s := serviceFixture(t)
	ownershipMember(t, ctx, s, "next")
	_, err := s.CreateRoleAuthorized(ctx, "owner", "owner", CreateRoleInput{ID: "reader", Code: "reader", Name: "Reader"})
	if err != nil {
		t.Fatal(err)
	}
	if _, err = s.ReplaceRoleActionsAuthorized(ctx, "owner", "owner", "reader", []string{"organization.profile.read"}); err != nil {
		t.Fatal(err)
	}
	if _, err = s.CreateRoleBindingAuthorized(ctx, "owner", "owner", CreateRoleBindingInput{ID: "independent-reader", SubjectType: "user", SubjectID: "owner", SubjectOrganizationID: "owner", ProviderID: "grant", ResourceType: "organization", ResourceID: "owner", RoleID: "reader", State: "active"}); err != nil {
		t.Fatal(err)
	}
	before, err := s.GetOrganizationOwnerAuthorized(ctx, "owner", "owner")
	if err != nil {
		t.Fatal(err)
	}
	after, err := s.TransferOrganizationOwnerAuthorized(ctx, "owner", "owner", "next", before.ETag)
	if err != nil {
		t.Fatal(err)
	}
	if after.OwnerUserID != "next" || after.ETag == before.ETag {
		t.Fatal(after)
	}
	if lifecycleCount(t, ctx, s, "SELECT * FROM role_bindings WHERE role_id='role-system-management-owner'") != 0 {
		t.Fatal("legacy ownership gained management privileges")
	}
	// Startup seeding is admission-gated by the existing bootstrap user. A
	// transferred bootstrap organization must not regain its original owner.
	if err = s.store.EnsureBootstrapData(ctx); err != nil {
		t.Fatal(err)
	}
	seeded, err := s.organizationOwnerSnapshot(ctx, "owner")
	if err != nil || seeded.OwnerUserID != "next" || seeded.ETag != after.ETag {
		t.Fatal("startup restored old owner", seeded, err)
	}
	if lifecycleCount(t, ctx, s, "SELECT * FROM organization_memberships WHERE organization_id='owner' AND membership_role='owner'") != 1 {
		t.Fatal("owner count changed")
	}
	if lifecycleCount(t, ctx, s, "SELECT * FROM role_bindings WHERE id='independent-reader' AND state='active'") != 1 {
		t.Fatal("independent grant removed")
	}
	for _, tc := range []struct {
		user, action string
		allowed      bool
	}{{"owner", "organization.profile.read", true}, {"owner", "organization.application.manage", false}, {"next", "organization.application.manage", true}} {
		decision, err := s.CheckAuthorization(ctx, AuthorizationSubject{SubjectType: "user", ID: tc.user, OrganizationID: "owner"}, tc.action, AuthorizationResource{ProviderID: "grant", ResourceType: "organization", ID: "owner"}, ReadStrong)
		if err != nil || decision.Allowed != tc.allowed {
			t.Fatal(tc, decision, err)
		}
	}
	if _, err = s.TransferOrganizationOwnerAuthorized(ctx, "owner", "owner", "next", after.ETag); ErrorStatus(err) != 403 {
		t.Fatal("old owner transferred", err)
	}
	if _, err = s.TransferOrganizationOwnerAuthorized(ctx, "next", "owner", "owner", before.ETag); ErrorStatus(err) != 412 {
		t.Fatal("stale revision accepted", err)
	}
	if _, err = s.TransferOrganizationOwnerAuthorized(ctx, "next", "owner", "owner", after.ETag); err != nil {
		t.Fatal("transfer back", err)
	}
	if _, err = s.TransferOrganizationOwnerAuthorized(ctx, "owner", "owner", "next", before.ETag); ErrorStatus(err) != 412 {
		t.Fatal("old revision survived round trip", err)
	}
}
func TestOwnershipTransferRejectsInvalidTargetsAndNonOwner(t *testing.T) {
	ctx, s := serviceFixture(t)
	ownershipMember(t, ctx, s, "next")
	before, err := s.GetOrganizationOwnerAuthorized(ctx, "owner", "owner")
	if err != nil {
		t.Fatal(err)
	}
	for _, tc := range []struct {
		actor, target, tag string
		status             int
	}{{"owner", "next", "", 428}, {"owner", "owner", before.ETag, 400}, {"owner", "missing", before.ETag, 400}, {"next", "owner", before.ETag, 403}} {
		if _, err = s.TransferOrganizationOwnerAuthorized(ctx, tc.actor, "owner", tc.target, tc.tag); ErrorStatus(err) != tc.status {
			t.Fatal(tc, err)
		}
	}
	lifecycleExec(t, ctx, s, "UPDATE users SET active=0 WHERE id='next'")
	if _, err = s.TransferOrganizationOwnerAuthorized(ctx, "owner", "owner", "next", before.ETag); ErrorStatus(err) != 400 {
		t.Fatal(err)
	}
	lifecycleExec(t, ctx, s, "UPDATE users SET active=1 WHERE id='next'")
	lifecycleExec(t, ctx, s, "UPDATE organization_memberships SET status='inactive' WHERE user_id='next'")
	if _, err = s.TransferOrganizationOwnerAuthorized(ctx, "owner", "owner", "next", before.ETag); ErrorStatus(err) != 400 {
		t.Fatal(err)
	}
	lifecycleExec(t, ctx, s, "UPDATE organization_memberships SET status='active',membership_role='owner' WHERE user_id='next'")
	if _, err = s.TransferOrganizationOwnerAuthorized(ctx, "owner", "owner", "next", before.ETag); ErrorStatus(err) != 409 {
		t.Fatal("legacy multiple owners not blocked", err)
	}
}
func TestOwnershipConcurrentTransfersHaveSingleWinner(t *testing.T) {
	ctx, s := serviceFixture(t)
	ownershipMember(t, ctx, s, "next-a")
	ownershipMember(t, ctx, s, "next-b")
	before, err := s.GetOrganizationOwnerAuthorized(ctx, "owner", "owner")
	if err != nil {
		t.Fatal(err)
	}
	var wg sync.WaitGroup
	results := make(chan error, 2)
	for _, target := range []string{"next-a", "next-b"} {
		wg.Add(1)
		go func() {
			defer wg.Done()
			_, e := s.TransferOrganizationOwnerAuthorized(ctx, "owner", "owner", target, before.ETag)
			results <- e
		}()
	}
	wg.Wait()
	close(results)
	success := 0
	for e := range results {
		if e == nil {
			success++
		} else if ErrorStatus(e) != 403 && ErrorStatus(e) != 412 {
			t.Fatal(e)
		}
	}
	if success != 1 {
		t.Fatal("winner count", success)
	}
	if lifecycleCount(t, ctx, s, "SELECT * FROM organization_memberships WHERE organization_id='owner' AND membership_role='owner' AND status='active'") != 1 {
		t.Fatal("lost sole owner")
	}
}

func TestOwnershipTransferCannotRestoreRevokedOwnerAuthority(t *testing.T) {
	ctx, s := serviceFixture(t)
	ownershipMember(t, ctx, s, "next")
	before, err := s.GetOrganizationOwnerAuthorized(ctx, "owner", "owner")
	if err != nil {
		t.Fatal(err)
	}
	lifecycleExec(t, ctx, s, "UPDATE role_bindings SET state='disabled' WHERE subject_id='owner' AND subject_organization_id='owner'")
	if _, err = s.TransferOrganizationOwnerAuthorized(ctx, "owner", "owner", "next", before.ETag); ErrorStatus(err) != 403 {
		t.Fatal("revoked owner recreated authority", err)
	}
	if lifecycleCount(t, ctx, s, "SELECT * FROM organization_memberships WHERE user_id='owner' AND membership_role='owner'") != 1 {
		t.Fatal("owner changed on rejected transfer")
	}
	if lifecycleCount(t, ctx, s, "SELECT * FROM role_bindings WHERE subject_id='next' AND role_id='role-system-bootstrap-owner'") != 0 {
		t.Fatal("target received rejected authority")
	}
}

func TestOwnershipTransferMovesExistingManagementAuthorityOnly(t *testing.T) {
	for _, state := range []string{"active", "disabled"} {
		t.Run(state, func(t *testing.T) {
			ctx, s := serviceFixture(t)
			ownershipMember(t, ctx, s, "next")
			if _, err := s.CreateServiceProviderAdmin(ctx, ServiceProvider{ID: "grant-management", Name: "Management", Active: true}); err != nil {
				t.Fatal(err)
			}
			_, actions, err := ManagementCatalog(1, nil)
			if err != nil {
				t.Fatal(err)
			}
			if _, err = s.ReplaceProviderActionsAdmin(ctx, "grant-management", actions); err != nil {
				t.Fatal(err)
			}
			if err = s.store.Transaction(ctx, managementOwnerBindingStatements("owner", "owner", "owner")); err != nil {
				t.Fatal(err)
			}
			if state == "disabled" {
				lifecycleExec(t, ctx, s, "UPDATE role_bindings SET state='disabled' WHERE role_id='role-system-management-owner'")
			}
			lifecycleApp(t, ctx, s)
			lifecycleExec(t, ctx, s, "UPDATE applications SET first_party=1,consent_policy='auto_approve',provisioning_status='ready' WHERE id='life-app'")
			lifecycleExec(t, ctx, s, "INSERT INTO application_actions(application_id,action_id) SELECT 'life-app',id FROM actions WHERE provider_id='grant-management'")
			check := func(user, action, resourceType, resourceID string, want bool) {
				t.Helper()
				app := "life-app"
				decisions, err := s.evaluateAuthorizations(ctx, []AuthorizationCheck{{Subject: AuthorizationSubject{SubjectType: "user", ID: user, OrganizationID: "owner"}, Action: action, Resource: AuthorizationResource{ProviderID: "grant-management", ResourceType: resourceType, ID: resourceID}}}, ReadStrong, &app)
				if err != nil || len(decisions) != 1 || decisions[0].Allowed != want {
					t.Fatalf("%s %s: %+v %v want %v", user, action, decisions, err, want)
				}
			}
			check("owner", "grant-management.groups.create", "organization", "owner", state == "active")
			before, err := s.GetOrganizationOwnerAuthorized(ctx, "owner", "owner")
			if err != nil {
				t.Fatal(err)
			}
			if _, err = s.TransferOrganizationOwnerAuthorized(ctx, "owner", "owner", "next", before.ETag); err != nil {
				t.Fatal(err)
			}
			check("owner", "grant-management.groups.create", "organization", "owner", false)
			check("next", "grant-management.groups.create", "organization", "owner", state == "active")
			check("next", "grant-management.admin.provider.create", "platform", "grant", false)
			check("next", "grant-management.groups.create", "organization", "another", false)
			// Grant's own coarse check must also pass after the Exposure-facing check.
			if _, err = s.CreateGroupAuthorized(ctx, "next", "owner", UpsertGroupInput{ID: "new-owner-group", Name: "New owner group"}); err != nil {
				t.Fatal("direct Grant management denied", err)
			}
			if lifecycleCount(t, ctx, s, "SELECT * FROM role_bindings WHERE role_id='role-system-management-owner' AND subject_id='owner'") != 0 {
				t.Fatal("old management ownership remained")
			}
			rows, err := s.store.Query(ctx, ReadStrong, "SELECT state FROM role_bindings WHERE role_id='role-system-management-owner' AND subject_id='next'")
			if err != nil || len(rows) != 1 || rowString(rows[0], "state") != state {
				t.Fatal("revived or lost binding state", rows, err)
			}
		})
	}
}
