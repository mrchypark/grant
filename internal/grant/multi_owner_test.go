package grant

import (
	"sync"
	"testing"
)

func TestOrganizationOwnersAddDemoteAndProtectLastOwner(t *testing.T) {
	ctx, s := serviceFixture(t)
	ownershipMember(t, ctx, s, "second")
	ownershipMember(t, ctx, s, "third")

	snap, err := s.ListOrganizationOwnersAuthorized(ctx, "owner", "owner")
	if err != nil || len(snap.Owners) != 1 {
		t.Fatalf("initial owners: %+v %v", snap.Owners, err)
	}
	if _, err = s.ChangeOrganizationOwnerAuthorized(ctx, "owner", "owner", "second", "", true); ErrorStatus(err) != 428 {
		t.Fatalf("missing If-Match: %v", err)
	}
	added, err := s.ChangeOrganizationOwnerAuthorized(ctx, "owner", "owner", "second", snap.ETag, true)
	if err != nil || len(added.Owners) != 2 {
		t.Fatalf("add owner: %+v %v", added.Owners, err)
	}
	if _, err = s.ChangeOrganizationOwnerAuthorized(ctx, "owner", "owner", "second", snap.ETag, true); ErrorStatus(err) != 412 {
		t.Fatalf("stale add accepted: %v", err)
	}
	idempotent, err := s.ChangeOrganizationOwnerAuthorized(ctx, "owner", "owner", "second", added.ETag, true)
	if err != nil || len(idempotent.Owners) != 2 {
		t.Fatalf("idempotent add: %+v %v", idempotent.Owners, err)
	}
	removed, err := s.ChangeOrganizationOwnerAuthorized(ctx, "owner", "owner", "second", idempotent.ETag, false)
	if err != nil || len(removed.Owners) != 1 || removed.Owners[0].UserID != "owner" {
		t.Fatalf("remove owner: %+v %v", removed.Owners, err)
	}
	if _, err = s.ChangeOrganizationOwnerAuthorized(ctx, "owner", "owner", "owner", removed.ETag, false); ErrorStatus(err) != 409 {
		t.Fatalf("removed sole owner: %v", err)
	}
	if _, err = s.ChangeOrganizationOwnerAuthorized(ctx, "owner", "owner", "third", removed.ETag, false); ErrorStatus(err) != 409 {
		t.Fatalf("removed non-owner instead of preserving sole owner: %v", err)
	}
	if _, err = s.SetUserActiveAdmin(ctx, "owner", false); ErrorStatus(err) != 409 {
		t.Fatalf("disabled sole owner: %v", err)
	}
	if err = s.store.EnsureBootstrapData(ctx); err != nil {
		t.Fatal(err)
	}
	final, err := s.ListOrganizationOwnersAuthorized(ctx, "owner", "owner")
	if err != nil || len(final.Owners) != 1 || final.Owners[0].UserID != "owner" {
		t.Fatalf("bootstrap restored removed owner: %+v %v", final.Owners, err)
	}
}

func TestOrganizationOwnersCountOnlyActiveMembershipsAndUsers(t *testing.T) {
	ctx, s := serviceFixture(t)
	ownershipMember(t, ctx, s, "inactive-membership")
	ownershipMember(t, ctx, s, "inactive-user")
	snap, err := s.ListOrganizationOwnersAuthorized(ctx, "owner", "owner")
	if err != nil {
		t.Fatal(err)
	}
	for _, id := range []string{"inactive-membership", "inactive-user"} {
		snap, err = s.ChangeOrganizationOwnerAuthorized(ctx, "owner", "owner", id, snap.ETag, true)
		if err != nil {
			t.Fatal(err)
		}
	}
	lifecycleExec(t, ctx, s, "UPDATE organization_memberships SET status='inactive' WHERE organization_id='owner' AND user_id='inactive-membership'")
	lifecycleExec(t, ctx, s, "UPDATE users SET active=0 WHERE id='inactive-user'")
	active, err := s.ListOrganizationOwnersAuthorized(ctx, "owner", "owner")
	if err != nil || len(active.Owners) != 1 || active.Owners[0].UserID != "owner" {
		t.Fatalf("inactive owners counted: %+v %v", active.Owners, err)
	}
	if _, err = s.ChangeOrganizationOwnerAuthorized(ctx, "owner", "owner", "owner", active.ETag, false); ErrorStatus(err) != 409 {
		t.Fatalf("inactive owners allowed removal of last active owner: %v", err)
	}
}

