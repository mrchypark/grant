package grant

import (
	"encoding/json"
	"testing"
)

func TestProviderCreateDoesNotOverwriteAndPatchPreservesOmittedFields(t *testing.T) {
	ctx, s := serviceFixture(t)
	description := "original"
	if _, err := s.CreateServiceProviderAdmin(ctx, ServiceProvider{ID: "contract", Name: "Original", Description: &description, Active: true}); err != nil {
		t.Fatal(err)
	}
	if _, err := s.CreateServiceProviderAdmin(ctx, ServiceProvider{ID: "contract", Name: "Overwrite"}); ErrorStatus(err) != 409 {
		t.Fatalf("duplicate = %v", err)
	}
	var in PatchServiceProviderInput
	if err := json.Unmarshal([]byte(`{"name":"Renamed"}`), &in); err != nil {
		t.Fatal(err)
	}
	p, err := s.PatchServiceProviderAdmin(ctx, "contract", in)
	if err != nil {
		t.Fatal(err)
	}
	if p.Name != "Renamed" || !p.Active || p.Description == nil || *p.Description != description {
		t.Fatalf("patch overwrote omitted fields: %+v", p)
	}
	in = PatchServiceProviderInput{}
	if err = json.Unmarshal([]byte(`{"description":null,"active":false}`), &in); err != nil {
		t.Fatal(err)
	}
	p, err = s.PatchServiceProviderAdmin(ctx, "contract", in)
	if err != nil || p.Description != nil || p.Active || p.Name != "Renamed" {
		t.Fatalf("explicit clear/false: %+v %v", p, err)
	}
}
func TestOrganizationCreationScopesAndNoDuplicateOwnerMutation(t *testing.T) {
	ctx, s := serviceFixture(t)
	if _, err := s.CreateOrganizationAdmin(ctx, CreateOrganizationInput{ID: "new", Name: "New", OwnerUserID: "missing"}); ErrorStatus(err) != 400 {
		t.Fatal(err)
	}
	if _, err := s.CreateOrganizationAdmin(ctx, CreateOrganizationInput{ID: "new", Name: "New", OwnerUserID: "owner"}); err != nil {
		t.Fatal(err)
	}
	if _, err := s.CreateOrganizationAdmin(ctx, CreateOrganizationInput{ID: "new", Name: "Overwrite", OwnerUserID: "owner"}); ErrorStatus(err) != 409 {
		t.Fatal(err)
	}
	org, err := s.GetOrganizationAuthorized(ctx, "owner", "new", ReadStrong)
	if err != nil || org.Name != "New" {
		t.Fatal(org, err)
	}
	name := "Renamed"
	if _, err = s.PatchOrganizationAuthorized(ctx, "owner", "new", PatchOrganizationInput{Name: &name}); err != nil {
		t.Fatal(err)
	}
	if _, err = s.PatchOrganizationAuthorized(ctx, "outsider", "new", PatchOrganizationInput{Name: &name}); ErrorStatus(err) != 403 {
		t.Fatal("cross-org patch", err)
	}
	orgs, err := s.ListOrganizationsAuthorized(ctx, "owner", ReadStrong)
	if err != nil || len(orgs) != 2 {
		t.Fatal(orgs, err)
	}
	orgs, err = s.ListOrganizationsAuthorized(ctx, "outsider", ReadStrong)
	if err != nil || len(orgs) != 0 {
		t.Fatal(orgs, err)
	}
}
func TestGroupAndAgreementPatchPreserveFieldsAndScope(t *testing.T) {
	ctx, s := serviceFixture(t)
	description := "keep"
	name := "renamed"
	if _, err := s.CreateGroupAuthorized(ctx, "owner", "owner", UpsertGroupInput{ID: "patchgroup", Name: "Group", Description: &description}); err != nil {
		t.Fatal(err)
	}
	g, err := s.PatchGroupAuthorized(ctx, "owner", "owner", "patchgroup", PatchGroupInput{Name: &name})
	if err != nil || g.Description == nil || *g.Description != description {
		t.Fatal(g, err)
	}
	if _, err = s.PatchGroupAuthorized(ctx, "owner", "other", "patchgroup", PatchGroupInput{Name: &name}); ErrorStatus(err) != 403 {
		t.Fatal(err)
	}
	if _, err = s.CreateOrganizationAdmin(ctx, CreateOrganizationInput{ID: "receiver", Name: "Receiver", OwnerUserID: "owner"}); err != nil {
		t.Fatal(err)
	}
	end := "2030-01-01T00:00:00.000000000Z"
	if _, err = s.CreateAgreementAuthorized(ctx, "owner", "owner", UpsertAgreementInput{ID: "patchagreement", Title: "Original", AgreementType: "delegation", Status: "active", GrantorOrganizationID: "owner", GranteeOrganizationID: "receiver", EndsAt: &end}); err != nil {
		t.Fatal(err)
	}
	a, err := s.PatchAgreementAuthorized(ctx, "owner", "receiver", "patchagreement", PatchAgreementInput{Title: &name})
	if err != nil || a.Status != "active" || a.EndsAt == nil || *a.EndsAt != end {
		t.Fatal(a, err)
	}
	if _, err = s.PatchAgreementAuthorized(ctx, "owner", "receiver", "patchagreement", PatchAgreementInput{EndsAt: PatchText{Present: true}}); ErrorStatus(err) != 403 {
		t.Fatal("grantee changed terms", err)
	}
	if _, err = s.PatchAgreementAuthorized(ctx, "owner", "other", "patchagreement", PatchAgreementInput{Title: &name}); ErrorStatus(err) != 403 {
		t.Fatal("cross organization", err)
	}
}

