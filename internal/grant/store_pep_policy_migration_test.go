package grant

import (
	"context"
	"strings"
	"testing"

	"github.com/mrchypark/rhiza"
)

func TestPepPolicyMigrationPreservesV5PolicyAndReceipts(t *testing.T) {
	ctx := context.Background()
	dir := t.TempDir()
	db, err := rhiza.Open(ctx, rhiza.Config{DataDir: dir, NodeID: "grant-1", ClusterID: "grant"})
	if err != nil {
		t.Fatal(err)
	}
	closed := false
	t.Cleanup(func() {
		if !closed {
			_ = db.Close()
		}
	})
	var migrations []rhiza.Migration
	for i, group := range []struct{ name, sql string }{
		{"grant canonical tables", schemaSQL},
		{"grant canonical indexes", indexesSQL},
		{"grant restore activation", "CREATE TABLE IF NOT EXISTS grant_restore_activation (id TEXT PRIMARY KEY, snapshot_digest TEXT NOT NULL, cluster_id TEXT NOT NULL, activated_at TEXT NOT NULL)"},
		{"grant qualified Goauthy identities", goauthySchemaV4SQL},
		{"grant independent PEP identities", pepIdentityMigrationSQL},
	} {
		var statements []rhiza.SQLStatement
		for _, sql := range strings.Split(group.sql, ";") {
			if strings.TrimSpace(sql) != "" {
				statements = append(statements, rhiza.SQLStatement{SQL: sql})
			}
		}
		migrations = append(migrations, rhiza.Migration{Version: int64(i + 1), Name: group.name, Statements: statements})
	}
	if err = db.Migrate(ctx, migrations); err != nil {
		t.Fatal(err)
	}
	legacy := &Store{db: db}
	if _, err = legacy.Exec(ctx, "INSERT INTO service_providers(id,name,active,created_at,updated_at) VALUES('legacy-provider','Legacy',1,'2026-10-01','2026-10-01')"); err != nil {
		t.Fatal(err)
	}
	for _, lifecycle := range []string{"active", "disabled", "deleted"} {
		if _, err = legacy.Exec(ctx, "INSERT INTO provider_pep_clients VALUES(?,NULL,'legacy-provider',?,'2026-10-01T00:00:00Z','2026-10-01','2026-10-01')", lifecycle, lifecycle); err != nil {
			t.Fatal(err)
		}
		if _, err = legacy.Exec(ctx, "INSERT INTO provider_pep_client_receipts VALUES(?,?,NULL,'legacy-provider',?,'2026-10-01T00:00:00Z','2026-10-01','operator')", lifecycle+"-receipt", lifecycle, lifecycle); err != nil {
			t.Fatal(err)
		}
	}
	if err = db.Close(); err != nil {
		t.Fatal(err)
	}
	closed = true
	s, err := OpenStore(ctx, Config{Database: DatabaseConfig{DataDir: dir}})
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() { _ = s.Close() })
	for _, lifecycle := range []string{"active", "disabled", "deleted"} {
		pep, err := s.ProviderPepClientAdmin(ctx, "legacy-provider", lifecycle)
		if err != nil || pep.AuthorizationMode != "application" || pep.PolicyRevision != 1 || pep.Lifecycle != lifecycle || pep.ValidUntil == nil || *pep.ValidUntil != "2026-10-01T00:00:00Z" || pep.ApplicationID != "" {
			t.Fatalf("migration changed legacy policy: %+v %v", pep, err)
		}
	}
	rows, err := s.Query(ctx, ReadStrong, "SELECT * FROM provider_pep_client_receipts ORDER BY receipt_id")
	if err != nil || len(rows) != 3 {
		t.Fatalf("legacy receipts: %v %v", rows, err)
	}
	for _, row := range rows {
		if rowString(row, "authorization_mode") != "application" || rowInt(row, "policy_revision") != 1 || rowString(row, "changed_by") != "operator" || row["application_id"] != nil {
			t.Fatalf("migration changed receipt: %v", row)
		}
	}
	if err = s.Migrate(ctx); err != nil {
		t.Fatalf("append-only migration must replay unchanged checksums: %v", err)
	}
	if canonicalSchemaDigest() == legacySchemaDigestV5() {
		t.Fatal("new policy columns missing from canonical schema digest")
	}
	if _, err = s.Exec(ctx, "UPDATE provider_pep_clients SET authorization_mode=NULL WHERE pep_client_id='active'"); err == nil {
		t.Fatal("NULL mode accepted")
	}
	if _, err = s.Exec(ctx, "UPDATE provider_pep_clients SET authorization_mode='automatic' WHERE pep_client_id='active'"); err == nil {
		t.Fatal("unknown mode accepted")
	}
}