func TestOrganizationOwnerAuthorityRequiresActiveMembership(t *testing.T) {
	ctx, s := serviceFixture(t)
	ownershipMember(t, ctx, s, "delegate")
	if _, err := s.CreateRoleAuthorized(ctx, "owner", "owner", CreateRoleInput{ID: "owner-manager", Code: "owner-manager", Name: "Owner Manager"}); err != nil {
		t.Fatal(err)
	}
	if _, err := s.ReplaceRoleActionsAuthorized(ctx, "owner", "owner", "owner-manager", []string{"organization.permission.manage"}); err != nil {
		t.Fatal(err)
	}
	if _, err := s.CreateRoleBindingAuthorized(ctx, "owner", "owner", CreateRoleBindingInput{ID: "delegate-management", SubjectType: "user", SubjectID: "delegate", SubjectOrganizationID: "owner", ProviderID: "grant", ResourceType: "organization", ResourceID: "owner", RoleID: "owner-manager", State: "active"}); err != nil {
		t.Fatal(err)
	}
	if err := s.store.Transaction(ctx, []Statement{{SQL: "INSERT INTO role_bindings(id,subject_type,subject_id,subject_organization_id,provider_id,resource_type,resource_id,role_id,state,created_by_user_id,created_at,updated_at) VALUES('stale-system-owner','user','delegate','owner','grant','organization','owner','role-system-bootstrap-owner','active','owner',?,?)", Args: []any{nowString(), nowString()}}}); err != nil {
		t.Fatal(err)
	}
	snap, err := s.ListOrganizationOwnersAuthorized(ctx, "owner", "owner")
	if err != nil {
		t.Fatal(err)
	}
	if _, err = s.ChangeOrganizationOwnerAuthorized(ctx, "delegate", "owner", "delegate", snap.ETag, true); ErrorStatus(err) != 403 {
		t.Fatalf("delegated/legacy authority managed owners: %v", err)
	}
	if _, err = s.CreateOperableCustomActionAuthorized(ctx, "delegate", "owner", CustomActionInput{Code: operableCustomNamespace("owner") + "calendar.event.create", Name: "Create event", RiskLevel: "low"}); ErrorStatus(err) != 403 {
		t.Fatalf("delegated/legacy authority created custom action: %v", err)
	}
}

func TestOrganizationOwnerCustomActionsFollowMembership(t *testing.T) {
	ctx, s := serviceFixture(t)
	if _, err := s.UpsertServiceProviderAdmin(ctx, ServiceProvider{ID: "operable", Name: "Operable", Active: true}); err != nil {
		t.Fatal(err)
	}
	ownershipMember(t, ctx, s, "next-owner")
	before, err := s.ListOrganizationOwnersAuthorized(ctx, "owner", "owner")
	if err != nil {
		t.Fatal(err)
	}
	added, err := s.ChangeOrganizationOwnerAuthorized(ctx, "owner", "owner", "next-owner", before.ETag, true)
	if err != nil {
		t.Fatal(err)
	}
	code := operableCustomNamespace("owner") + "calendar.event.create"
	created, err := s.CreateOperableCustomActionAuthorized(ctx, "next-owner", "owner", CustomActionInput{Code: code, Name: "Create event", RiskLevel: "medium"})
	if err != nil {
		t.Fatal(err)
	}
	removed, err := s.ChangeOrganizationOwnerAuthorized(ctx, "owner", "owner", "next-owner", added.ETag, false)
	if err != nil {
		t.Fatal(err)
	}
	patch := CustomActionPatch{Name: stringPtr("Edited")}
	if _, err = s.PatchOperableCustomActionAuthorized(ctx, "next-owner", "owner", created.ID, patch); ErrorStatus(err) != 403 {
		t.Fatalf("demoted owner edited custom action: %v", err)
	}
	if err = s.DeleteOperableCustomActionAuthorized(ctx, "next-owner", "owner", created.ID); ErrorStatus(err) != 403 {
		t.Fatalf("demoted owner deleted custom action: %v", err)
	}
	if _, err = s.PatchOperableCustomActionAuthorized(ctx, "owner", "owner", created.ID, patch); err != nil {
		t.Fatalf("remaining owner cannot edit custom action (snapshot %v): %v", removed.ETag, err)
	}
}

