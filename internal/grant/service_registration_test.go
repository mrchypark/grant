package grant

import (
	"reflect"
	"testing"
)

func TestProviderRegistrationReadsAndApplicationLifecycle(t *testing.T) {
	ctx, s := serviceFixture(t)
	p := ServiceProvider{ID: "registry", Name: "Registry", Active: true}
	if _, err := s.UpsertServiceProviderAdmin(ctx, p); err != nil {
		t.Fatal(err)
	}
	if got, err := s.ListProviderActionsAdmin(ctx, p.ID); err != nil || got == nil || len(got) != 0 {
		t.Fatal("empty registry", got, err)
	}
	if _, err := s.ListProviderActionsAdmin(ctx, "missing"); ErrorStatus(err) != 404 {
		t.Fatal(err)
	}
	actions := []Action{{ID: "registry.z", ProviderID: p.ID, Code: "z", Name: "Internal", RiskLevel: "low"}, {ID: "registry.a", ProviderID: p.ID, Code: "a", Name: "Candidate", RiskLevel: "high", GrantableToApplications: true}}
	if _, err := s.ReplaceProviderActionsAdmin(ctx, p.ID, actions); err != nil {
		t.Fatal(err)
	}
	if got, err := s.ListProviderActionsAdmin(ctx, p.ID); err != nil || len(got) != 2 || got[0].Code != "a" {
		t.Fatal(got, err)
	}
	if got, err := s.ListProviderActionsAuthorized(ctx, "owner", "owner", p.ID); err != nil || len(got) != 1 || got[0].ID != "registry.a" {
		t.Fatal(got, err)
	}
	if _, err := s.ListProviderActionsAuthorized(ctx, "other", "owner", p.ID); ErrorStatus(err) != 403 {
		t.Fatal("unprivileged candidate read", err)
	}
	if _, err := s.ListProviderActionsAuthorized(ctx, "owner", "other", p.ID); ErrorStatus(err) != 403 {
		t.Fatal("cross org candidate read", err)
	}
	if _, err := s.CreateApplication(ctx, "owner", "owner", CreateApplicationInput{ID: "registry-app", Slug: "registry-app", Name: "App", ApplicationType: "confidential", AppType: "server", Active: true}); err != nil {
		t.Fatal(err)
	}
	if _, err := s.ReplaceApplicationActionsAuthorized(ctx, "owner", "owner", "registry-app", []string{"registry.a"}); err != nil {
		t.Fatal(err)
	}
	if _, err := s.ReplaceApplicationConsent(ctx, "owner", "owner", "registry-app", []string{"registry.a"}); err != nil {
		t.Fatal(err)
	}
	if _, err := s.store.Exec(ctx, "INSERT INTO role_actions(role_id,action_id) VALUES('role-system-bootstrap-owner','registry.a')"); err != nil {
		t.Fatal(err)
	}
	actions[1].GrantableToApplications = false
	if _, err := s.ReplaceProviderActionsAdmin(ctx, p.ID, actions); err != nil {
		t.Fatal(err)
	}
	rows, err := s.store.Query(ctx, ReadStrong, `SELECT (SELECT COUNT(*) FROM application_actions WHERE action_id='registry.a') AS admitted,(SELECT COUNT(*) FROM application_grant_actions WHERE action_id='registry.a') AS consent,(SELECT COUNT(*) FROM role_actions WHERE action_id='registry.a') AS roles`)
	if err != nil || rowInt(rows[0], "admitted") != 0 || rowInt(rows[0], "consent") != 0 || rowInt(rows[0], "roles") != 1 {
		t.Fatal("ungrantable cleanup", rows, err)
	}
	actions[1].GrantableToApplications = true
	if _, err = s.ReplaceProviderActionsAdmin(ctx, p.ID, actions); err != nil {
		t.Fatal(err)
	}
	if _, err = s.ReplaceApplicationActionsAuthorized(ctx, "owner", "owner", "registry-app", []string{"registry.a"}); err != nil {
		t.Fatal(err)
	}
	p.Active = false
	if _, err = s.UpsertServiceProviderAdmin(ctx, p); err != nil {
		t.Fatal(err)
	}
	if _, err = s.ListProviderActionsAuthorized(ctx, "owner", "owner", p.ID); ErrorStatus(err) != 404 {
		t.Fatal("inactive candidate", err)
	}
	if got, err := s.ListProviderActionsAdmin(ctx, p.ID); err != nil || len(got) != 2 {
		t.Fatal("inactive admin", got, err)
	}
	if _, err = s.ReplaceApplicationActionsAuthorized(ctx, "owner", "owner", "registry-app", []string{"registry.a"}); ErrorStatus(err) != 400 {
		t.Fatal("inactive admission", err)
	}
	// The store must guard this too: service preflight alone races provider deactivation.
	if _, err = s.store.ReplaceApplicationActions(ctx, "registry-app", []string{"registry.a"}); err == nil {
		t.Fatal("inactive provider bypassed transactional guard")
	}
	got, err := s.store.ListApplicationActionsWithConsistency(ctx, "registry-app", ReadStrong)
	if err != nil || len(got) != 1 {
		t.Fatal("rejected replacement lost old selection", got, err)
	}
}

