package grant

import (
	"context"
	"testing"
)

func reactivateSetup(t *testing.T, ctx context.Context, s *Service) {
	t.Helper()
	now := nowString()
	must := func(err error) {
		t.Helper()
		if err != nil {
			t.Fatal(err)
		}
	}
	_, e := s.store.Exec(ctx, "INSERT OR IGNORE INTO actions(id,provider_id,code,name,risk_level,visibility_granting,grantable_to_applications,created_at,updated_at) VALUES('grant-management.organization.reactivate','grant-management','organization.reactivate','Reactivate','high',0,1,?,?)", now, now)
	must(e)
	_, e = s.store.Exec(ctx, "INSERT OR IGNORE INTO role_actions(role_id,action_id) VALUES('role-mgmt','grant-management.organization.reactivate')")
	must(e)
	_, e = s.store.Exec(ctx, "INSERT OR IGNORE INTO application_actions(application_id,action_id) VALUES('app-1','grant-management.organization.reactivate')")
	must(e)
	_, e = s.store.Exec(ctx, "INSERT OR IGNORE INTO application_grant_actions(application_grant_id,action_id) VALUES('g1','grant-management.organization.reactivate')")
	must(e)
	_, e = s.store.Exec(ctx, "UPDATE role_bindings SET resource_type='organization',resource_id='mgmt-org' WHERE id='bind-mgmt'")
	must(e)
	_, e = s.store.Exec(ctx, "UPDATE organizations SET inactivated_at=? WHERE id='mgmt-org'", now)
	must(e)
	// Every denial starts from a working app-scoped reactivation grant.
	reactivateCheck(t, ctx, s, true, true, reactivateBaseCheck())
}

func reactivateCheck(t *testing.T, ctx context.Context, s *Service, allow bool, inactive bool, c UserActionCheck) {
	t.Helper()
	ds, err := s.store.evaluateUserActions(ctx, []UserActionCheck{c}, inactive)
	if err != nil {
		t.Fatal(err)
	}
	if len(ds) != 1 {
		t.Fatalf("expected 1 decision, got %d", len(ds))
	}
	if ds[0].Allowed != allow {
		t.Fatalf("expected allowed=%v, got %v (rbac=%v, trusted=%v, consent=%v)", allow, ds[0].Allowed, ds[0].RBACAllowed, ds[0].TrustedApplication, ds[0].ApplicationAllowed)
	}
}

func reactivateBaseCheck() UserActionCheck {
	appID := "app-1"
	return UserActionCheck{
		UserID: "owner", OrganizationID: "mgmt-org",
		ProviderID: "grant-management", ResourceType: "organization",
		ResourceID: "mgmt-org", ActionID: "grant-management.organization.reactivate",
		ApplicationID: &appID, Consistency: ReadStrong,
	}
}

func TestManagementReactivationAllow(t *testing.T) {
	ctx, s := managementFixture(t, "mgmt-org")
	reactivateSetup(t, ctx, s)
	reactivateCheck(t, ctx, s, true, true, reactivateBaseCheck())
}

func TestManagementReactivationDenyRevokeConsent(t *testing.T) {
	ctx, s := managementFixture(t, "mgmt-org")
	reactivateSetup(t, ctx, s)
	_, err := s.store.Exec(ctx, "UPDATE application_grants SET status='revoked' WHERE id='g1'")
	if err != nil {
		t.Fatal(err)
	}
	reactivateCheck(t, ctx, s, false, true, reactivateBaseCheck())
}

func TestManagementReactivationDenyRemoveAppAction(t *testing.T) {
	ctx, s := managementFixture(t, "mgmt-org")
	reactivateSetup(t, ctx, s)
	_, err := s.store.Exec(ctx, "DELETE FROM application_actions WHERE application_id='app-1' AND action_id='grant-management.organization.reactivate'")
	if err != nil {
		t.Fatal(err)
	}
	reactivateCheck(t, ctx, s, false, true, reactivateBaseCheck())
}

func TestManagementReactivationDenyDisableBinding(t *testing.T) {
	ctx, s := managementFixture(t, "mgmt-org")
	reactivateSetup(t, ctx, s)
	_, err := s.store.Exec(ctx, "UPDATE role_bindings SET state='disabled' WHERE id='bind-mgmt'")
	if err != nil {
		t.Fatal(err)
	}
	reactivateCheck(t, ctx, s, false, true, reactivateBaseCheck())
}

func TestManagementReactivationDenyInactiveFalse(t *testing.T) {
	ctx, s := managementFixture(t, "mgmt-org")
	reactivateSetup(t, ctx, s)
	reactivateCheck(t, ctx, s, false, false, reactivateBaseCheck())
}

func TestManagementReactivationDenyWrongResourceID(t *testing.T) {
	ctx, s := managementFixture(t, "mgmt-org")
	reactivateSetup(t, ctx, s)
	c := reactivateBaseCheck()
	c.ResourceID = "other-org"
	reactivateCheck(t, ctx, s, false, true, c)
}

func TestManagementReactivationDenyWrongAction(t *testing.T) {
	ctx, s := managementFixture(t, "mgmt-org")
	reactivateSetup(t, ctx, s)
	c := reactivateBaseCheck()
	// The fixture grants this ordinary action too; only the inactive-org
	// boundary should prevent its use on the same organization resource.
	c.ActionID = "grant-management.admin.provider.create"
	reactivateCheck(t, ctx, s, false, true, c)
}

func TestManagementReactivationDenyInactiveUser(t *testing.T) {
	ctx, s := managementFixture(t, "mgmt-org")
	reactivateSetup(t, ctx, s)
	_, err := s.store.Exec(ctx, "UPDATE users SET active=0 WHERE id='owner'")
	if err != nil {
		t.Fatal(err)
	}
	reactivateCheck(t, ctx, s, false, true, reactivateBaseCheck())
}
