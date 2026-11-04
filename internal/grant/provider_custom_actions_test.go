package grant

import (
	"strings"
	"testing"
)

func TestProviderCustomActionOwnership(t *testing.T) {
	ctx, s, operable := eligibilityFixture(t)
	const provider = "independent-authz"
	if _, err := s.UpsertServiceProviderAdmin(ctx, ServiceProvider{ID: provider, Name: provider, Active: true}); err != nil {
		t.Fatal(err)
	}
	input := CustomActionInput{Code: operable.Code, Name: "Independent", RiskLevel: "low"}
	a, err := s.CreateCustomActionAuthorized(ctx, "owner", "owner", provider, input)
	if err != nil || a.ID != provider+"."+operable.Code || a.ID == operable.ID || a.GrantableToApplications || a.VisibilityGranting {
		t.Fatalf("provider identity/defaults: %+v %v", a, err)
	}
	ownershipMember(t, ctx, s, "next-owner")
	if _, err := s.CreateOrganizationAdmin(ctx, CreateOrganizationInput{ID: "other-org", Name: "Other", OwnerUserID: "next-owner"}); err != nil {
		t.Fatal(err)
	}
	patch := CustomActionPatch{Name: stringPtr("Forbidden")}
	if _, err := s.CreateProviderAction(ctx, provider, Action{ID: provider + ".read", ProviderID: provider, Code: "read", Name: "Built-in", RiskLevel: "low"}); err != nil {
		t.Fatal(err)
	}
	for _, target := range []struct{ provider, id string }{
		{"operable", a.ID}, {provider, operable.ID}, {provider, provider + ".read"}, {"grant", "organization.profile.read"},
	} {
		if _, err := s.PatchCustomActionAuthorized(ctx, "owner", "owner", target.provider, target.id, patch); ErrorStatus(err) != 409 {
			t.Fatalf("cross-provider/built-in patch %+v: %v", target, err)
		}
		if err := s.DeleteCustomActionAuthorized(ctx, "owner", "owner", target.provider, target.id); ErrorStatus(err) != 409 {
			t.Fatalf("cross-provider/built-in delete %+v: %v", target, err)
		}
	}
	if _, err := s.PatchCustomActionAuthorized(ctx, "next-owner", "other-org", provider, a.ID, patch); ErrorStatus(err) != 409 {
		t.Fatalf("foreign owner patch: %v", err)
	}
	if err := s.DeleteCustomActionAuthorized(ctx, "next-owner", "other-org", provider, a.ID); ErrorStatus(err) != 409 {
		t.Fatalf("foreign owner delete: %v", err)
	}
	for _, code := range []string{"organization.profile.read", "custom.shared", organizationCustomNamespace("other-org") + "calendar.event.create"} {
		input.Code = code
		if _, err := s.CreateCustomActionAuthorized(ctx, "owner", "owner", provider, input); ErrorStatus(err) != 400 {
			t.Fatalf("unowned code %q: %v", code, err)
		}
	}
	input.Code = a.Code
	if _, err := s.CreateCustomActionAuthorized(ctx, "owner", "owner", "missing-provider", input); ErrorStatus(err) != 404 {
		t.Fatalf("missing provider: %v", err)
	}
	// A code in the reserved namespace cannot give an unrelated built-in ID ownership.
	lifecycleExec(t, ctx, s, "UPDATE actions SET id=? WHERE id=?", provider+".builtin", a.ID)
	if _, err := s.PatchCustomActionAuthorized(ctx, "owner", "owner", provider, provider+".builtin", patch); ErrorStatus(err) != 409 {
		t.Fatalf("noncanonical ID patch: %v", err)
	}
	if err := s.DeleteCustomActionAuthorized(ctx, "owner", "owner", provider, provider+".builtin"); ErrorStatus(err) != 409 {
		t.Fatalf("noncanonical ID delete: %v", err)
	}
}

