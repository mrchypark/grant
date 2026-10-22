package grant

import (
	"context"
	"fmt"
	"testing"
)

func TestClusterRestoreCanonicalAtomicMoreThan64Rows(t *testing.T) {
	ctx := context.Background()
	source, e := OpenStore(ctx, Config{Database: DatabaseConfig{DataDir: t.TempDir(), ClusterID: "source"}})
	if e != nil {
		t.Fatal(e)
	}
	defer source.Close()
	for i := range 100 {
		if _, e = source.Exec(ctx, "INSERT INTO users(id,display_name,created_at) VALUES(?,?,'now')", fmt.Sprint(i), fmt.Sprint(i)); e != nil {
			t.Fatal(e)
		}
	}
	snap, e := source.ExportSnapshot(ctx)
	if e != nil {
		t.Fatal(e)
	}
	target, e := OpenStore(ctx, Config{Database: DatabaseConfig{DataDir: t.TempDir(), ClusterID: "target"}})
	if e != nil {
		t.Fatal(e)
	}
	defer target.Close()
	if e = target.restoreCanonicalAtomic(ctx, snap, "target"); e != nil {
		t.Fatal(e)
	}
	if e = target.restoreCanonicalAtomic(ctx, snap, "target"); e != nil {
		t.Fatal(e)
	}
	rows, e := target.Query(ctx, ReadStrong, "SELECT COUNT(*) n FROM users")
	if e != nil || rowInt(rows[0], "n") != 100 {
		t.Fatalf("rows %v %v", rows, e)
	}
	dirty, e := OpenStore(ctx, Config{Database: DatabaseConfig{DataDir: t.TempDir()}})
	if e != nil {
		t.Fatal(e)
	}
	defer dirty.Close()
	if _, e = dirty.Exec(ctx, "INSERT INTO users(id,display_name,created_at) VALUES('existing','Existing','now')"); e != nil {
		t.Fatal(e)
	}
	if e = dirty.restoreCanonicalAtomic(ctx, snap, "target"); e == nil {
		t.Fatal("nonempty canonical target accepted")
	}
	rows, e = dirty.Query(ctx, ReadStrong, "SELECT id FROM users")
	if e != nil || len(rows) != 1 || rowString(rows[0], "id") != "existing" {
		t.Fatalf("failed restore changed canonical rows %v %v", rows, e)
	}
}