func TestOrganizationOwnerDemotionRacesAccountDisable(t *testing.T) {
	ctx, s := serviceFixture(t)
	ownershipMember(t, ctx, s, "second")
	snap, err := s.ListOrganizationOwnersAuthorized(ctx, "owner", "owner")
	if err != nil {
		t.Fatal(err)
	}
	snap, err = s.ChangeOrganizationOwnerAuthorized(ctx, "owner", "owner", "second", snap.ETag, true)
	if err != nil {
		t.Fatal(err)
	}
	start := make(chan struct{})
	var wg sync.WaitGroup
	results := make(chan error, 2)
	wg.Add(2)
	go func() {
		defer wg.Done()
		<-start
		_, e := s.ChangeOrganizationOwnerAuthorized(ctx, "owner", "owner", "second", snap.ETag, false)
		results <- e
	}()
	go func() {
		defer wg.Done()
		<-start
		_, e := s.SetUserActiveAdmin(ctx, "owner", false)
		results <- e
	}()
	close(start)
	wg.Wait()
	close(results)
	for err := range results {
		if err != nil && ErrorStatus(err) != 403 && ErrorStatus(err) != 409 && ErrorStatus(err) != 412 {
			t.Fatalf("unexpected concurrent result: %v", err)
		}
	}
	if n := lifecycleCount(t, ctx, s, "SELECT * FROM organization_memberships m JOIN users u ON u.id=m.user_id WHERE m.organization_id='owner' AND m.membership_role='owner' AND m.status='active' AND u.active=1"); n < 1 {
		t.Fatal("concurrent demotion and account disable removed every active owner")
	}
}

func TestOrganizationOwnerDemotionsRaceWithoutRemovingLastOwner(t *testing.T) {
	ctx, s := serviceFixture(t)
	ownershipMember(t, ctx, s, "second")
	snap, err := s.ListOrganizationOwnersAuthorized(ctx, "owner", "owner")
	if err != nil {
		t.Fatal(err)
	}
	added, err := s.ChangeOrganizationOwnerAuthorized(ctx, "owner", "owner", "second", snap.ETag, true)
	if err != nil {
		t.Fatal(err)
	}
	start := make(chan struct{})
	var wg sync.WaitGroup
	results := make(chan error, 2)
	for _, request := range []struct{ actor, target string }{{"owner", "second"}, {"second", "owner"}} {
		wg.Add(1)
		go func(actor, target string) {
			defer wg.Done()
			<-start
			_, e := s.ChangeOrganizationOwnerAuthorized(ctx, actor, "owner", target, added.ETag, false)
			results <- e
		}(request.actor, request.target)
	}
	close(start)
	wg.Wait()
	close(results)
	successes := 0
	for err := range results {
		if err == nil {
			successes++
		} else if ErrorStatus(err) != 403 && ErrorStatus(err) != 409 && ErrorStatus(err) != 412 {
			t.Fatalf("unexpected concurrent result: %v", err)
		}
	}
	if successes != 1 {
		t.Fatalf("demotion winner count: %d", successes)
	}
	if n := lifecycleCount(t, ctx, s, "SELECT * FROM organization_memberships m JOIN users u ON u.id=m.user_id WHERE m.organization_id='owner' AND m.membership_role='owner' AND m.status='active' AND u.active=1"); n < 1 {
		t.Fatal("concurrent owner demotions removed every active owner")
	}
}