func TestContractPatchRejectsNullRequiredAndImmutableFields(t *testing.T) {
	for _, input := range []string{`{"name":null,"active":false}`, `{"name":"valid","active":null}`, `{"id":"change","name":"valid"}`, `{"Name":null,"description":"x"}`, `{"name":"a","Name":"b"}`} {
		var p PatchServiceProviderInput
		if err := json.Unmarshal([]byte(input), &p); err == nil {
			t.Fatalf("accepted %s", input)
		}
	}
	for _, input := range []string{`{"title":null,"referenceNo":"valid"}`, `{"status":null,"title":"valid"}`, `{"agreementType":null,"title":"valid"}`, `{"grantorOrganizationId":"other","title":"valid"}`} {
		var p PatchAgreementInput
		if err := json.Unmarshal([]byte(input), &p); err == nil {
			t.Fatalf("accepted %s", input)
		}
	}
}

func TestAgreementIntervalsValidatedAcrossCreatePutAndPatch(t *testing.T) {
	ctx, s := serviceFixture(t)
	if _, err := s.CreateOrganizationAdmin(ctx, CreateOrganizationInput{ID: "receiver", Name: "Receiver", OwnerUserID: "owner"}); err != nil {
		t.Fatal(err)
	}
	start, end := "2030-01-01T09:00:00+09:00", "2030-01-01T19:00:00+09:00"
	input := UpsertAgreementInput{ID: "interval", Title: "Interval", AgreementType: "delegation", Status: "active", GrantorOrganizationID: "owner", GranteeOrganizationID: "receiver", StartsAt: &start, EndsAt: &end}
	a, err := s.CreateAgreementAuthorized(ctx, "owner", "owner", input)
	if err != nil {
		t.Fatal(err)
	}
	if ptrValue(a.StartsAt) != "2030-01-01T00:00:00.000000000Z" || ptrValue(a.EndsAt) != "2030-01-01T10:00:00.000000000Z" {
		t.Fatal("not UTC", a)
	}
	for _, bad := range []string{"never", "", "2030-01-01", "2029-01-01T00:00:00.000000000Z"} {
		t.Run(bad, func(t *testing.T) {
			badInput := input
			badInput.ID = "bad"
			badInput.EndsAt = &bad
			if _, err := s.CreateAgreementAuthorized(ctx, "owner", "owner", badInput); ErrorStatus(err) != 400 {
				t.Fatalf("create accepted %q: %v", bad, err)
			}
			badInput.ID = input.ID
			if _, err := s.UpdateAgreementAuthorized(ctx, "owner", "owner", input.ID, badInput); ErrorStatus(err) != 400 {
				t.Fatalf("PUT accepted %q: %v", bad, err)
			}
			if _, err := s.PatchAgreementAuthorized(ctx, "owner", "owner", input.ID, PatchAgreementInput{EndsAt: PatchText{Present: true, Value: &bad}}); ErrorStatus(err) != 400 {
				t.Fatalf("PATCH accepted %q: %v", bad, err)
			}
		})
	}
	unchanged, err := s.GetAgreementAuthorized(ctx, "owner", "owner", input.ID, ReadStrong)
	if err != nil || ptrValue(unchanged.EndsAt) != ptrValue(a.EndsAt) {
		t.Fatal("invalid update changed interval", unchanged, err)
	}
	offset := "2030-01-01T12:00:00+02:00"
	a, err = s.PatchAgreementAuthorized(ctx, "owner", "owner", input.ID, PatchAgreementInput{EndsAt: PatchText{Present: true, Value: &offset}})
	if err != nil || ptrValue(a.EndsAt) != "2030-01-01T10:00:00.000000000Z" {
		t.Fatal(a, err)
	}
	a, err = s.PatchAgreementAuthorized(ctx, "owner", "owner", input.ID, PatchAgreementInput{EndsAt: PatchText{Present: true}})
	if err != nil || a.EndsAt != nil {
		t.Fatal("explicit null did not clear bound", a, err)
	}
}

