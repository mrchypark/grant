package grant

import (
	"context"
	"path/filepath"
	"strings"
	"testing"
)

// Materialize the actual historical table and column inventory.
func pepSnapshotFixtureVersion(t *testing.T, source StoreSnapshot, digest string) StoreSnapshot {
	t.Helper()
	out := source
	out.SchemaDigest = digest
	out.Tables = make([]SnapshotTable, 0, len(source.Tables))
	for _, original := range source.Tables {
		if digest != canonicalSchemaDigest() && grpcSnapshotColumns(original.Name) != nil {
			if original.Count != 0 || len(original.Rows) != 0 {
				t.Fatal("legacy fixture would drop gRPC authority")
			}
			continue
		}
		table := original
		table.Columns = append([]string(nil), original.Columns...)
		table.Rows = make([][]any, len(original.Rows))
		for j, row := range original.Rows {
			table.Rows[j] = append([]any(nil), row...)
		}
		if !snapshotHasPEPPolicy(digest) && (table.Name == "provider_pep_clients" || table.Name == "provider_pep_client_receipts") {
			table.Columns = table.Columns[:len(table.Columns)-2]
			for j, row := range table.Rows {
				table.Rows[j] = row[:len(row)-2]
			}
		}
		var err error
		table.Digest, err = tableDigest(table)
		if err != nil {
			t.Fatal(err)
		}
		out.Tables = append(out.Tables, table)
	}
	var err error
	out.Digest, err = snapshotDigest(out)
	if err != nil {
		t.Fatal(err)
	}
	return out
}

func TestPepPolicyV6SnapshotRoundTripPreservesAuthority(t *testing.T) {
	ctx := context.Background()
	s, err := OpenStore(ctx, Config{Database: DatabaseConfig{DataDir: t.TempDir()}})
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() { _ = s.Close() })
	if err = s.EnsureBootstrapData(ctx); err != nil {
		t.Fatal(err)
	}
	pep, err := s.UpsertProviderPepClient(ctx, "grant", "backup-pep", "bootstrap", UpsertProviderPepClient{Lifecycle: "active"})
	if err != nil {
		t.Fatal(err)
	}
	user := "user"
	pep, err = s.UpsertProviderPepClientConditional(ctx, "grant", "backup-pep", "operator", UpsertProviderPepClient{Lifecycle: "disabled", AuthorizationMode: &user}, &pep.PolicyRevision)
	if err != nil {
		t.Fatal(err)
	}
	snapshot, err := s.ExportSnapshot(ctx)
	if err != nil {
		t.Fatal(err)
	}
	path := filepath.Join(t.TempDir(), "snapshot.json")
	if err = WriteStoreSnapshot(path, snapshot); err != nil {
		t.Fatal(err)
	}
	wire, err := ReadStoreSnapshot(path)
	if err != nil || wire.Digest != snapshot.Digest {
		t.Fatalf("snapshot file: %v", err)
	}
	cfg := Config{Database: DatabaseConfig{DataDir: filepath.Join(t.TempDir(), "restored")}}
	if err = RestoreStoreSnapshot(ctx, cfg, wire); err != nil {
		t.Fatal(err)
	}
	restored, err := OpenStore(ctx, cfg)
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() { _ = restored.Close() })
	got, err := restored.ProviderPepClientAdmin(ctx, "grant", "backup-pep")
	if err != nil || got.AuthorizationMode != "user" || got.PolicyRevision != 2 || got.Lifecycle != "disabled" {
		t.Fatalf("user policy changed on restore: %+v %v", got, err)
	}
	again, err := restored.ExportSnapshot(ctx)
	if err != nil {
		t.Fatal(err)
	}
	for i, table := range snapshot.Tables {
		if table.Digest != again.Tables[i].Digest {
			t.Fatalf("authority table changed: %s", table.Name)
		}
	}
	rows, err := restored.Query(ctx, ReadStrong, "SELECT snapshot_digest FROM grant_restore_activation WHERE id='active'")
	if err != nil || len(rows) != 1 || rowString(rows[0], "snapshot_digest") != snapshot.Digest {
		t.Fatalf("source provenance changed: %v %v", rows, err)
	}
}

func TestPepPolicySnapshotRejectsInvalidPolicyAndRelabeledLegacy(t *testing.T) {
	ctx := context.Background()
	s, err := OpenStore(ctx, Config{Database: DatabaseConfig{DataDir: t.TempDir()}})
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() { _ = s.Close() })
	if err = s.EnsureBootstrapData(ctx); err != nil {
		t.Fatal(err)
	}
	if _, err = s.UpsertProviderPepClient(ctx, "grant", "snapshot-policy", "operator", UpsertProviderPepClient{Lifecycle: "active"}); err != nil {
		t.Fatal(err)
	}
	snapshot, err := s.ExportSnapshot(ctx)
	if err != nil {
		t.Fatal(err)
	}
	for _, tableName := range []string{"provider_pep_clients", "provider_pep_client_receipts"} {
		for _, invalid := range []struct{ mode, revision any }{
			{nil, int64(1)}, {"automatic", int64(1)}, {"user", nil}, {"user", int64(0)}, {"user", int64(-1)}, {"user", 1.5}, {"user", "2"},
		} {
			bad := pepSnapshotFixtureVersion(t, snapshot, canonicalSchemaDigest())
			for i := range bad.Tables {
				table := &bad.Tables[i]
				if table.Name != tableName {
					continue
				}
				row := table.Rows[0]
				row[len(row)-2], row[len(row)-1] = invalid.mode, invalid.revision
				table.Digest, err = tableDigest(*table)
				if err != nil {
					t.Fatal(err)
				}
			}
			bad.Digest, err = snapshotDigest(bad)
			if err != nil {
				t.Fatal(err)
			}
			if err = ValidateStoreSnapshot(bad); err == nil {
				t.Fatalf("invalid %s policy admitted: %+v", tableName, invalid)
			}
		}
	}
	for _, digest := range []string{legacySchemaDigestV5(), legacySchemaDigestV4(), legacySchemaDigestV3()} {
		bad := pepSnapshotFixtureVersion(t, snapshot, legacySchemaDigestV6())
		bad.SchemaDigest = digest
		bad.Digest, err = snapshotDigest(bad)
		if err != nil {
			t.Fatal(err)
		}
		if err = ValidateStoreSnapshot(bad); err == nil || !strings.Contains(err.Error(), "PEP columns mismatch") {
			t.Fatalf("v6 policy columns admitted under a legacy schema digest: %v", err)
		}
	}
}
