package grant

import "testing"

func TestDeletedApplicationEmptyETagCannotAuthorizeReplacement(t *testing.T) {
	for _, replacementOrg := range []string{"owner", "replacement-org"} {
		t.Run(replacementOrg, func(t *testing.T) {
			ctx, s := serviceFixture(t)
			lifecycleApp(t, ctx, s)
			before, err := s.ApplicationActionsSnapshot(ctx, "owner", "owner", "life-app")
			if err != nil {
				t.Fatal(err)
			}
			if string(before.Value) != "[]" {
				t.Fatal("snapshot DTO changed", string(before.Value))
			}
			pending, err := checkedActionContext(ctx, before, before.ETag, "")
			if err != nil {
				t.Fatal(err)
			}
			target, err := serviceRow[applicationActionTarget](ctx, s, ReadStrong, "SELECT id,organization_id,created_at FROM applications WHERE id='life-app'")
			if err != nil {
				t.Fatal(err)
			}
			if err = s.DeleteApplicationAuthorized(ctx, "owner", "owner", "life-app"); err != nil {
				t.Fatal(err)
			}
			if replacementOrg != "owner" {
				if _, err = s.CreateOrganizationAdmin(ctx, CreateOrganizationInput{ID: replacementOrg, Name: "Replacement", OwnerUserID: "owner"}); err != nil {
					t.Fatal(err)
				}
			}
			if _, err = s.CreateApplication(ctx, "owner", replacementOrg, CreateApplicationInput{ID: "life-app", Slug: "life-app", Name: "Replacement", ApplicationType: "public", AppType: "web", Active: true}); err != nil {
				t.Fatal(err)
			}
			after, err := s.ApplicationActionsSnapshot(ctx, "owner", replacementOrg, "life-app")
			if err != nil {
				t.Fatal(err)
			}
			if before.ETag == after.ETag {
				t.Fatal("empty action ETag survived application recreation")
			}
			if _, err = s.ReplaceApplicationActionsConditional(ctx, "owner", replacementOrg, "life-app", []string{"organization.profile.read"}, before.ETag); ErrorStatus(err) != 412 {
				t.Fatalf("old ETag accepted: %v", err)
			}
			// Simulate a request that passed authorization before deletion and reaches
			// its store transaction only after another app has reused the ID.
			if _, err = s.store.replaceApplicationActions(pending, target, "owner", []string{"organization.profile.read"}); ErrorStatus(err) != 412 {
				t.Fatalf("pending old transaction = %v", err)
			}
			if _, err = s.store.replaceApplicationActions(ctx, target, "owner", []string{"organization.profile.read"}); err == nil {
				t.Fatal("unguarded stale incarnation accepted")
			}
			actions, err := s.ListApplicationActionsAuthorized(ctx, "owner", replacementOrg, "life-app", ReadStrong)
			if err != nil || len(actions) != 0 {
				t.Fatal("replacement application changed", actions, err)
			}
		})
	}
}

func TestBulkApplicationActionsRecheckActorInTransaction(t *testing.T) {
	ctx, s := serviceFixture(t)
	lifecycleApp(t, ctx, s)
	target, err := serviceRow[applicationActionTarget](ctx, s, ReadStrong, "SELECT id,organization_id,created_at FROM applications WHERE id='life-app'")
	if err != nil {
		t.Fatal(err)
	}
	lifecycleExec(t, ctx, s, "UPDATE role_bindings SET state='disabled' WHERE subject_id='owner'")
	if _, err = s.store.replaceApplicationActions(ctx, target, "owner", []string{"organization.profile.read"}); err == nil {
		t.Fatal("revoked actor changed actions")
	}
	if lifecycleCount(t, ctx, s, "SELECT * FROM application_actions WHERE application_id='life-app'") != 0 {
		t.Fatal("revoked actor persisted actions")
	}
}

func TestRemovedAdminReenrollsAsMemberWithoutAuthority(t *testing.T) {
	ctx, s := serviceFixture(t)
	lifecycleExec(t, ctx, s, "INSERT INTO users(id,display_name,active,created_at) VALUES('admin','Admin',1,?)", nowString())
	if _, err := s.EnrollOrganizationMemberAuthorized(ctx, "owner", "owner", "admin"); err != nil {
		t.Fatal(err)
	}
	lifecycleExec(t, ctx, s, "UPDATE organization_memberships SET membership_role='admin' WHERE user_id='admin' AND organization_id='owner'")
	if _, err := s.CreateRoleBindingAuthorized(ctx, "owner", "owner", CreateRoleBindingInput{ID: "admin-role-binding", SubjectType: "user", SubjectID: "admin", SubjectOrganizationID: "owner", ProviderID: "grant", ResourceType: "organization", ResourceID: "owner", RoleID: "role-system-bootstrap-owner", State: "active"}); err != nil {
		t.Fatal(err)
	}
	if err := s.RemoveOrganizationMemberAuthorized(ctx, "owner", "owner", "admin"); err != nil {
		t.Fatal(err)
	}
	member, err := s.EnrollOrganizationMemberAuthorized(ctx, "owner", "owner", "admin")
	if err != nil {
		t.Fatal("removed admin cannot re-enroll", err)
	}
	if member.MembershipRole != "member" || member.Status != "active" {
		t.Fatal("admin role restored", member)
	}
	decision, err := s.CheckAuthorization(ctx, AuthorizationSubject{SubjectType: "user", ID: "admin", OrganizationID: "owner"}, "organization.application.manage", AuthorizationResource{ProviderID: "grant", ResourceType: "organization", ID: "owner"}, ReadStrong)
	if err != nil || decision.Allowed {
		t.Fatal("old administrative authority restored", decision, err)
	}
}
