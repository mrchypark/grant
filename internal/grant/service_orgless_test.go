package grant

import (
	"context"
	"testing"
)

func TestOrglessBindingRevocationSurvivesInactiveDependencies(t *testing.T) {
	for _, tc := range []struct{ name, mutation string }{
		{"inactive-user", "UPDATE users SET active=0 WHERE id='owner'"},
		{"inactive-provider", "UPDATE service_providers SET active=0 WHERE id='grant'"},
		{"action-no-longer-grantable", "UPDATE actions SET grantable_to_applications=0 WHERE id='organization.profile.read'"},
		{"removed-role-action", "DELETE FROM role_actions WHERE action_id='organization.profile.read'"},
		{"inactive-owner-organization", "UPDATE organizations SET inactivated_at='now' WHERE id='owner'"},
	} {
		t.Run(tc.name, func(t *testing.T) {
			ctx, s := serviceFixture(t)
			input := OrglessUserBinding{SubjectID: "owner", ProviderID: "grant", ResourceType: "document", ResourceID: "one", ActionID: "organization.profile.read", State: "active"}
			got, err := s.SetOrglessUserBindingAdmin(ctx, "orgless-revoke", input)
			if err != nil || got.State != "active" {
				t.Fatal(got, err)
			}
			if _, err = s.store.Exec(ctx, tc.mutation); err != nil {
				t.Fatal(err)
			}
			input.State = "disabled"
			got, err = s.SetOrglessUserBindingAdmin(ctx, "orgless-revoke", input)
			if err != nil || got.State != "disabled" || got.SubjectOrganizationID != "" {
				t.Fatalf("revocation blocked by %s: %+v %v", tc.name, got, err)
			}
			// An idempotent retry must also work while dependencies remain unavailable.
			if _, err = s.SetOrglessUserBindingAdmin(ctx, "orgless-revoke", input); err != nil {
				t.Fatal("retry", err)
			}
			input.ResourceID = "other"
			if _, err = s.SetOrglessUserBindingAdmin(ctx, "orgless-revoke", input); err == nil {
				t.Fatal("revocation silently retargeted grant")
			}
		})
	}
}

func TestOrglessBindingSupportsResourceValuesAndProtectsTenantAuthority(t *testing.T) {
	ctx, s := serviceFixture(t)
	for _, resource := range []string{"{550e8400-e29b-41d4-a716-446655440000}", "urn:uuid:550e8400-e29b-41d4-a716-446655440000"} {
		if err := scalarFormat("uuid", resource); err != nil {
			t.Fatal("invalid test UUID", err)
		}
		input := OrglessUserBinding{SubjectID: "owner", ProviderID: "grant", ResourceType: "document", ResourceID: resource, ActionID: "organization.profile.read", State: "active"}
		got, err := s.SetOrglessUserBindingAdmin(ctx, "uuid-resource", input)
		if err != nil || got.ResourceID != resource {
			t.Fatalf("supported UUID rejected: %+v %v", got, err)
		}
		if err = s.SetRoleAction(ctx, "owner", "owner", got.RoleID, "organization.permission.manage", true); err == nil {
			t.Fatal("tenant mutated operator global role")
		}
		if _, err = s.CreateRoleAuthorized(ctx, "owner", "owner", CreateRoleInput{ID: got.RoleID, Code: "forged", Name: "Forged"}); err == nil {
			t.Fatal("tenant reserved operator role")
		}
		if _, err = s.CreateRoleBindingAuthorized(ctx, "owner", "owner", CreateRoleBindingInput{ID: "forged-orgless", SubjectType: "user", SubjectID: "owner", SubjectOrganizationID: "", ProviderID: "grant", ResourceType: "document", ResourceID: resource, RoleID: got.RoleID, State: "active"}); ErrorStatus(err) != 403 {
			t.Fatal("tenant minted empty-organization grant", err)
		}
		input.State = "disabled"
		if _, err = s.SetOrglessUserBindingAdmin(ctx, "uuid-resource", input); err != nil {
			t.Fatal(err)
		}
	}
	for _, value := range []string{"", " padded", "trailing ", "line\nbreak", "zero\x00byte", string([]byte{0xff})} {
		if _, err := s.SetOrglessUserBindingAdmin(ctx, "bad-resource", OrglessUserBinding{SubjectID: "owner", ProviderID: "grant", ResourceType: "document", ResourceID: value, ActionID: "organization.profile.read", State: "active"}); ErrorStatus(err) != 422 {
			t.Fatal("invalid resource accepted", value, err)
		}
	}
}

