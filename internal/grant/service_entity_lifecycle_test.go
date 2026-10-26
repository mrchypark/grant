package grant

import (
	"context"
	"testing"
)

func lifecycleExec(t *testing.T, ctx context.Context, s *Service, q string, args ...any) {
	t.Helper()
	if _, err := s.store.Exec(ctx, q, args...); err != nil {
		t.Fatal(err)
	}
}
func lifecycleCount(t *testing.T, ctx context.Context, s *Service, q string, args ...any) int {
	t.Helper()
	r, err := s.store.Query(ctx, ReadStrong, q, args...)
	if err != nil {
		t.Fatal(err)
	}
	return len(r)
}
func lifecycleApp(t *testing.T, ctx context.Context, s *Service) {
	t.Helper()
	_, err := s.CreateApplication(ctx, "owner", "owner", CreateApplicationInput{ID: "life-app", Slug: "life-app", Name: "Life", ApplicationType: "public", AppType: "web", Active: true})
	if err != nil {
		t.Fatal(err)
	}
}

func TestLifecycleMemberRemovalDoesNotRestorePersonalAccess(t *testing.T) {
	ctx, s := serviceFixture(t)
	lifecycleExec(t, ctx, s, "INSERT INTO users(id,display_name,active,created_at) VALUES('member','Member',1,?)", nowString())
	if _, err := s.EnrollOrganizationMemberAuthorized(ctx, "owner", "owner", "member"); err != nil {
		t.Fatal(err)
	}
	if _, err := s.CreateGroupAuthorized(ctx, "owner", "owner", UpsertGroupInput{ID: "life-group", Name: "Group"}); err != nil {
		t.Fatal(err)
	}
	if _, err := s.AddGroupMemberAuthorized(ctx, "owner", "owner", "life-group", "member"); err != nil {
		t.Fatal(err)
	}
	if _, err := s.CreateRoleAuthorized(ctx, "owner", "owner", CreateRoleInput{ID: "life-role", Code: "life", Name: "Life"}); err != nil {
		t.Fatal(err)
	}
	if _, err := s.ReplaceRoleActionsAuthorized(ctx, "owner", "owner", "life-role", []string{"organization.profile.read"}); err != nil {
		t.Fatal(err)
	}
	for _, subject := range []struct{ id, kind, who string }{{"life-direct", "user", "member"}, {"life-group-bind", "group", "life-group"}} {
		_, err := s.CreateRoleBindingAuthorized(ctx, "owner", "owner", CreateRoleBindingInput{ID: subject.id, SubjectType: subject.kind, SubjectID: subject.who, SubjectOrganizationID: "owner", ProviderID: "grant", ResourceType: "organization", ResourceID: "owner", RoleID: "life-role", State: "active"})
		if err != nil {
			t.Fatal(err)
		}
	}
	lifecycleApp(t, ctx, s)
	lifecycleExec(t, ctx, s, "INSERT INTO application_grants(id,user_id,application_id,organization_id,status,consented_at,created_at,updated_at) VALUES('life-consent','member','life-app','owner','active',?,?,?)", nowString(), nowString(), nowString())
	check := func(want bool) {
		t.Helper()
		d, err := s.CheckAuthorization(ctx, AuthorizationSubject{SubjectType: "user", ID: "member", OrganizationID: "owner"}, "organization.profile.read", AuthorizationResource{ProviderID: "grant", ResourceType: "organization", ID: "owner"}, ReadStrong)
		if err != nil || d.Allowed != want {
			t.Fatalf("allowed=%v want=%v err=%v", d.Allowed, want, err)
		}
	}
	check(true)
	if err := s.RemoveOrganizationMemberAuthorized(ctx, "member", "owner", "owner"); ErrorStatus(err) != 403 {
		t.Fatalf("unauthorized removal: %v", err)
	}
	if err := s.RemoveOrganizationMemberAuthorized(ctx, "owner", "owner", "owner"); ErrorStatus(err) != 409 {
		t.Fatalf("owner removal: %v", err)
	}
	if err := s.RemoveOrganizationMemberAuthorized(ctx, "owner", "owner", "member"); err != nil {
		t.Fatal(err)
	}
	check(false)
	if _, err := s.EnrollOrganizationMemberAuthorized(ctx, "owner", "owner", "member"); err != nil {
		t.Fatal(err)
	}
	check(false)
	for _, q := range []string{"SELECT * FROM group_memberships WHERE group_id='life-group'", "SELECT * FROM role_bindings WHERE id='life-direct'", "SELECT * FROM application_grants WHERE id='life-consent' AND status='active'"} {
		if lifecycleCount(t, ctx, s, q) != 0 {
			t.Fatal("stale access", q)
		}
	}
}

