package grant

import (
	"encoding/json"
	"fmt"
	"strings"
	"testing"
)

func TestApplicationAndConsentTransactionCAS(t *testing.T) {
	ctx, s := serviceFixture(t)
	s.cfg.Auth.ProviderMode = "pocketbase"
	_, err := s.CreateApplication(ctx, "owner", "owner", CreateApplicationInput{ID: "cas-app", Slug: "cas-app", Name: "CAS", ApplicationType: "confidential", AppType: "internal", Active: true})
	if err != nil {
		t.Fatal(err)
	}
	a, b := "organization.profile.read", "test.extra"
	if _, err = s.CreateProviderAction(ctx, "grant", Action{ID: b, ProviderID: "grant", Code: b, Name: "Extra", RiskLevel: "low", GrantableToApplications: true}); err != nil {
		t.Fatal(err)
	}
	snap, err := s.ApplicationActionsSnapshot(ctx, "owner", "owner", "cas-app")
	if err != nil {
		t.Fatal(err)
	}
	guarded, err := checkedActionContext(ctx, snap, snap.ETag, "")
	if err != nil {
		t.Fatal(err)
	}
	if err = s.SetApplicationAction(ctx, "owner", "owner", "cas-app", a, true); err != nil {
		t.Fatal(err)
	}
	if _, err = s.ReplaceApplicationActionsAuthorized(guarded, "owner", "owner", "cas-app", nil); ErrorStatus(err) != 412 {
		t.Fatalf("application race: %v", err)
	}
	if err = s.SetApplicationAction(ctx, "owner", "owner", "cas-app", b, true); err != nil {
		t.Fatal(err)
	}
	if _, err = s.ReplaceConsentConditional(ctx, "owner", "owner", "cas-app", []string{a}, "", ""); ErrorStatus(err) != 428 {
		t.Fatalf("missing condition: %v", err)
	}
	if _, err = s.ReplaceConsentConditional(ctx, "owner", "owner", "cas-app", []string{a}, "", "*"); err != nil {
		t.Fatal(err)
	}
	if _, err = s.ReplaceConsentConditional(ctx, "owner", "owner", "cas-app", []string{a}, "", "*"); ErrorStatus(err) != 412 {
		t.Fatalf("duplicate creation: %v", err)
	}
	snap, err = s.ConsentSnapshot(ctx, "owner", "owner", "cas-app")
	if err != nil {
		t.Fatal(err)
	}
	guarded, err = checkedActionContext(ctx, snap, snap.ETag, "")
	if err != nil {
		t.Fatal(err)
	}
	if _, err = s.ReplaceConsentConditional(ctx, "owner", "owner", "cas-app", []string{a, b}, snap.ETag, ""); err != nil {
		t.Fatal(err)
	}
	if _, err = s.ReplaceApplicationConsent(guarded, "owner", "owner", "cas-app", []string{a}); ErrorStatus(err) != 412 {
		t.Fatalf("consent race: %v", err)
	}
	if err = s.SetApplicationAction(ctx, "owner", "owner", "cas-app", a, false); err != nil {
		t.Fatal(err)
	}
	consent, err := s.GetApplicationConsent(ctx, "owner", "owner", "cas-app")
	if err != nil || len(consent.Actions) != 1 || consent.Actions[0].ID != b {
		t.Fatalf("consent cleanup: %+v %v", consent, err)
	}
	if _, err = s.CreateRoleAuthorized(ctx, "owner", "owner", CreateRoleInput{ID: "kept-role", Code: "kept", Name: "Kept"}); err != nil {
		t.Fatal(err)
	}
	if err = s.SetRoleAction(ctx, "owner", "owner", "kept-role", b, true); err != nil {
		t.Fatal(err)
	}
	if _, err = s.PatchProviderAction(ctx, "grant", b, map[string]json.RawMessage{"grantableToApplications": json.RawMessage("false")}); err != nil {
		t.Fatal(err)
	}
	consent, err = s.GetApplicationConsent(ctx, "owner", "owner", "cas-app")
	if err != nil || len(consent.Actions) != 0 {
		t.Fatalf("grantability cleanup: %+v %v", consent, err)
	}
	role, err := s.RoleActionsSnapshot(ctx, "owner", "owner", "kept-role")
	if err != nil || string(role.Value) == "[]" {
		t.Fatal("grantability removed role link", err)
	}
	if _, err = s.SetApplicationTrustAdmin(ctx, "owner", "cas-app", true); err != nil {
		t.Fatal(err)
	}
	if err = s.SetApplicationAction(ctx, "owner", "owner", "cas-app", a, true); ErrorStatus(err) != 403 {
		t.Fatalf("trusted mutation: %v", err)
	}
}

func TestLargeProviderSnapshotCAS(t *testing.T) {
	ctx, s := serviceFixture(t)
	if _, err := s.UpsertServiceProviderAdmin(ctx, ServiceProvider{ID: "large", Name: "Large", Active: true}); err != nil {
		t.Fatal(err)
	}
	actions := make([]Action, 250)
	for i := range actions {
		id := fmt.Sprintf("a%d", i)
		actions[i] = Action{ID: id, ProviderID: "large", Code: id, Name: strings.Repeat("가", 90), RiskLevel: "low", GrantableToApplications: true}
	}
	if _, err := s.ReplaceProviderActionsAdmin(ctx, "large", actions); err != nil {
		t.Fatal(err)
	}
	snap, err := s.ProviderActionsSnapshot(ctx, "large")
	if err != nil {
		t.Fatal(err)
	}
	if len(snap.state) < 65536 {
		t.Fatal("fixture does not exercise scalar limit")
	}
	var got []Action
	if err = json.Unmarshal(snap.Value, &got); err != nil || len(got) != 250 {
		t.Fatalf("snapshot: %d %v", len(got), err)
	}
	if _, err = s.ReplaceProviderActionsConditional(ctx, "large", actions, snap.ETag); err != nil {
		t.Fatal(err)
	}
	if _, err = s.ReplaceProviderActionsConditional(ctx, "large", nil, snap.ETag); ErrorStatus(err) != 412 {
		t.Fatalf("stale full set: %v", err)
	}
}