func TestSecondProviderCustomAdmissionAndSharedCompatibility(t *testing.T) {
	ctx, s := serviceFixture(t)
	const provider = "independent-authz"
	if _, err := s.UpsertServiceProviderAdmin(ctx, ServiceProvider{ID: provider, Name: provider, Active: true}); err != nil {
		t.Fatal(err)
	}
	ownCode := organizationCustomNamespace("owner") + "calendar.event.create"
	foreignCode := organizationCustomNamespace("foreign") + "calendar.event.create"
	for _, code := range []string{ownCode, foreignCode, "read", "custom.shared"} {
		if _, err := s.CreateProviderAction(ctx, provider, Action{ID: provider + "." + code, ProviderID: provider, Code: code, Name: code, RiskLevel: "low", GrantableToApplications: true}); err != nil {
			t.Fatal(err)
		}
	}
	candidates, err := s.ListProviderActionsAuthorized(ctx, "owner", "owner", provider)
	if err != nil || len(candidates) != 3 {
		t.Fatalf("shared/own candidates: %+v %v", candidates, err)
	}
	snapshot, err := s.ProviderActionsSnapshotAuthorized(ctx, "owner", "owner", provider)
	if err != nil || strings.Contains(string(snapshot.Value), foreignCode) {
		t.Fatalf("foreign metadata leak: %s %v", snapshot.Value, err)
	}
	admin, err := s.ListProviderActionsAdmin(ctx, provider)
	if err != nil || len(admin) != 4 {
		t.Fatalf("admin catalog changed: %+v %v", admin, err)
	}
	if err := s.store.InsertApplicationWithRedirectURI(ctx, Application{ID: "second-app", OrganizationID: "owner", Slug: "second-app", Name: "Second", ApplicationType: "confidential", AppType: "internal", ConsentPolicy: "explicit", Active: true, ProvisioningStatus: "ready", CreatedByUserID: "owner"}, nil); err != nil {
		t.Fatal(err)
	}
	want := []string{provider + "." + ownCode, provider + ".read", provider + ".custom.shared"}
	if _, err := s.ReplaceApplicationActionsAuthorized(ctx, "owner", "owner", "second-app", want); err != nil {
		t.Fatal(err)
	}
	foreignID := provider + "." + foreignCode
	if err := s.SetApplicationAction(ctx, "owner", "owner", "second-app", foreignID, true); ErrorStatus(err) != 409 {
		t.Fatalf("single foreign admission: %v", err)
	}
	if _, err := s.ReplaceApplicationActionsAuthorized(ctx, "owner", "owner", "second-app", []string{foreignID}); ErrorStatus(err) != 400 {
		t.Fatalf("foreign replacement: %v", err)
	}
	target, err := serviceRow[applicationActionTarget](ctx, s, ReadStrong, "SELECT id,organization_id,created_at FROM applications WHERE id='second-app'")
	if err != nil {
		t.Fatal(err)
	}
	if _, err := s.store.replaceApplicationActions(ctx, target, "owner", []string{foreignID}); err == nil {
		t.Fatal("transaction accepted foreign namespace")
	}
	if got := lifecycleCount(t, ctx, s, "SELECT * FROM application_actions WHERE application_id='second-app'"); got != len(want) {
		t.Fatalf("failed replacement lost existing admissions: %d", got)
	}
	no := false
	if _, err := s.PatchCustomActionConditional(ctx, "owner", "owner", provider, want[0], CustomActionPatch{GrantableToApplications: &no}, snapshot.ETag); ErrorStatus(err) != 409 {
		t.Fatalf("disabled admitted action: %v", err)
	}
	if err := s.SetApplicationAction(ctx, "owner", "owner", "second-app", want[0], false); err != nil {
		t.Fatal(err)
	}
	if _, err := s.PatchCustomActionConditional(ctx, "owner", "owner", provider, want[0], CustomActionPatch{GrantableToApplications: &no}, snapshot.ETag); err != nil {
		t.Fatal(err)
	}
	if err := s.DeleteCustomActionAuthorized(ctx, "owner", "owner", provider, want[0]); err != nil {
		t.Fatal(err)
	}
}

