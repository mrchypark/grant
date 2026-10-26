package grant

import (
	"context"
	"strings"
	"testing"

	"github.com/mrchypark/rhiza"
)

func TestPepIdentityMigrationPreservesLegacyRowsAndLifecycle(t *testing.T) {
	ctx := context.Background()
	dir := t.TempDir()
	db, err := rhiza.Open(ctx, rhiza.Config{DataDir: dir, NodeID: "grant-1", ClusterID: "grant"})
	if err != nil {
		t.Fatal(err)
	}
	legacy := []rhiza.Migration{}
	for i, group := range []struct{ name, sql string }{
		{"grant canonical tables", schemaSQL},
		{"grant canonical indexes", indexesSQL},
		{"grant restore activation", "CREATE TABLE IF NOT EXISTS grant_restore_activation (id TEXT PRIMARY KEY, snapshot_digest TEXT NOT NULL, cluster_id TEXT NOT NULL, activated_at TEXT NOT NULL)"},
		{"grant qualified Goauthy identities", goauthySchemaV4SQL},
	} {
		var statements []rhiza.SQLStatement
		for _, sql := range strings.Split(group.sql, ";") {
			if strings.TrimSpace(sql) != "" {
				statements = append(statements, rhiza.SQLStatement{SQL: sql})
			}
		}
		legacy = append(legacy, rhiza.Migration{Version: int64(i + 1), Name: group.name, Statements: statements})
	}
	if err = db.Migrate(ctx, legacy); err != nil {
		t.Fatal(err)
	}
	legacyStore := &Store{db: db}
	now, expiry := "2026-10-01T00:00:00Z", "2027-10-01T00:00:00Z"
	for _, sql := range []string{
		"INSERT INTO organizations(id,name,created_at,updated_at) VALUES('legacy-org','Legacy','" + now + "','" + now + "')",
		"INSERT INTO service_providers(id,name,active,created_at,updated_at) VALUES('legacy-provider','Legacy',1,'" + now + "','" + now + "')",
		"INSERT INTO applications(id,organization_id,slug,name,application_type,app_type,consent_policy,active,provisioning_status,created_by_user_id,created_at,updated_at) VALUES('legacy-disabled-app','legacy-org','legacy-disabled','Legacy disabled','confidential','internal','standard',1,'ready','owner','" + now + "','" + now + "')",
		"INSERT INTO applications(id,organization_id,slug,name,application_type,app_type,consent_policy,active,provisioning_status,created_by_user_id,created_at,updated_at) VALUES('legacy-deleted-app','legacy-org','legacy-deleted','Legacy deleted','confidential','internal','standard',1,'ready','owner','" + now + "','" + now + "')",
		"INSERT INTO provider_pep_clients VALUES('legacy-disabled-pep','legacy-disabled-app','legacy-provider','disabled','" + expiry + "','" + now + "','" + now + "')",
		"INSERT INTO provider_pep_clients VALUES('legacy-deleted-pep','legacy-deleted-app','legacy-provider','deleted',NULL,'" + now + "','" + now + "')",
		"INSERT INTO provider_pep_client_receipts VALUES('legacy-disabled-receipt','legacy-disabled-pep','legacy-disabled-app','legacy-provider','disabled','" + expiry + "','" + now + "','operator')",
		"INSERT INTO provider_pep_client_receipts VALUES('legacy-deleted-receipt','legacy-deleted-pep','legacy-deleted-app','legacy-provider','deleted',NULL,'" + now + "','operator')",
	} {
		if _, err = legacyStore.Exec(ctx, sql); err != nil {
			t.Fatal(err)
		}
	}
	legacyRows, err := db.Query(ctx, rhiza.QueryRequest{SQL: "SELECT pep_client_id,lifecycle FROM provider_pep_clients ORDER BY pep_client_id"})
	if err != nil || len(legacyRows.Rows) != 2 {
		t.Fatalf("v4 fixture was not populated: %#v %v", legacyRows, err)
	}
	if err = db.Close(); err != nil {
		t.Fatal(err)
	}

	s, err := OpenStore(ctx, Config{Database: DatabaseConfig{DataDir: dir}})
	if err != nil {
		t.Fatal(err)
	}
	defer s.Close()
	rows, err := s.Query(ctx, ReadStrong, "SELECT pep_client_id,application_id,lifecycle,valid_until FROM provider_pep_clients ORDER BY pep_client_id")
	if err != nil || len(rows) != 2 || rowString(rows[0], "application_id") != "legacy-deleted-app" || rowString(rows[0], "lifecycle") != "deleted" || rows[0]["valid_until"] != nil || rowString(rows[1], "application_id") != "legacy-disabled-app" || rowString(rows[1], "lifecycle") != "disabled" || rowString(rows[1], "valid_until") != expiry {
		t.Fatalf("legacy PEP state changed during migration: %#v %v", rows, err)
	}
	receipts, err := s.Query(ctx, ReadStrong, "SELECT receipt_id,application_id,lifecycle FROM provider_pep_client_receipts ORDER BY receipt_id")
	if err != nil || len(receipts) != 2 || rowString(receipts[0], "application_id") != "legacy-deleted-app" || rowString(receipts[0], "lifecycle") != "deleted" || rowString(receipts[1], "application_id") != "legacy-disabled-app" || rowString(receipts[1], "lifecycle") != "disabled" {
		t.Fatalf("legacy PEP receipts changed during migration: %#v %v", receipts, err)
	}
	indexes, err := s.Query(ctx, ReadStrong, "SELECT name FROM sqlite_schema WHERE type='index' AND name='idx_provider_pep_clients_provider_lifecycle'")
	if err != nil || len(indexes) != 1 {
		t.Fatalf("PEP lifecycle index missing after migration: %#v %v", indexes, err)
	}
	foreignKeys, err := s.Query(ctx, ReadStrong, "PRAGMA foreign_key_list(provider_pep_clients)")
	if err != nil || len(foreignKeys) != 1 || rowString(foreignKeys[0], "table") != "service_providers" {
		t.Fatalf("PEP binding still depends on applications: %#v %v", foreignKeys, err)
	}
	if _, err = s.Exec(ctx, "DELETE FROM applications WHERE id IN('legacy-disabled-app','legacy-deleted-app')"); err != nil {
		t.Fatalf("legacy application deletion blocked independent PEP: %v", err)
	}
	if err = s.Migrate(ctx); err != nil {
		t.Fatalf("migration not idempotent: %v", err)
	}
}
