package grant

import (
	"context"
	"encoding/json"
	"fmt"
	"os"
	"path/filepath"
	"strings"
	"sync"
	"testing"
)

func frozenV2(t *testing.T) AuthoritySnapshotV2 {
	t.Helper()
	var v AuthoritySnapshotV2
	// Frozen Rust crates/api/tests/patch_data_authority_v2.rs exporter fixture.
	raw := `{"schemaVersion":2,"sourceId":"patch-data","generation":1,"sourceRevision":"9a27580e7fd7e4bb0df63c1e2e8876e3a24c717d","sourceLedgerDigest":"sha256:` + strings.Repeat("a", 64) + `","sourceWatermark":"sha256:` + strings.Repeat("b", 64) + `","producedAt":"2026-08-26T00:00:00Z","principals":[],"organizations":[],"memberships":[],"applications":[],"effectiveBindings":[],"snapshotDigest":"sha256:868dff9d46dcb754ac82aa760837227bc4d0489505bd0063587122f26bc3ee30"}`
	if e := json.Unmarshal([]byte(raw), &v); e != nil {
		t.Fatal(e)
	}
	return v
}
func TestV2FrozenRustDigestAndGenerationAtomicity(t *testing.T) {
	ctx := context.Background()
	v := frozenV2(t)
	d, e := SnapshotDigest(v)
	if e != nil || d != v.SnapshotDigest {
		t.Fatalf("Rust oracle: %s %v", d, e)
	}
	s, e := OpenStore(ctx, Config{Database: DatabaseConfig{DataDir: t.TempDir()}})
	if e != nil {
		t.Fatal(e)
	}
	defer s.Close()
	var wg sync.WaitGroup
	results := make(chan AuthoritySnapshotV2Result, 2)
	errs := make(chan error, 2)
	for range 2 {
		wg.Go(func() { r, e := s.ImportAuthoritySnapshotV2(ctx, v); results <- r; errs <- e })
	}
	wg.Wait()
	close(results)
	close(errs)
	for e := range errs {
		if e != nil {
			t.Fatal(e)
		}
	}
	applied := 0
	for r := range results {
		if r.Applied {
			applied++
		}
	}
	if applied != 1 {
		t.Fatalf("applied %d", applied)
	}
	changed := v
	changed.SourceRevision = "changed"
	changed.SnapshotDigest, _ = SnapshotDigest(changed)
	if _, e = s.ImportAuthoritySnapshotV2(ctx, changed); e == nil {
		t.Fatal("conflicting generation accepted")
	}
	gap := v
	gap.Generation = 3
	gap.SnapshotDigest, _ = SnapshotDigest(gap)
	if _, e = s.ImportAuthoritySnapshotV2(ctx, gap); e == nil {
		t.Fatal("generation gap accepted")
	}
	rows, e := s.Query(ctx, ReadStrong, "SELECT generation FROM authority_snapshots_v2")
	if e != nil || len(rows) != 1 || rowInt(rows[0], "generation") != 1 {
		t.Fatalf("failed import changed rows %v %v", rows, e)
	}
	next := v
	next.Generation = 2
	next.SnapshotDigest, _ = SnapshotDigest(next)
	r, e := s.ImportAuthoritySnapshotV2(ctx, next)
	if e != nil || r.PriorGeneration == nil || *r.PriorGeneration != 1 {
		t.Fatalf("next=%+v %v", r, e)
	}
}
func TestCanonicalJSONUnicodeSeparators(t *testing.T) {
	// serde_json writes valid UTF-8 U+2028/U+2029 literally, preserving literal backslash text.
	got, e := canonicalJSON(map[string]string{"value": "a\u2028b\u2029c\\u2028"}, "")
	want := "{\"value\":\"a\u2028b\u2029c\\\\u2028\"}"
	if e != nil || got != want {
		t.Fatalf("got %q want %q err=%v", got, want, e)
	}
}