func TestLifecycleApplicationRevocationAndSafeDeletion(t *testing.T) {
	ctx, s := serviceFixture(t)
	lifecycleApp(t, ctx, s)
	lifecycleExec(t, ctx, s, "INSERT INTO application_grants(id,user_id,application_id,organization_id,status,consented_at,created_at,updated_at) VALUES('life-consent','owner','life-app','owner','active',?,?,?)", nowString(), nowString(), nowString())
	lifecycleExec(t, ctx, s, "INSERT INTO application_grant_actions(application_grant_id,action_id) VALUES('life-consent','organization.profile.read')")
	lifecycleExec(t, ctx, s, "INSERT INTO application_actions(application_id,action_id) VALUES('life-app','organization.profile.read')")
	if _, err := s.SetApplicationActiveAuthorized(ctx, "owner", "owner", "life-app", false); err != nil {
		t.Fatal(err)
	}
	if _, err := s.SetApplicationActiveAuthorized(ctx, "owner", "owner", "life-app", true); err != nil {
		t.Fatal(err)
	}
	if lifecycleCount(t, ctx, s, "SELECT * FROM application_grants WHERE id='life-consent' AND status='active'") != 0 {
		t.Fatal("reactivation restored consent")
	}
	name := "Renamed"
	if a, err := s.PatchApplicationAuthorized(ctx, "owner", "owner", "life-app", MetadataPatch{Name: &name}); err != nil || a.Name != name || !a.Active {
		t.Fatal(a, err)
	}
	if err := s.DeleteApplicationAuthorized(ctx, "owner", "other", "life-app"); ErrorStatus(err) != 403 {
		t.Fatalf("crossorg %v", err)
	}
	lifecycleExec(t, ctx, s, "INSERT INTO provider_pep_clients(pep_client_id,application_id,provider_id,lifecycle,created_at,updated_at) VALUES('life-pep','life-app','grant','disabled',?,?)", nowString(), nowString())
	lifecycleExec(t, ctx, s, "INSERT INTO provider_pep_client_receipts(receipt_id,pep_client_id,application_id,provider_id,lifecycle,changed_at,changed_by) VALUES('life-pep-receipt','life-pep','life-app','grant','disabled',?,'test')", nowString())
	if err := s.DeleteApplicationAuthorized(ctx, "owner", "owner", "life-app"); err != nil {
		t.Fatalf("application deletion must not affect independently registered PEP: %v", err)
	}
	if lifecycleCount(t, ctx, s, "SELECT * FROM provider_pep_clients WHERE pep_client_id='life-pep' AND lifecycle='disabled'") != 1 {
		t.Fatal("application deletion removed or changed PEP lifecycle")
	}
	if lifecycleCount(t, ctx, s, "SELECT * FROM provider_pep_client_receipts WHERE receipt_id='life-pep-receipt' AND application_id='life-app'") != 1 {
		t.Fatal("application deletion removed historical PEP receipt")
	}
	for _, q := range []string{"SELECT * FROM applications WHERE id='life-app'", "SELECT * FROM application_grants WHERE application_id='life-app'", "SELECT * FROM application_grant_actions WHERE application_grant_id='life-consent'", "SELECT * FROM application_actions WHERE application_id='life-app'"} {
		if lifecycleCount(t, ctx, s, q) != 0 {
			t.Fatal("orphan", q)
		}
	}
}

