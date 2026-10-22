package grant

import (
	"context"
	"testing"
)

func serviceFixture(t *testing.T) (context.Context, *Service) {
	t.Helper()
	ctx := context.Background()
	cfg := Config{Security: SecurityConfig{BootstrapUserID: "owner", BootstrapUserName: "Owner"}, Database: DatabaseConfig{DataDir: t.TempDir(), DefaultReadConsistency: ReadStrong}}
	store, err := OpenStore(ctx, cfg)
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() { _ = store.Close() })
	if err = store.EnsureBootstrapData(ctx); err != nil {
		t.Fatal(err)
	}
	return ctx, NewService(cfg, store)
}
func TestServiceGroupsAndRolesRespectScope(t *testing.T) {
	ctx, s := serviceFixture(t)
	g, err := s.CreateGroupAuthorized(ctx, "owner", "owner", UpsertGroupInput{ID: "team", Name: "Team"})
	if err != nil {
		t.Fatal(err)
	}
	if g.OrganizationID != "owner" {
		t.Fatal(g)
	}
	if _, err = s.GetGroupAuthorized(ctx, "owner", "other", "team", ReadStrong); err == nil || ErrorStatus(err) != 403 {
		t.Fatal("cross tenant group access", err)
	}
	if _, err = s.CreateRoleAuthorized(ctx, "owner", "owner", CreateRoleInput{ID: "role", Code: "reader", Name: "Reader"}); err != nil {
		t.Fatal(err)
	}
	actions, err := s.ReplaceRoleActionsAuthorized(ctx, "owner", "owner", "role", []string{"organization.profile.read", "organization.profile.read"})
	if err != nil || len(actions) != 1 {
		t.Fatal(actions, err)
	}
	if _, err = s.ReplaceRoleActionsAuthorized(ctx, "owner", "owner", "role", []string{"missing"}); err == nil {
		t.Fatal("unknown action accepted")
	}
	actions, err = serviceRows[Action](ctx, s, ReadStrong, "SELECT a.* FROM actions a JOIN role_actions ra ON ra.action_id=a.id WHERE ra.role_id='role'")
	if err != nil || len(actions) != 1 {
		t.Fatal("failed replacement lost prior actions", actions, err)
	}
	if err = s.DeleteGroupAuthorized(ctx, "owner", "owner", "team"); err != nil {
		t.Fatal(err)
	}
}
func TestServiceActionCatalogCoarseAndAtomic(t *testing.T) {
	ctx, s := serviceFixture(t)
	if _, err := s.UpsertServiceProviderAdmin(ctx, ServiceProvider{ID: "provider", Name: "Provider", Active: true}); err != nil {
		t.Fatal(err)
	}
	actions, err := s.ReplaceProviderActionsAdmin(ctx, "provider", []Action{{ID: "read", ProviderID: "provider", Code: "read", Name: "Read", RiskLevel: "low"}})
	if err != nil || len(actions) != 1 || actions[0].ID != "provider.read" {
		t.Fatal(actions, err)
	}
	if _, err = s.ReplaceProviderActionsAdmin(ctx, "provider", []Action{{ID: "view", ProviderID: "provider", Code: "view", Name: "View", RiskLevel: "low", VisibilityGranting: true}}); err == nil || ErrorStatus(err) != 400 {
		t.Fatal("field action accepted", err)
	}
	rows, err := s.store.Query(ctx, ReadStrong, "SELECT id FROM actions WHERE provider_id='provider'")
	if err != nil || len(rows) != 1 || rowString(rows[0], "id") != "provider.read" {
		t.Fatal(rows, err)
	}
}
func TestServiceLocalUserKillSwitch(t *testing.T) {
	ctx, s := serviceFixture(t)
	if _, err := s.store.SetUserActive(ctx, "owner", false); err != nil {
		rows, readErr := s.store.Query(ctx, ReadStrong, "SELECT id,active FROM users")
		t.Fatalf("set active: %v; users=%v readErr=%v", err, rows, readErr)
	}
	d, err := s.CheckAuthorization(ctx, AuthorizationSubject{SubjectType: "user", ID: "owner", OrganizationID: "owner"}, "organization.profile.read", AuthorizationResource{ProviderID: "grant", ResourceType: "organization", ID: "owner"}, ReadStrong)
	if err != nil {
		t.Fatal(err)
	}
	if d.Allowed {
		t.Fatal("inactive user authorized")
	}
	if _, err = s.CreateGroupAuthorized(ctx, "owner", "owner", UpsertGroupInput{ID: "blocked", Name: "Blocked"}); err == nil || ErrorStatus(err) != 403 {
		t.Fatal("inactive actor mutated", err)
	}
}
