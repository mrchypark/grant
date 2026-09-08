package grant

import (
	"context"
	"fmt"
	"strings"
	"testing"
)

func TestLargeProviderActionReplacementIsAtomic(t *testing.T) {
	ctx := context.Background()
	cfg := Config{Database: DatabaseConfig{DataDir: t.TempDir()}}
	store, err := OpenStore(ctx, cfg)
	if err != nil {
		t.Fatal(err)
	}
	defer store.Close()
	if err = store.EnsureBootstrapData(ctx); err != nil {
		t.Fatal(err)
	}
	svc := NewService(cfg, store)
	if _, err = svc.UpsertServiceProviderAdmin(ctx, ServiceProvider{ID: "catalog-test", Name: "Catalog test", Active: true}); err != nil {
		t.Fatal(err)
	}
	description := strings.Repeat("large action metadata ", 50)
	actions := make([]Action, 200)
	for i := range actions {
		code := fmt.Sprintf("a-%03d", i)
		actions[i] = Action{ID: "catalog-test." + code, ProviderID: "catalog-test", Code: code, Name: code, Description: &description, RiskLevel: "high", GrantableToApplications: true}
	}
	if got, err := svc.ReplaceProviderActionsAdmin(ctx, "catalog-test", actions); err != nil || len(got) != 200 {
		t.Fatal("large replacement", len(got), err)
	}
	rows, err := store.Query(ctx, ReadStrong, "SELECT COUNT(*) AS n FROM grant_import_payloads WHERE kind='provider-actions'")
	if err != nil || rowInt(rows[0], "n") != 0 {
		t.Fatal("successful stage not cleaned", rows, err)
	}
	last := actions[len(actions)-1].ID
	if _, err = store.Exec(ctx, "INSERT INTO role_actions(role_id,action_id) VALUES('role-system-bootstrap-owner',?)", last); err != nil {
		t.Fatal(err)
	}
	// A conflicting code/ID pair fails only at the final insert, after the
	// omitted action and its role edge would have been removed in that txn.
	invalid := append([]Action(nil), actions[:len(actions)-1]...)
	invalid[0].Code = invalid[0].ID
	if _, err = svc.ReplaceProviderActionsAdmin(ctx, "catalog-test", invalid); err == nil {
		t.Fatal("conflicting ID accepted")
	}
	rows, err = store.Query(ctx, ReadStrong, "SELECT (SELECT COUNT(*) FROM actions WHERE provider_id='catalog-test') AS n,(SELECT COUNT(*) FROM role_actions WHERE action_id=?) AS bindings", last)
	if err != nil || rowInt(rows[0], "n") != 200 || rowInt(rows[0], "bindings") != 1 {
		t.Fatal("failed replacement modified authority", rows, err)
	}
	if _, err = svc.ReplaceProviderActionsAdmin(ctx, "catalog-test", actions); err != nil {
		t.Fatal("retry after failed final transaction", err)
	}
	rows, err = store.Query(ctx, ReadStrong, "SELECT COUNT(*) AS n FROM role_actions WHERE action_id=?", last)
	if err != nil || rowInt(rows[0], "n") != 1 {
		t.Fatal("retry removed retained grant", rows, err)
	}
}
