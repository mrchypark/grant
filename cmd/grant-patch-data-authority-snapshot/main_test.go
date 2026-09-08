package main

import (
	"database/sql"
	"encoding/json"
	"github.com/Conalog/grant/internal/grant"
	"os"
	"path/filepath"
	"strings"
	"testing"
)

func TestImmutablePublicationNeverReplaces(t *testing.T) {
	root := t.TempDir()
	target := filepath.Join(root, "bundle")
	if e := os.Mkdir(target, 0700); e != nil {
		t.Fatal(e)
	}
	if e := persistBundle(target, []byte("snapshot"), []byte("receipt")); e == nil {
		t.Fatal("replaced existing target")
	}
	entries, e := os.ReadDir(target)
	if e != nil || len(entries) != 0 {
		t.Fatal(entries, e)
	}
}
func TestArgumentAndFrozenFileValidation(t *testing.T) {
	base := []string{"--database", filepath.Join(t.TempDir(), "db"), "--source-revision", "r1", "--source-ledger-digest", "sha256:" + strings.Repeat("a", 64), "--generation", "1", "--produced-at", "2026-09-08T00:00:00Z", "--output-dir", filepath.Join(t.TempDir(), "bundle")}
	if _, e := parseArgs(base); e != nil {
		t.Fatal(e)
	}
	for _, extra := range [][]string{{"--generation", "2"}, {"--unknown", "x"}, {"--database"}} {
		if _, e := parseArgs(append(append([]string{}, base...), extra...)); e == nil {
			t.Fatal("accepted", extra)
		}
	}
	if e := os.WriteFile(base[1], []byte{}, 0600); e != nil {
		t.Fatal(e)
	}
	if e := os.WriteFile(base[1]+"-wal", []byte{}, 0600); e != nil {
		t.Fatal(e)
	}
	if e := run(base); e == nil || !strings.Contains(e.Error(), "frozen checkpoint") {
		t.Fatal(e)
	}
}
func TestSQLiteSnapshotExport(t *testing.T) {
	root := t.TempDir()
	path := filepath.Join(root, "source.db")
	db, e := sql.Open("sqlite3", path)
	if e != nil {
		t.Fatal(e)
	}
	for _, q := range []string{`CREATE TABLE organizations(id TEXT,owner TEXT)`, `CREATE TABLE users(id TEXT,deleted TEXT,organizations TEXT)`, `CREATE TABLE viewers(id TEXT,deleted TEXT,organizations TEXT)`, `CREATE TABLE temporary_accounts(id TEXT,expired TEXT,organization TEXT)`, `CREATE TABLE service_principals(id TEXT,enabled INTEGER)`, `CREATE TABLE org_parent_map(organization TEXT,parent TEXT)`, `CREATE TABLE plants(id TEXT,organization TEXT,isArchive INTEGER)`, `CREATE TABLE permissions(plant TEXT,users TEXT,viewers TEXT,temporary TEXT)`, `INSERT INTO organizations VALUES('o','u')`, `INSERT INTO users VALUES('u','','["o"]')`, `INSERT INTO plants VALUES('p','o',0)`} {
		if _, e = db.Exec(q); e != nil {
			t.Fatal(e)
		}
	}
	if e = db.Close(); e != nil {
		t.Fatal(e)
	}
	target := filepath.Join(root, "bundle")
	e = run([]string{"--database", path, "--source-revision", "r1", "--source-ledger-digest", "sha256:" + strings.Repeat("a", 64), "--generation", "1", "--produced-at", "2026-09-08T00:00:00Z", "--output-dir", target})
	if e != nil {
		t.Fatal(e)
	}
	raw, e := os.ReadFile(filepath.Join(target, "snapshot.json"))
	if e != nil {
		t.Fatal(e)
	}
	var v grant.AuthoritySnapshotV2
	if e = json.Unmarshal(raw, &v); e != nil {
		t.Fatal(e)
	}
	if len(v.Principals) != 1 || len(v.Memberships) != 2 || len(v.EffectiveBindings) != 1 {
		t.Fatalf("%+v", v)
	}
	// Independently framed source rows and sorted JSON from the frozen Rust
	// contract; see testdata/README.md. Do not derive these from Go helpers.
	if v.SourceWatermark != "sha256:f3c71c19fa99588541db89a6aac38912c8876e1c6987c96e020b7e72069b506f" || v.SnapshotDigest != "sha256:d7bb40927cf19e06e515bfdd562092031865531575ead14c455d68c8d89fa06b" {
		t.Fatalf("canonical contract mismatch: watermark=%s digest=%s", v.SourceWatermark, v.SnapshotDigest)
	}
	digest, e := grant.SnapshotDigest(v)
	if e != nil || digest != v.SnapshotDigest {
		t.Fatal(digest, e)
	}
}