func TestSecondProviderCustomAuthorityAndProviderGuard(t *testing.T) {
	ctx, s := serviceFixture(t)
	const provider = "independent-authz"
	if _, err := s.UpsertServiceProviderAdmin(ctx, ServiceProvider{ID: provider, Name: provider, Active: true}); err != nil {
		t.Fatal(err)
	}
	input := CustomActionInput{Code: organizationCustomNamespace("owner") + "calendar.event.create", Name: "Original", RiskLevel: "low"}
	a, err := s.CreateCustomActionAuthorized(ctx, "owner", "owner", provider, input)
	if err != nil {
		t.Fatal(err)
	}
	yes := true
	snapshot, err := s.ProviderActionsSnapshotAuthorized(ctx, "owner", "owner", provider)
	if err != nil {
		t.Fatal(err)
	}
	ownershipMember(t, ctx, s, "delegate")
	if _, err := s.CreateCustomActionAuthorized(ctx, "delegate", "owner", provider, input); ErrorStatus(err) != 403 {
		t.Fatalf("nonowner create: %v", err)
	}
	if _, err := s.PatchCustomActionConditional(ctx, "delegate", "owner", provider, a.ID, CustomActionPatch{GrantableToApplications: &yes}, snapshot.ETag); ErrorStatus(err) != 403 {
		t.Fatalf("nonowner eligibility: %v", err)
	}
	if err := s.DeleteCustomActionAuthorized(ctx, "delegate", "owner", provider, a.ID); ErrorStatus(err) != 403 {
		t.Fatalf("nonowner delete: %v", err)
	}
	guard, err := s.ownerActionMutationGuard(ctx, "owner", "owner", provider)
	if err != nil {
		t.Fatal(err)
	}
	lifecycleExec(t, ctx, s, "UPDATE service_providers SET active=0 WHERE id=?", provider)
	guard = append(guard, Statement{SQL: "UPDATE actions SET name='Must not persist' WHERE id=?", Args: []any{a.ID}})
	if err := lifecycleError(s.store.Transaction(ctx, guard)); ErrorStatus(err) != 409 {
		t.Fatalf("transaction accepted deactivated provider: %v", err)
	}
	if _, err := s.CreateCustomActionAuthorized(ctx, "owner", "owner", provider, input); ErrorStatus(err) != 404 {
		t.Fatalf("inactive provider create: %v", err)
	}
	if _, err := s.PatchCustomActionConditional(ctx, "owner", "owner", provider, a.ID, CustomActionPatch{GrantableToApplications: &yes}, snapshot.ETag); ErrorStatus(err) != 404 {
		t.Fatalf("inactive provider patch: %v", err)
	}
	if err := s.DeleteCustomActionAuthorized(ctx, "owner", "owner", provider, a.ID); ErrorStatus(err) != 404 {
		t.Fatalf("inactive provider delete: %v", err)
	}
	if _, err := s.UpsertServiceProviderAdmin(ctx, ServiceProvider{ID: provider, Name: provider, Active: true}); err != nil {
		t.Fatal(err)
	}
	lifecycleExec(t, ctx, s, "DELETE FROM role_actions WHERE action_id='organization.permission.manage'")
	if _, err := s.PatchCustomActionConditional(ctx, "owner", "owner", provider, a.ID, CustomActionPatch{GrantableToApplications: &yes}, snapshot.ETag); ErrorStatus(err) != 403 {
		t.Fatalf("owner without permission: %v", err)
	}
	current, err := serviceRow[Action](ctx, s, ReadStrong, "SELECT * FROM actions WHERE id=?", a.ID)
	if err != nil || current.Name != "Original" || current.GrantableToApplications {
		t.Fatalf("partial mutation: %+v %v", current, err)
	}
}