func TestOrglessBindingDoesNotUseTenantOrGroupEntitlements(t *testing.T) {
	ctx, s := serviceFixture(t)
	app := Application{ID: "orgless-app", OrganizationID: "owner", Slug: "orgless-app", Name: "Orgless", ApplicationType: "confidential", AppType: "server", Active: true, ProvisioningStatus: "ready", CreatedByUserID: "owner"}
	if err := s.store.InsertApplicationWithRedirectURI(ctx, app, nil); err != nil {
		t.Fatal(err)
	}
	if _, err := s.store.Exec(ctx, "INSERT INTO application_actions(application_id,action_id) VALUES('orgless-app','organization.profile.read')"); err != nil {
		t.Fatal(err)
	}
	c := applicationAuthorizationContext{UserID: "owner", ApplicationID: app.ID}
	check := func(want bool) {
		t.Helper()
		got, err := s.orglessActionAllowed(ctx, c, "grant", "organization.profile.read", "document", "one")
		if err != nil || got != want {
			t.Fatal("entitlement", got, want, err)
		}
	}
	check(false)
	input := OrglessUserBinding{SubjectID: "owner", ProviderID: "grant", ResourceType: "document", ResourceID: "one", ActionID: "organization.profile.read", State: "active"}
	if _, err := s.SetOrglessUserBindingAdmin(ctx, "direct", input); err != nil {
		t.Fatal(err)
	}
	check(true)
	for _, tc := range []struct{ apply, restore string }{
		{"UPDATE role_bindings SET subject_organization_id='owner' WHERE id='direct'", "UPDATE role_bindings SET subject_organization_id='' WHERE id='direct'"},
		{"UPDATE role_bindings SET subject_type='group' WHERE id='direct'", "UPDATE role_bindings SET subject_type='user' WHERE id='direct'"},
		{"UPDATE role_bindings SET agreement_id='agreement' WHERE id='direct'", "UPDATE role_bindings SET agreement_id=NULL WHERE id='direct'"},
		{"UPDATE role_bindings SET resource_id='other' WHERE id='direct'", "UPDATE role_bindings SET resource_id='one' WHERE id='direct'"},
		{"DELETE FROM application_actions WHERE application_id='orgless-app'", "INSERT INTO application_actions(application_id,action_id) VALUES('orgless-app','organization.profile.read')"},
	} {
		if _, err := s.store.Exec(ctx, tc.apply); err != nil {
			t.Fatal(err)
		}
		check(false)
		if _, err := s.store.Exec(ctx, tc.restore); err != nil {
			t.Fatal(err)
		}
		check(true)
	}
	// Losing an action does not make revocation depend on the action relation.
	if _, err := s.store.Exec(ctx, "DELETE FROM role_actions WHERE action_id='organization.profile.read'"); err != nil {
		t.Fatal(err)
	}
	if _, err := s.store.Exec(ctx, "DELETE FROM application_actions WHERE action_id='organization.profile.read'"); err != nil {
		t.Fatal(err)
	}
	if _, err := s.store.Exec(ctx, "DELETE FROM actions WHERE id='organization.profile.read'"); err != nil {
		t.Fatal(err)
	}
	input.State = "disabled"
	if _, err := s.SetOrglessUserBindingAdmin(context.Background(), "direct", input); err != nil {
		t.Fatal("removed action blocked revocation", err)
	}
	check(false)
}