func TestLifecycleApplicationRedirectPatchIsPreBindingOnly(t *testing.T) {
	ctx, s := serviceFixture(t)
	lifecycleApp(t, ctx, s)
	callback := "https://operable.dev.conalog.com/oauth/callback"
	updated, err := s.PatchApplicationMetadataAuthorized(ctx, "owner", "owner", "life-app", ApplicationMetadataPatch{
		RedirectURI: PatchText{Present: true, Value: &callback},
	})
	if err != nil || updated.ProvisioningStatus != "unprovisioned" || updated.RauthyClientID != nil {
		t.Fatalf("redirect patch changed binding state: %+v %v", updated, err)
	}
	read, err := s.GetApplicationDetailAuthorized(ctx, "owner", "owner", "life-app", ReadStrong)
	if err != nil || read.RedirectURI == nil || *read.RedirectURI != callback {
		t.Fatalf("redirect missing after patch: %+v %v", read, err)
	}

	name := "Renamed"
	updated, err = s.PatchApplicationMetadataAuthorized(ctx, "owner", "owner", "life-app", ApplicationMetadataPatch{Name: &name})
	if err != nil || updated.Name != name {
		t.Fatalf("name patch failed: %+v %v", updated, err)
	}
	read, err = s.GetApplicationDetailAuthorized(ctx, "owner", "owner", "life-app", ReadStrong)
	if err != nil || read.RedirectURI == nil || *read.RedirectURI != callback {
		t.Fatalf("omitted redirectUri was not preserved: %+v %v", read, err)
	}

	for _, patch := range []PatchText{{Present: true}, {Present: true, Value: ptr("")}, {Present: true, Value: ptr("https://operable.dev.conalog.com/oauth/callback?next=elsewhere")}} {
		if _, err = s.PatchApplicationMetadataAuthorized(ctx, "owner", "owner", "life-app", ApplicationMetadataPatch{RedirectURI: patch}); ErrorStatus(err) != 400 {
			t.Fatalf("invalid redirect patch accepted: %+v err=%v", patch, err)
		}
	}
	read, err = s.GetApplicationDetailAuthorized(ctx, "owner", "owner", "life-app", ReadStrong)
	if err != nil || read.RedirectURI == nil || *read.RedirectURI != callback {
		t.Fatalf("rejected patch changed redirectUri: %+v %v", read, err)
	}

	app, err := s.store.GetApplicationWithConsistency(ctx, "life-app", ReadStrong)
	if err != nil {
		t.Fatal(err)
	}
	if _, err = s.store.BindApplicationIdentity(ctx, app, "bound-client", read.RedirectURI); err != nil {
		t.Fatal("test bind", err)
	}
	changed := "https://operable.dev.conalog.com/other"
	if _, err = s.PatchApplicationMetadataAuthorized(ctx, "owner", "owner", "life-app", ApplicationMetadataPatch{RedirectURI: PatchText{Present: true, Value: &changed}}); ErrorStatus(err) != 409 {
		t.Fatalf("bound app redirect changed: %v", err)
	}
}

func TestLifecycleApplicationRedirectPatchRejectsPlatformManagedApp(t *testing.T) {
	ctx, s := serviceFixture(t)
	lifecycleApp(t, ctx, s)
	lifecycleExec(t, ctx, s, "UPDATE applications SET first_party=1,consent_policy='auto_approve' WHERE id='life-app'")
	callback := "https://operable.dev.conalog.com/oauth/callback"
	if _, err := s.PatchApplicationMetadataAuthorized(ctx, "owner", "owner", "life-app", ApplicationMetadataPatch{RedirectURI: PatchText{Present: true, Value: &callback}}); ErrorStatus(err) != 409 {
		t.Fatalf("platform app redirect changed: %v", err)
	}
	if lifecycleCount(t, ctx, s, "SELECT * FROM application_redirect_uris WHERE application_id='life-app'") != 0 {
		t.Fatal("platform app redirect manifest changed")
	}
}

func TestLifecycleApplicationRedirectPatchRespectsSecurityLeaseAndBindSnapshot(t *testing.T) {
	ctx, s := serviceFixture(t)
	lifecycleApp(t, ctx, s)
	callback := "https://operable.dev.conalog.com/oauth/callback"
	lifecycleExec(t, ctx, s, "UPDATE applications SET security_mutation_token='lease',security_mutation_expires_at='9999-01-01T00:00:00Z' WHERE id='life-app'")
	if _, err := s.PatchApplicationMetadataAuthorized(ctx, "owner", "owner", "life-app", ApplicationMetadataPatch{RedirectURI: PatchText{Present: true, Value: &callback}}); ErrorStatus(err) != 409 {
		t.Fatalf("redirect changed under security lease: %v", err)
	}
	lifecycleExec(t, ctx, s, "UPDATE applications SET security_mutation_token=NULL,security_mutation_expires_at=NULL WHERE id='life-app'")

	app, err := s.store.GetApplicationWithConsistency(ctx, "life-app", ReadStrong)
	if err != nil {
		t.Fatal(err)
	}
	if _, err = s.PatchApplicationMetadataAuthorized(ctx, "owner", "owner", "life-app", ApplicationMetadataPatch{RedirectURI: PatchText{Present: true, Value: &callback}}); err != nil {
		t.Fatal(err)
	}
	if _, err = s.store.BindApplicationIdentity(ctx, app, "stale-client", nil); err == nil {
		t.Fatal("binding with stale redirect snapshot succeeded")
	}
	bound, err := s.store.GetApplicationWithConsistency(ctx, "life-app", ReadStrong)
	if err != nil || bound.ProvisioningStatus != "unprovisioned" || bound.RauthyClientID != nil {
		t.Fatalf("failed stale bind changed app: %+v %v", bound, err)
	}
}

