package grant

import (
	"context"
	"database/sql"
	"os"
	"path/filepath"
	"testing"
)

func TestLegacySQLiteExportRestoreKeepsOriginal(t *testing.T) {
	ctx := context.Background()
	root := t.TempDir()
	old := filepath.Join(root, "hiqlite.sqlite")
	db, e := sql.Open("sqlite3", old)
	if e != nil {
		t.Fatal(e)
	}
	if _, e = db.ExecContext(ctx, schemaSQL+indexesSQL); e != nil {
		t.Fatal(e)
	}
	if _, e = db.ExecContext(ctx, "INSERT INTO users(id,email,display_name,active,created_at) VALUES('legacy',NULL,'Legacy',0,'2026-09-08T00:00:00Z')"); e != nil {
		t.Fatal(e)
	}
	db.Close()
	before, e := os.ReadFile(old)
	if e != nil {
		t.Fatal(e)
	}
	snap, e := ExportLegacySQLite(ctx, old)
	if e != nil {
		t.Fatal(e)
	}
	out := filepath.Join(root, "snapshot.json")
	if e = WriteStoreSnapshot(out, snap); e != nil {
		t.Fatal(e)
	}
	loaded, e := ReadStoreSnapshot(out)
	if e != nil {
		t.Fatal(e)
	}
	cfg := Config{Database: DatabaseConfig{DataDir: filepath.Join(root, "rhiza")}}
	if e = RestoreStoreSnapshot(ctx, cfg, loaded); e != nil {
		t.Fatal(e)
	}
	after, _ := os.ReadFile(old)
	if string(before) != string(after) {
		t.Fatal("legacy database changed during read-only export")
	}
	s, e := OpenStore(ctx, cfg)
	if e != nil {
		t.Fatal(e)
	}
	defer s.Close()
	u, e := s.GetUserWithConsistency(ctx, "legacy", ReadStrong)
	if e != nil || u.Active || u.DisplayName != "Legacy" {
		t.Fatalf("restored identity=%+v err=%v", u, e)
	}
}

func TestLegacyExportRejectsDiscardedMaskEvidence(t *testing.T) {
	ctx := context.Background()
	path := filepath.Join(t.TempDir(), "legacy.sqlite")
	db, e := sql.Open("sqlite3", path)
	if e != nil {
		t.Fatal(e)
	}
	if _, e = db.ExecContext(ctx, schemaSQL+"CREATE TABLE mask_catalog_active(id TEXT); INSERT INTO mask_catalog_active VALUES('evidence');"); e != nil {
		t.Fatal(e)
	}
	db.Close()
	if _, e = ExportLegacySQLite(ctx, path); e == nil {
		t.Fatal("legacy mask evidence was silently discarded")
	}
}
func TestRestoreRejectsSharedTopologyBeforeFiles(t *testing.T) {
	ctx := context.Background()
	s, e := OpenStore(ctx, Config{Database: DatabaseConfig{DataDir: t.TempDir()}})
	if e != nil {
		t.Fatal(e)
	}
	defer s.Close()
	snap, e := s.ExportSnapshot(ctx)
	if e != nil {
		t.Fatal(e)
	}
	for _, mode := range []string{"remote", "split", "shared"} {
		t.Run(mode, func(t *testing.T) {
			dir := filepath.Join(t.TempDir(), "target")
			cfg := Config{Database: DatabaseConfig{DataDir: dir, RuntimeMode: mode}}
			if mode == "shared" {
				cfg.Database.RuntimeMode = "embedded"
				cfg.Database.Rhiza.ObjStoreBucket = "production"
			}
			if e = RestoreStoreSnapshot(ctx, cfg, snap); e == nil {
				t.Fatal("unsafe restore accepted")
			}
			if _, e = os.Stat(dir); !os.IsNotExist(e) {
				t.Fatalf("restore created files: %v", e)
			}
		})
	}
}