func TestOrganizationOwnerBindingRecordsOperator(t *testing.T) {
	ctx, s := serviceFixture(t)
	ctx = WithManagementActor(ctx, "platform-operator")
	if _, err := s.CreateOrganizationAdmin(ctx, CreateOrganizationInput{ID: "audit-org", Name: "Audited", OwnerUserID: "owner"}); err != nil {
		t.Fatal(err)
	}
	rows, err := s.store.Query(ctx, ReadStrong, "SELECT created_by_user_id,subject_id FROM role_bindings WHERE subject_organization_id='audit-org'")
	if err != nil || len(rows) != 1 || rowString(rows[0], "created_by_user_id") != "platform-operator" || rowString(rows[0], "subject_id") != "owner" {
		t.Fatal(rows, err)
	}
}

func TestAgreementBoundsGuardRejectsConcurrentInvalidInterval(t *testing.T) {
	ctx, s := serviceFixture(t)
	if _, err := s.CreateOrganizationAdmin(ctx, CreateOrganizationInput{ID: "receiver", Name: "Receiver", OwnerUserID: "owner"}); err != nil {
		t.Fatal(err)
	}
	start, end := "2030-01-01T00:00:00.000000000Z", "2030-01-01T10:00:00.000000000Z"
	snapshot, err := s.CreateAgreementAuthorized(ctx, "owner", "owner", UpsertAgreementInput{ID: "interval-race", Title: "Race", AgreementType: "delegation", Status: "active", GrantorOrganizationID: "owner", GranteeOrganizationID: "receiver", StartsAt: &start, EndsAt: &end})
	if err != nil {
		t.Fatal(err)
	}
	// Writer A validates 09:00 against the old 10:00 end. Writer B commits 08:00.
	candidate := "2030-01-01T09:00:00.000000000Z"
	if _, _, err = normalizeAgreementInterval(&candidate, snapshot.EndsAt); err != nil {
		t.Fatal(err)
	}
	concurrentEnd := "2030-01-01T08:00:00.000000000Z"
	if _, err = s.PatchAgreementAuthorized(ctx, "owner", "owner", snapshot.ID, PatchAgreementInput{EndsAt: PatchText{Present: true, Value: &concurrentEnd}}); err != nil {
		t.Fatal(err)
	}
	err = s.store.Transaction(ctx, []Statement{agreementBoundsGuard(snapshot), {SQL: "UPDATE agreements SET starts_at=? WHERE id=?", Args: []any{candidate, snapshot.ID}}})
	if err == nil {
		t.Fatal("stale interval check committed an inverted range")
	}
	current, err := s.GetAgreementAuthorized(ctx, "owner", "owner", snapshot.ID, ReadStrong)
	if err != nil || ptrValue(current.StartsAt) != start || ptrValue(current.EndsAt) != concurrentEnd {
		t.Fatal(current, err)
	}
}