func TestLifecycleRoleAndAdminConsentBoundaries(t *testing.T) {
	ctx, s := serviceFixture(t)
	if err := s.DeleteRoleAuthorized(ctx, "owner", "owner", "role-system-bootstrap-owner"); ErrorStatus(err) != 404 {
		t.Fatalf("global role deletion %v", err)
	}
	_, err := s.CreateRoleAuthorized(ctx, "owner", "owner", CreateRoleInput{ID: "life-role", Code: "life", Name: "Life"})
	if err != nil {
		t.Fatal(err)
	}
	_, err = s.CreateRoleBindingAuthorized(ctx, "owner", "owner", CreateRoleBindingInput{ID: "life-bind", SubjectType: "user", SubjectID: "owner", SubjectOrganizationID: "owner", ProviderID: "grant", ResourceType: "organization", ResourceID: "owner", RoleID: "life-role", State: "disabled"})
	if err != nil {
		t.Fatal(err)
	}
	if err = s.DeleteRoleAuthorized(ctx, "owner", "owner", "life-role"); ErrorStatus(err) != 409 {
		t.Fatalf("bound role %v", err)
	}
	if err = s.DeleteRoleBindingAuthorized(ctx, "owner", "owner", "life-bind"); err != nil {
		t.Fatal(err)
	}
	if err = s.DeleteRoleAuthorized(ctx, "owner", "owner", "life-role"); err != nil {
		t.Fatal(err)
	}
	lifecycleApp(t, ctx, s)
	lifecycleExec(t, ctx, s, "INSERT INTO application_grants(id,user_id,application_id,organization_id,status,consented_at,created_at,updated_at) VALUES('life-consent','owner','life-app','owner','active',?,?,?)", nowString(), nowString(), nowString())
	if err = s.RevokeApplicationGrantAuthorized(ctx, "owner", "other", "life-consent"); ErrorStatus(err) != 403 {
		t.Fatalf("crossorg revocation %v", err)
	}
	if err = s.RevokeApplicationGrantAuthorized(ctx, "owner", "owner", "life-consent"); err != nil {
		t.Fatal(err)
	}
	if err = s.RevokeApplicationGrantAuthorized(ctx, "owner", "owner", "life-consent"); err != nil {
		t.Fatal("idempotent revoke", err)
	}
	if lifecycleCount(t, ctx, s, "SELECT * FROM application_grants WHERE id='life-consent' AND status='revoked' AND revoked_by_user_id='owner'") != 1 {
		t.Fatal("missing revocation audit")
	}
}

func TestLifecycleMutationGuardRechecksAuthorityInsideTransaction(t *testing.T) {
	ctx, s := serviceFixture(t)
	guard := lifecycleActorGuard("owner", "owner", "organization.application.manage")
	// Simulate revocation between the service's initial authorization and its write.
	lifecycleExec(t, ctx, s, "UPDATE organization_memberships SET membership_role='member',updated_at=? WHERE organization_id='owner' AND user_id='owner'", nowString())
	err := s.store.Transaction(ctx, []Statement{guard, {SQL: "UPDATE users SET display_name='should-not-write' WHERE id='owner'"}})
	if err == nil {
		t.Fatal("transaction accepted stale authority")
	}
	if lifecycleCount(t, ctx, s, "SELECT id FROM users WHERE id='owner' AND display_name='should-not-write'") != 0 {
		t.Fatal("failed guard did not roll back mutation")
	}
}

func TestLifecyclePlatformApplicationAndSecurityLeaseProtected(t *testing.T) {
	ctx, s := serviceFixture(t)
	lifecycleApp(t, ctx, s)
	lifecycleExec(t, ctx, s, "UPDATE applications SET first_party=1,consent_policy='auto_approve' WHERE id='life-app'")
	if err := s.DeleteApplicationAuthorized(ctx, "owner", "owner", "life-app"); ErrorStatus(err) != 403 {
		t.Fatalf("first-party delete: %v", err)
	}
	if _, err := s.SetApplicationActiveAuthorized(ctx, "owner", "owner", "life-app", false); ErrorStatus(err) != 403 {
		t.Fatalf("first-party deactivate: %v", err)
	}
	lifecycleExec(t, ctx, s, "UPDATE applications SET first_party=0,consent_policy='explicit',security_mutation_token='lease',security_mutation_expires_at='9999-01-01T00:00:00Z' WHERE id='life-app'")
	if err := s.DeleteApplicationAuthorized(ctx, "owner", "owner", "life-app"); ErrorStatus(err) != 409 {
		t.Fatalf("leased delete: %v", err)
	}
	if _, err := s.SetApplicationActiveAuthorized(ctx, "owner", "owner", "life-app", false); ErrorStatus(err) != 409 {
		t.Fatalf("leased deactivate: %v", err)
	}
}