func TestV2NonemptyExporterGolden(t *testing.T) {
	b, e := os.ReadFile("../../cmd/grant-patch-data-authority-snapshot/testdata/snapshot-simple.json")
	if e != nil {
		t.Fatal(e)
	}
	var v AuthoritySnapshotV2
	if e = json.Unmarshal(b, &v); e != nil {
		t.Fatal(e)
	}
	// Fixed baseline Rust contract fixture, independently checked by the exporter CLI.
	v.SnapshotDigest = "sha256:d7bb40927cf19e06e515bfdd562092031865531575ead14c455d68c8d89fa06b"
	d, e := SnapshotDigest(v)
	if e != nil || d != v.SnapshotDigest {
		t.Fatalf("nonempty golden %s %v", d, e)
	}
	ctx := context.Background()
	s, e := OpenStore(ctx, Config{Database: DatabaseConfig{DataDir: t.TempDir()}})
	if e != nil {
		t.Fatal(e)
	}
	defer s.Close()
	r, e := s.ImportAuthoritySnapshotV2(ctx, v)
	if e != nil || !r.Applied || r.PrincipalCount != 1 || r.MembershipCount != 2 || r.EffectiveBindingCount != 1 {
		t.Fatalf("golden import %+v %v", r, e)
	}
	if r, e = s.ImportAuthoritySnapshotV2(ctx, v); e != nil || r.Applied {
		t.Fatalf("golden replay %+v %v", r, e)
	}
	changed := v
	changed.Principals = append([]AuthorityPrincipalV2(nil), v.Principals...)
	changed.SourceRevision = "changed"
	changed.SnapshotDigest, _ = SnapshotDigest(changed)
	if _, e = s.ImportAuthoritySnapshotV2(ctx, changed); e == nil {
		t.Fatal("nonempty conflicting replay accepted")
	}
	rows, e := s.Query(ctx, ReadStrong, "SELECT snapshot_digest FROM authority_snapshots_v2")
	if e != nil || len(rows) != 1 || rowString(rows[0], "snapshot_digest") != v.SnapshotDigest {
		t.Fatalf("failed conflict changed golden %v %v", rows, e)
	}
}

func TestV2LargeCanonicalImportBackupRestore(t *testing.T) {
	ctx := context.Background()
	v := frozenV2(t)
	for i := range 5500 {
		v.Principals = append(v.Principals, AuthorityPrincipalV2{ID: fmt.Sprintf("p%05d", i), Kind: "human", Active: true, ExternalSubject: fmt.Sprintf("subject-%05d-", i) + strings.Repeat("x", 190) + "雪\u2028\\\""})
	}
	v.SnapshotDigest, _ = SnapshotDigest(v)
	canonical, e := canonicalJSON(v, "")
	if e != nil {
		t.Fatal(e)
	}
	if len(canonical) < 1500000 || len(canonical) > 2<<20 {
		t.Fatalf("unexpected fixture size %d", len(canonical))
	}
	source, e := OpenStore(ctx, Config{Database: DatabaseConfig{DataDir: t.TempDir(), ClusterID: "source"}})
	if e != nil {
		t.Fatal(e)
	}
	defer source.Close()
	if _, e = source.ImportAuthoritySnapshotV2(ctx, v); e != nil {
		t.Fatal(e)
	}
	if r, e := source.ImportAuthoritySnapshotV2(ctx, v); e != nil || r.Applied {
		t.Fatalf("large replay %+v %v", r, e)
	}
	snap, e := source.ExportSnapshot(ctx)
	if e != nil {
		t.Fatal(e)
	}
	cfg := Config{Database: DatabaseConfig{DataDir: filepath.Join(t.TempDir(), "restored"), ClusterID: "target"}}
	if e = RestoreStoreSnapshot(ctx, cfg, snap); e != nil {
		t.Fatal(e)
	}
	target, e := OpenStore(ctx, cfg)
	if e != nil {
		t.Fatal(e)
	}
	defer target.Close()
	r, e := target.ImportAuthoritySnapshotV2(ctx, v)
	if e != nil || r.Applied {
		t.Fatalf("large restored replay %+v %v", r, e)
	}
}
