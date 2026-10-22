package grant

import (
	"encoding/json"
	"testing"
)

func TestRoleActionTransactionCAS(t *testing.T) {
	ctx, s := serviceFixture(t)
	role, err := s.CreateRoleAuthorized(ctx, "owner", "owner", CreateRoleInput{ID: "role-cas", Code: "writer", Name: "Writer"})
	if err != nil {
		t.Fatal(err)
	}
	snap, err := s.RoleActionsSnapshot(ctx, "owner", "owner", role.ID)
	if err != nil {
		t.Fatal(err)
	}
	if snap.ETag == "" {
		t.Fatal("empty etag")
	}

	ctx2, err := checkedActionContext(ctx, snap, snap.ETag, "")
	if err != nil {
		t.Fatal(err)
	}
	if err := s.SetRoleAction(ctx2, "owner", "owner", role.ID, "organization.profile.read", true); err != nil {
		t.Fatal(err)
	}

	stale, err := s.RoleActionsSnapshot(ctx, "owner", "owner", role.ID)
	if err != nil {
		t.Fatal(err)
	}
	_, err = checkedActionContext(ctx, stale, snap.ETag, "")
	if err == nil || ErrorStatus(err) != 412 {
		t.Fatal("stale ETag accepted", err)
	}

	if _, err = s.ReplaceRoleActionsAuthorized(ctx2, "owner", "owner", role.ID, nil); ErrorStatus(err) != 412 {
		t.Fatalf("transaction accepted stale snapshot: %v", err)
	}
	current, err := s.RoleActionsSnapshot(ctx, "owner", "owner", role.ID)
	if err != nil || current.ETag != stale.ETag {
		t.Fatal("rejected replacement changed links", err)
	}
}

func TestProviderCreatePatchStaleETag(t *testing.T) {
	ctx, s := serviceFixture(t)
	if _, err := s.UpsertServiceProviderAdmin(ctx, ServiceProvider{ID: "prov-stale", Name: "Prov", Active: true}); err != nil {
		t.Fatal(err)
	}
	created, err := s.CreateProviderAction(ctx, "prov-stale", Action{ID: "prov-stale.read", ProviderID: "prov-stale", Code: "read", Name: "Read", RiskLevel: "low"})
	if err != nil {
		t.Fatal(err)
	}
	snap1, err := s.ProviderActionsSnapshot(ctx, "prov-stale")
	if err != nil {
		t.Fatal(err)
	}
	guarded, err := checkedActionContext(ctx, snap1, snap1.ETag, "")
	if err != nil {
		t.Fatal(err)
	}
	patched, err := s.PatchProviderAction(ctx, "prov-stale", created.ID, map[string]json.RawMessage{"name": json.RawMessage(`"Read V2"`)})
	if err != nil {
		t.Fatal(err)
	}
	if patched.Name != "Read V2" {
		t.Fatal("patch did not apply", patched)
	}
	_, err = s.ReplaceProviderActionsConditional(ctx, "prov-stale", []Action{{ID: "prov-stale.read", ProviderID: "prov-stale", Code: "read", Name: "Read V2", RiskLevel: "low"}}, snap1.ETag)
	if err == nil || ErrorStatus(err) != 412 {
		t.Fatal("stale ETag accepted on provider action", err)
	}
	if _, err = s.ReplaceProviderActionsAdmin(guarded, "prov-stale", nil); ErrorStatus(err) != 412 {
		t.Fatalf("provider transaction accepted stale snapshot: %v", err)
	}
}

func TestBootstrapOwnerAtomicReservedUserIDRejectsSecondSubject(t *testing.T) {
	ctx, s := serviceFixture(t)
	ident1 := ExternalIdentityMaterialization{Provider: "pocketbase", Issuer: "https://auth.example.test", CollectionID: "users_id", ExternalUserID: "pb-res1", DisplayName: "Res1"}
	cmd1 := OwnerBootstrapCommand{Identity: ident1, OrganizationID: "owner", ReservedUserID: "owner"}
	r, err := s.store.BootstrapOwnerAtomic(ctx, cmd1)
	if err != nil {
		t.Fatal(err)
	}
	if r.UserID != "owner" {
		t.Fatal("expected reserved user id", r)
	}
	ident2 := ExternalIdentityMaterialization{Provider: "pocketbase", Issuer: "https://auth.example.test", CollectionID: "users_id", ExternalUserID: "pb-res2", DisplayName: "Res2"}
	cmd2 := OwnerBootstrapCommand{Identity: ident2, OrganizationID: "owner", ReservedUserID: "owner"}
	_, err = s.store.BootstrapOwnerAtomic(ctx, cmd2)
	if err == nil {
		t.Fatal("second subject accepted for reserved user id")
	}
}

