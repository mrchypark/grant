package grant

import (
	"context"
	"fmt"
	"strings"
	"testing"
)

func TestAuthorityImportMoreThan64EntitiesAtomic(t *testing.T) {
	ctx := context.Background()
	s, e := OpenStore(ctx, Config{Database: DatabaseConfig{DataDir: t.TempDir()}})
	if e != nil {
		t.Fatal(e)
	}
	defer s.Close()
	c := PatchDataAuthorityImport{ManifestVersion: "grant.patch-data-authority.v1", Generation: "g1", LedgerDigest: strings.Repeat("a", 64), Organizations: []PatchDataImportedOrganization{{OrganizationID: "org", Name: "Org", Active: true}}}
	for i := 0; i < 100; i++ {
		id := fmt.Sprintf("u%03d", i)
		c.Users = append(c.Users, PatchDataImportedUser{UserID: id, ExternalUserID: id, DisplayName: id, Active: true})
		c.Memberships = append(c.Memberships, PatchDataImportedMembership{MembershipID: id, OrganizationID: "org", UserID: id, Role: "member", Status: "active"})
	}
	c.ManifestDigest = PatchDataManifestDigest(c)
	if _, e = s.ImportPatchDataAuthority(ctx, c); e != nil {
		t.Fatal(e)
	}
	r, e := s.Query(ctx, ReadStrong, "SELECT COUNT(*) AS n FROM users")
	if e != nil || rowInt(r[0], "n") != 100 {
		t.Fatalf("rows=%v err=%v", r, e)
	}
	_, e = s.Exec(ctx, "INSERT INTO users(id,display_name,created_at) VALUES('native','Native','now')")
	if e != nil {
		t.Fatal(e)
	}
	g := c.Generation
	c.Generation = "g2"
	c.ExpectedPreviousGeneration = &g
	c.Users = append([]PatchDataImportedUser{{UserID: "native", ExternalUserID: "native", DisplayName: "Overwrite", Active: true}}, c.Users...)
	c.ManifestDigest = PatchDataManifestDigest(c)
	if _, e = s.ImportPatchDataAuthority(ctx, c); e == nil {
		t.Fatal("native collision must roll back")
	}
	r, e = s.Query(ctx, ReadStrong, "SELECT generation FROM patch_data_authority_imports")
	if e != nil || rowString(r[0], "generation") != "g1" {
		t.Fatalf("receipt changed on rejected import: %v %v", r, e)
	}
	r, e = s.Query(ctx, ReadStrong, "SELECT COUNT(*) AS n FROM organization_memberships")
	if e != nil || rowInt(r[0], "n") != 100 {
		t.Fatalf("memberships lost: %v %v", r, e)
	}
}

func TestStagingRetryImmutabilityAndReplayDrift(t *testing.T) {
	ctx := context.Background()
	s, e := OpenStore(ctx, Config{Database: DatabaseConfig{DataDir: t.TempDir()}})
	if e != nil {
		t.Fatal(e)
	}
	defer s.Close()
	c := PatchDataAuthorityImport{ManifestVersion: "grant.patch-data-authority.v1", Generation: "null", LedgerDigest: strings.Repeat("a", 64), Users: []PatchDataImportedUser{{UserID: "user", ExternalUserID: "subject", DisplayName: "User", Active: true}}}
	c.ManifestDigest = PatchDataManifestDigest(c)
	if e = s.stageImport(ctx, "stage", c, nil); e != nil {
		t.Fatal(e)
	}
	if e = s.stageImport(ctx, "stage", c, nil); e != nil {
		t.Fatal(e)
	}
	changed := c
	changed.Users = append([]PatchDataImportedUser(nil), c.Users...)
	changed.Users[0].DisplayName = "changed"
	if e = s.stageImport(ctx, "stage", changed, nil); e == nil {
		t.Fatal("immutable staging payload overwritten")
	}
	r, e := s.Query(ctx, ReadStrong, "SELECT COUNT(*) n FROM users")
	if e != nil || rowInt(r[0], "n") != 0 {
		t.Fatal("staging affected canonical users")
	}
	if _, e = s.ImportPatchDataAuthority(ctx, c); e != nil {
		t.Fatal(e)
	}
	if r, e := s.ImportPatchDataAuthority(ctx, c); e != nil || r.Applied {
		t.Fatalf("retry=%+v %v", r, e)
	}
	if _, e = s.Exec(ctx, "UPDATE users SET active=0 WHERE id='user'"); e != nil {
		t.Fatal(e)
	}
	if _, e = s.ImportPatchDataAuthority(ctx, c); e == nil {
		t.Fatal("replay accepted materialized state drift")
	}
}