func TestActiveRegistryReferencesAndAdminReadback(t *testing.T) {
	ctx, s := serviceFixture(t)
	if _, err := s.ActiveHttpCapabilityCatalogAdmin(ctx, "missing"); ErrorStatus(err) != 404 {
		t.Fatal(err)
	}
	if _, err := s.UpsertServiceProviderAdmin(ctx, ServiceProvider{ID: "patch-data", Name: "Patch", Active: true}); err != nil {
		t.Fatal(err)
	}
	if _, err := s.ActiveHttpCapabilityCatalogAdmin(ctx, "patch-data"); ErrorStatus(err) != 404 {
		t.Fatal(err)
	}
	stage, actions, err := PatchDataCatalog(1, nil)
	if err != nil {
		t.Fatal(err)
	}
	responseAction := Action{ID: "patch-data.response-only.read", ProviderID: "patch-data", Code: "response-only.read", Name: "Response only", RiskLevel: "high", GrantableToApplications: true}
	actions = append(actions, responseAction)
	node := stage.ResponseResources[0]
	node.Key = "zz-response-only"
	node.ActionID = responseAction.ID
	node.Relations = nil
	stage.ResponseResources = append(stage.ResponseResources, node)
	stage.ResponseResources[0].Relations = append(stage.ResponseResources[0].Relations, HttpResponseRelation{Field: "zzchild", TargetKey: node.Key})
	stage.CatalogDigest, err = HttpCapabilityCatalogDigest("patch-data", stage)
	if err != nil {
		t.Fatal(err)
	}
	if _, err = s.ReplaceProviderActionsAdmin(ctx, "patch-data", actions); err != nil {
		t.Fatal(err)
	}
	app := Application{ID: "registry-pep", OrganizationID: "owner", Slug: "registry-pep", Name: "PEP", ApplicationType: "confidential", AppType: "server", FirstParty: true, ConsentPolicy: "auto_approve", Active: true, ProvisioningStatus: "ready", CreatedByUserID: "owner"}
	if err = s.store.InsertApplicationWithRedirectURI(ctx, app, nil); err != nil {
		t.Fatal(err)
	}
	if _, err = s.SetProviderPepClientAdmin(ctx, "patch-data", "registry-pep", UpsertProviderPepClient{Lifecycle: "active"}); err != nil {
		t.Fatal(err)
	}
	if _, err = s.StageHttpCapabilityGenerationAdmin(ctx, "patch-data", stage); err != nil {
		t.Fatal(err)
	}
	// Staging is not activation: removing a response-only action is still allowed.
	if _, err = s.ReplaceProviderActionsAdmin(ctx, "patch-data", actions[:len(actions)-1]); err != nil {
		t.Fatal(err)
	}
	activate := ActivateHttpCapabilityGeneration{TargetGeneration: 1, TargetCatalogDigest: stage.CatalogDigest}
	if _, err = s.ActivateHttpCapabilityGenerationAdmin(ctx, "patch-data", activate); ErrorStatus(err) != 409 {
		t.Fatal("missing response action activated", err)
	}
	rows, err := s.store.Query(ctx, ReadStrong, "SELECT COUNT(*) AS n FROM provider_http_capability_activations")
	if err != nil || rowInt(rows[0], "n") != 0 {
		t.Fatal("rejected activation wrote receipt", rows, err)
	}
	if _, err = s.ReplaceProviderActionsAdmin(ctx, "patch-data", actions); err != nil {
		t.Fatal(err)
	}
	if _, err = s.ActivateHttpCapabilityGenerationAdmin(ctx, "patch-data", activate); err != nil {
		t.Fatal(err)
	}
	before, err := s.ActiveHttpCapabilityCatalogAdmin(ctx, "patch-data")
	if err != nil || before.CapabilityCount != 369 || before.ResponseResourceCount != len(stage.ResponseResources) {
		t.Fatal(before, err)
	}
	for _, kept := range [][]Action{actions[:len(actions)-1], actions[1:]} {
		if _, err = s.ReplaceProviderActionsAdmin(ctx, "patch-data", kept); ErrorStatus(err) != 409 {
			t.Fatal("active reference removed", err)
		}
	}
	got, err := s.ListProviderActionsAdmin(ctx, "patch-data")
	if err != nil || len(got) != len(actions) {
		t.Fatal("rejected registry changed", got, err)
	}
	if _, err = s.SetProviderPepClientAdmin(ctx, "patch-data", "registry-pep", UpsertProviderPepClient{Lifecycle: "disabled"}); err != nil {
		t.Fatal(err)
	}
	if _, err = s.UpsertServiceProviderAdmin(ctx, ServiceProvider{ID: "patch-data", Name: "Patch", Active: false}); err != nil {
		t.Fatal(err)
	}
	after, err := s.ActiveHttpCapabilityCatalogAdmin(ctx, "patch-data")
	if err != nil || !reflect.DeepEqual(before, after) {
		t.Fatal("admin read depends on provider/PEP serving state", err)
	}
}