func TestDeleteProviderActionProtectsActiveCatalog(t *testing.T) {
	ctx, s := serviceFixture(t)
	if err := s.store.InsertApplicationWithRedirectURI(ctx, Application{ID: "catalog-pep-app", OrganizationID: "owner", Slug: "catalog-pep-app", Name: "PEP", ApplicationType: "confidential", AppType: "internal", FirstParty: true, ConsentPolicy: "auto_approve", Active: true, ProvisioningStatus: "ready", CreatedByUserID: "owner"}, nil); err != nil {
		t.Fatal(err)
	}
	stage, actions, err := ManagementCatalog(1, nil)
	if err != nil {
		t.Fatal(err)
	}
	if _, err = s.UpsertServiceProviderAdmin(ctx, ServiceProvider{ID: "grant-management", Name: "Management", Active: true}); err != nil {
		t.Fatal(err)
	}
	if _, err = s.ReplaceProviderActionsAdmin(ctx, "grant-management", actions); err != nil {
		t.Fatal(err)
	}
	if _, err = s.SetProviderPepClientAdmin(ctx, "grant-management", "catalog-pep", UpsertProviderPepClient{ApplicationID: "catalog-pep-app", Lifecycle: "active"}); err != nil {
		t.Fatal(err)
	}
	if _, err = s.StageHttpCapabilityGenerationAdmin(ctx, "grant-management", stage); err != nil {
		t.Fatal(err)
	}
	if _, err = s.ActivateHttpCapabilityGenerationAdmin(ctx, "grant-management", ActivateHttpCapabilityGeneration{TargetGeneration: 1, TargetCatalogDigest: stage.CatalogDigest}); err != nil {
		t.Fatal(err)
	}
	before, err := s.ProviderActionsSnapshot(ctx, "grant-management")
	if err != nil {
		t.Fatal(err)
	}
	if err = s.DeleteProviderAction(ctx, "grant-management", "grant-management.groups.read"); ErrorStatus(err) != 409 {
		t.Fatalf("active reference delete: %v", err)
	}
	after, err := s.ProviderActionsSnapshot(ctx, "grant-management")
	if err != nil || after.ETag != before.ETag {
		t.Fatal("rejected deletion modified actions", err)
	}
}

func TestRauthyIdentityNeverAdoptsUserByEmailOrRawID(t *testing.T) {
	ctx, s := serviceFixture(t)
	email := "owner@example.test"
	if _, err := s.store.Exec(ctx, "UPDATE users SET email=? WHERE id='owner'", email); err != nil {
		t.Fatal(err)
	}
	identity := ExternalIdentity{Provider: "rauthy", ExternalUserID: "owner", Email: &email, DisplayName: "Different subject", Active: true}
	user, err := s.store.EnsureExternalIdentity(ctx, ExternalIdentityMaterialization{Provider: identity.Provider, Issuer: identity.Issuer, CollectionID: identity.CollectionID, ExternalUserID: identity.ExternalUserID, Email: identity.Email, DisplayName: identity.DisplayName})
	if err != nil {
		t.Fatal(err)
	}
	if user.ID == "owner" {
		t.Fatal("unmapped subject adopted existing owner")
	}
	if allowed, err := s.store.HasActiveOrganizationMembershipWithConsistency(ctx, user.ID, "owner", ReadStrong); err != nil || allowed {
		t.Fatal("new identity inherited membership", err)
	}
	again, err := s.store.EnsureExternalIdentity(ctx, ExternalIdentityMaterialization{Provider: identity.Provider, Issuer: identity.Issuer, CollectionID: identity.CollectionID, ExternalUserID: identity.ExternalUserID, Email: identity.Email, DisplayName: identity.DisplayName})
	if err != nil || again.ID != user.ID {
		t.Fatal("explicit identity mapping was not stable", err)
	}
	rows, err := s.store.Query(ctx, ReadStrong, "SELECT display_name FROM users WHERE id='owner'")
	if err != nil || rowString(rows[0], "display_name") != "Owner" {
		t.Fatal("existing owner overwritten", err)
	}
}

func TestBootstrapOwnerConcurrentAdmission(t *testing.T) {
	ctx, s := serviceFixture(t)
	result := make(chan error, 2)
	for _, subject := range []string{"first", "second"} {
		go func(subject string) {
			_, err := s.store.BootstrapOwnerAtomic(ctx, OwnerBootstrapCommand{Identity: ExternalIdentityMaterialization{Provider: "rauthy", ExternalUserID: subject, DisplayName: subject}, OrganizationID: "racing-org"})
			result <- err
		}(subject)
	}
	successes := 0
	for range 2 {
		if err := <-result; err == nil {
			successes++
		} else if ErrorStatus(err) != 409 {
			t.Fatal(err)
		}
	}
	if successes != 1 {
		t.Fatalf("admitted %d concurrent owners", successes)
	}
	rows, err := s.store.Query(ctx, ReadStrong, "SELECT COUNT(*) AS n FROM organization_memberships WHERE organization_id='racing-org' AND membership_role='owner'")
	if err != nil || rowInt(rows[0], "n") != 1 {
		t.Fatalf("owner rows: %v %v", rows, err)
	}
	rows, err = s.store.Query(ctx, ReadStrong, "SELECT COUNT(*) AS n FROM user_identities WHERE provider='rauthy' AND external_user_id IN('first','second')")
	if err != nil || rowInt(rows[0], "n") != 1 {
		t.Fatalf("partial failed bootstrap: %v %v", rows, err)
	}
}
