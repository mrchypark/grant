package grant

import (
	"strings"
	"testing"
)

func TestAgreementClassificationDoesNotChangeAuthority(t *testing.T) {
	ctx, s := serviceFixture(t)
	if _, err := s.CreateOrganizationAdmin(ctx, CreateOrganizationInput{ID: "receiver", Name: "Receiver", OwnerUserID: "owner"}); err != nil {
		t.Fatal(err)
	}
	in := UpsertAgreementInput{ID: "classification", Title: "Contract", AgreementType: "  공동 운영  ", Status: "active", GrantorOrganizationID: "owner", GranteeOrganizationID: "receiver"}
	a, err := s.CreateAgreementAuthorized(ctx, "owner", "owner", in)
	if err != nil || a.AgreementType != "공동 운영" {
		t.Fatal(a, err)
	}
	if _, err = s.CreateRoleAuthorized(ctx, "owner", "receiver", CreateRoleInput{ID: "class-role", Code: "class", Name: "Class"}); err != nil {
		t.Fatal(err)
	}
	if _, err = s.ReplaceRoleActionsAuthorized(ctx, "owner", "receiver", "class-role", []string{"organization.profile.read"}); err != nil {
		t.Fatal(err)
	}
	if _, err = s.CreateRoleBindingAuthorized(ctx, "owner", "receiver", CreateRoleBindingInput{ID: "class-binding", SubjectType: "organization", SubjectID: "receiver", SubjectOrganizationID: "receiver", ProviderID: "grant", ResourceType: "organization", ResourceID: "owner", RoleID: "class-role", AgreementID: &in.ID, State: "active"}); err != nil {
		t.Fatal(err)
	}
	check := func(want bool) {
		t.Helper()
		d, err := s.CheckAuthorization(ctx, AuthorizationSubject{SubjectType: "user", ID: "owner", OrganizationID: "receiver"}, "organization.profile.read", AuthorizationResource{ProviderID: "grant", ResourceType: "organization", ID: "owner"}, ReadStrong)
		if err != nil || d.Allowed != want {
			t.Fatal(d, err)
		}
	}
	check(true)
	for _, typ := range []string{"epc", "om", "distributor", "dealer", "delegation", "maintenance-contract", strings.Repeat("가", 128)} {
		a, err = s.PatchAgreementAuthorized(ctx, "owner", "receiver", in.ID, PatchAgreementInput{AgreementType: &typ})
		if err != nil || a.AgreementType != typ {
			t.Fatal(a, err)
		}
		check(true)
	}
	in.AgreementType = "custom-service-category"
	in.Status = "terminated"
	if _, err = s.UpdateAgreementAuthorized(ctx, "owner", "owner", in.ID, in); err != nil {
		t.Fatal(err)
	}
	check(false)
	typ := "delegation"
	if _, err = s.PatchAgreementAuthorized(ctx, "owner", "receiver", in.ID, PatchAgreementInput{AgreementType: &typ}); err != nil {
		t.Fatal(err)
	}
	check(false)
}

func TestAgreementClassificationValidationAllWritePaths(t *testing.T) {
	ctx, s := serviceFixture(t)
	if _, err := s.CreateOrganizationAdmin(ctx, CreateOrganizationInput{ID: "receiver", Name: "Receiver", OwnerUserID: "owner"}); err != nil {
		t.Fatal(err)
	}
	in := UpsertAgreementInput{ID: "classification", Title: "Contract", AgreementType: "delegation", Status: "active", GrantorOrganizationID: "owner", GranteeOrganizationID: "receiver"}
	if _, err := s.CreateAgreementAuthorized(ctx, "owner", "owner", in); err != nil {
		t.Fatal(err)
	}
	for _, bad := range []string{"", "   ", strings.Repeat("a", 129), "bad\x00type", "bad\ntype", string([]byte{0xff})} {
		invalid := in
		invalid.AgreementType = bad
		invalid.ID = "invalid"
		if _, err := s.CreateAgreementAuthorized(ctx, "owner", "owner", invalid); ErrorStatus(err) != 400 {
			t.Fatalf("create %q: %v", bad, err)
		}
		invalid.ID = in.ID
		if _, err := s.UpdateAgreementAuthorized(ctx, "owner", "owner", in.ID, invalid); ErrorStatus(err) != 400 {
			t.Fatalf("PUT %q: %v", bad, err)
		}
		if _, err := s.PatchAgreementAuthorized(ctx, "owner", "owner", in.ID, PatchAgreementInput{AgreementType: &bad}); ErrorStatus(err) != 400 {
			t.Fatalf("PATCH %q: %v", bad, err)
		}
	}
	a, err := s.GetAgreementAuthorized(ctx, "owner", "owner", in.ID, ReadStrong)
	if err != nil || a.AgreementType != "delegation" {
		t.Fatal("invalid patch changed persisted type", a, err)
	}
}
