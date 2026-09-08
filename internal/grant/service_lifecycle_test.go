package grant

import "testing"

func TestApplicationConsentLifecycleAndRejectedReplacement(t *testing.T) {
	ctx, s := serviceFixture(t)
	uri := "https://client.example/callback"
	app, err := s.CreateApplication(ctx, "owner", "owner", CreateApplicationInput{ID: "client", Slug: "client", Name: "Client", ApplicationType: "confidential", AppType: "web", Active: true, RedirectURI: &uri})
	if err != nil {
		t.Fatal(err)
	}
	if app.Application.ProvisioningStatus != "unprovisioned" || app.Application.FirstParty || app.Application.ConsentPolicy != "explicit" {
		t.Fatal(app)
	}
	if _, err = s.GetApplicationDetailAuthorized(ctx, "owner", "another", "client", ReadStrong); ErrorStatus(err) != 403 {
		t.Fatal("cross organization application read", err)
	}
	if _, err = s.ReplaceApplicationActionsAuthorized(ctx, "owner", "owner", "client", []string{"organization.profile.read"}); err != nil {
		t.Fatal(err)
	}
	consent, err := s.ReplaceApplicationConsent(ctx, "owner", "owner", "client", []string{"organization.profile.read"})
	if err != nil {
		t.Fatal(err)
	}
	if consent.Grant.Status != "active" || len(consent.Actions) != 1 {
		t.Fatal("invalid active consent", consent)
	}
	if _, err = s.ReplaceApplicationConsent(ctx, "owner", "owner", "client", []string{"organization.role.manage"}); err == nil {
		t.Fatal("non application action consent accepted")
	}
	rows, err := s.store.Query(ctx, ReadStrong, "SELECT action_id FROM application_grant_actions")
	if err != nil || len(rows) != 1 || rowString(rows[0], "action_id") != "organization.profile.read" {
		t.Fatal("rejected consent destroyed previous grant", rows, err)
	}
	if err = s.RevokeApplicationConsent(ctx, "owner", "owner", "client"); err != nil {
		t.Fatal(err)
	}
	if got, err := s.GetApplicationConsent(ctx, "owner", "owner", "client"); err != nil || got != nil {
		t.Fatal("revoked consent remains active", got, err)
	}
	if _, err = s.ReplaceApplicationActionsAuthorized(ctx, "owner", "owner", "client", []string{"organization.role.manage"}); ErrorStatus(err) != 400 {
		t.Fatal("invalid application action status", err)
	}
	if _, err = s.SetApplicationTrustAdmin(ctx, "owner", "absent", true); ErrorStatus(err) != 404 {
		t.Fatal("missing trust target status", err)
	}
	if _, err = s.SetApplicationTrustAdmin(ctx, "owner", "client", true); err != nil {
		t.Fatal(err)
	}
	if _, err = s.ReplaceApplicationActionsAuthorized(ctx, "owner", "owner", "client", nil); ErrorStatus(err) != 403 {
		t.Fatal("trusted mutation status", err)
	}
	if _, err = s.store.Exec(ctx, "UPDATE applications SET security_mutation_token='held',security_mutation_expires_at='9999-01-01T00:00:00Z' WHERE id='client'"); err != nil {
		t.Fatal(err)
	}
	if _, err = s.SetApplicationTrustAdmin(ctx, "owner", "client", false); ErrorStatus(err) != 409 {
		t.Fatal("leased trust mutation status", err)
	}
}

func TestAgreementEndpointImmutabilityAndOrganizationKillSwitch(t *testing.T) {
	ctx, s := serviceFixture(t)
	if _, err := s.store.Exec(ctx, "INSERT INTO organizations(id,name,created_at,updated_at) VALUES('receiver','Receiver','now','now')"); err != nil {
		t.Fatal(err)
	}
	in := UpsertAgreementInput{ID: "agreement", AgreementType: "delegation", GrantorOrganizationID: "owner", GranteeOrganizationID: "receiver", Status: "active", Title: "Delegation"}
	if _, err := s.CreateAgreementAuthorized(ctx, "owner", "owner", in); err != nil {
		t.Fatal(err)
	}
	in.GranteeOrganizationID = "owner"
	if _, err := s.UpdateAgreementAuthorized(ctx, "owner", "owner", "agreement", in); ErrorStatus(err) != 400 {
		t.Fatal("agreement endpoint changed", err)
	}
	got, err := s.GetAgreementAuthorized(ctx, "owner", "owner", "agreement", ReadStrong)
	if err != nil || got.GranteeOrganizationID != "receiver" {
		t.Fatal(got, err)
	}
	if _, err = s.DeactivateOrganization(ctx, "owner", "owner", "owner"); err != nil {
		t.Fatal(err)
	}
	if _, err = s.CreateGroupAuthorized(ctx, "owner", "owner", UpsertGroupInput{ID: "blocked", Name: "Blocked"}); ErrorStatus(err) != 403 {
		t.Fatal("inactive organization accepted mutation", err)
	}
	if _, err = s.ReactivateOrganization(ctx, "owner", "owner", "owner"); err != nil {
		t.Fatal(err)
	}
	if _, err = s.CreateGroupAuthorized(ctx, "owner", "owner", UpsertGroupInput{ID: "resumed", Name: "Resumed"}); err != nil {
		t.Fatal(err)
	}
}
