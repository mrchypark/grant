package grant

import (
	"context"
	"testing"
)

func TestBootstrapOwnerAtomicPocketBaseIdempotency(t *testing.T) {
	ctx := context.Background()
	s, e := OpenStore(ctx, Config{Database: DatabaseConfig{DataDir: t.TempDir()}})
	if e != nil {
		t.Fatal(e)
	}
	defer s.Close()
	if e = s.EnsureBootstrapData(ctx); e != nil {
		t.Fatal(e)
	}
	ident := ExternalIdentityMaterialization{Provider: "pocketbase", Issuer: "https://auth.example.test", CollectionID: "users_id", ExternalUserID: "pb-user-1", DisplayName: "First"}
	cmd := OwnerBootstrapCommand{Identity: ident, OrganizationID: "org-1"}
	r1, e := s.BootstrapOwnerAtomic(ctx, cmd)
	if e != nil {
		t.Fatal("first bootstrap:", e)
	}
	r2, e := s.BootstrapOwnerAtomic(ctx, cmd)
	if e != nil {
		t.Fatal("second bootstrap:", e)
	}
	if r1.UserID != r2.UserID || r1.OrganizationID != r2.OrganizationID {
		t.Fatalf("idempotency mismatch: %+v vs %+v", r1, r2)
	}
	rows, e := s.Query(ctx, ReadStrong, "SELECT COUNT(*) AS n FROM users WHERE id=?", r1.UserID)
	if e != nil || rowInt(rows[0], "n") != 1 {
		t.Fatalf("user count: %v %v", rows, e)
	}
	rows, e = s.Query(ctx, ReadStrong, "SELECT membership_role FROM organization_memberships WHERE organization_id=? AND user_id=?", "org-1", r1.UserID)
	if e != nil || len(rows) == 0 || rowString(rows[0], "membership_role") != "owner" {
		t.Fatalf("owner membership missing: %v %v", rows, e)
	}
}

func TestBootstrapOwnerAtomicPocketBaseNoApplicationCreated(t *testing.T) {
	ctx := context.Background()
	s, e := OpenStore(ctx, Config{Database: DatabaseConfig{DataDir: t.TempDir()}})
	if e != nil {
		t.Fatal(e)
	}
	defer s.Close()
	if e = s.EnsureBootstrapData(ctx); e != nil {
		t.Fatal(e)
	}
	ident := ExternalIdentityMaterialization{Provider: "pocketbase", Issuer: "https://auth.example.test", CollectionID: "users_id", ExternalUserID: "pb-no-app", DisplayName: "NoApp"}
	r, e := s.BootstrapOwnerAtomic(ctx, OwnerBootstrapCommand{Identity: ident, OrganizationID: "org-no-app"})
	if e != nil {
		t.Fatal(e)
	}
	rows, e := s.Query(ctx, ReadStrong, "SELECT COUNT(*) AS n FROM applications WHERE organization_id=?", "org-no-app")
	if e != nil {
		t.Fatal(e)
	}
	if rowInt(rows[0], "n") != 0 {
		t.Fatalf("BootstrapOwnerAtomic must not create applications; found %d", rowInt(rows[0], "n"))
	}
	rows, e = s.Query(ctx, ReadStrong, "SELECT COUNT(*) AS n FROM user_identities WHERE provider='pocketbase' AND external_user_id='pb-no-app'")
	if e != nil || rowInt(rows[0], "n") != 1 {
		t.Fatalf("identity not created: %v %v", rows, e)
	}
	_ = r
}

func TestBootstrapOwnerAtomicPocketBaseInactiveUserDenied(t *testing.T) {
	ctx := context.Background()
	s, e := OpenStore(ctx, Config{Database: DatabaseConfig{DataDir: t.TempDir()}})
	if e != nil {
		t.Fatal(e)
	}
	defer s.Close()
	if e = s.EnsureBootstrapData(ctx); e != nil {
		t.Fatal(e)
	}
	ident := ExternalIdentityMaterialization{Provider: "pocketbase", Issuer: "https://auth.example.test", CollectionID: "users_id", ExternalUserID: "pb-active", DisplayName: "Active"}
	r, e := s.BootstrapOwnerAtomic(ctx, OwnerBootstrapCommand{Identity: ident, OrganizationID: "org-act"})
	if e != nil {
		t.Fatal(e)
	}
	_, e = s.Exec(ctx, "UPDATE users SET active=0 WHERE id=?", r.UserID)
	if e != nil {
		t.Fatal(e)
	}
	_, e = s.BootstrapOwnerAtomic(ctx, OwnerBootstrapCommand{Identity: ident, OrganizationID: "org-act"})
	if e == nil {
		t.Fatal("inactive user accepted re-bootstrap")
	}
}

func TestBootstrapOwnerAtomicRauthyLegacyBindingRefusesTakeover(t *testing.T) {
	ctx := context.Background()
	s, e := OpenStore(ctx, Config{Database: DatabaseConfig{DataDir: t.TempDir()}})
	if e != nil {
		t.Fatal(e)
	}
	defer s.Close()
	if e = s.EnsureBootstrapData(ctx); e != nil {
		t.Fatal(e)
	}
	now := nowString()
	_, e = s.Exec(ctx, "INSERT INTO users(id,email,display_name,active,created_at) VALUES('existing-user','e@test','Exist',1,?)", now)
	if e != nil {
		t.Fatal(e)
	}
	_, e = s.Exec(ctx, "INSERT INTO user_identities(id,user_id,provider,external_user_id,email_snapshot,display_name_snapshot,last_seen_at,created_at,updated_at) VALUES('ei1','existing-user','rauthy','taken-subject',NULL,NULL,?,?,?)", now, now, now)
	if e != nil {
		t.Fatal(e)
	}
	_, e = s.BindIdentityToUser(ctx, BindIdentityInput{UserID: "conalog-grant", Provider: "rauthy", ExternalUserID: "taken-subject", DisplayName: "Try", Active: true, AllowRebind: false})
	if e == nil {
		t.Fatal("AllowRebind=false must reject binding to already-bound identity")
	}
	_, e = s.BindIdentityToUser(ctx, BindIdentityInput{UserID: "conalog-grant", Provider: "rauthy", ExternalUserID: "taken-subject", DisplayName: "Try", Active: true, AllowRebind: true})
	if e != nil {
		t.Fatal("AllowRebind=true should succeed:", e)
	}
}

func TestBootstrapOwnerAtomicOrganizationMustNotExistOrBeActive(t *testing.T) {
	ctx := context.Background()
	s, e := OpenStore(ctx, Config{Database: DatabaseConfig{DataDir: t.TempDir()}})
	if e != nil {
		t.Fatal(e)
	}
	defer s.Close()
	if e = s.EnsureBootstrapData(ctx); e != nil {
		t.Fatal(e)
	}
	ident := ExternalIdentityMaterialization{Provider: "pocketbase", Issuer: "https://auth.example.test", CollectionID: "users_id", ExternalUserID: "pb-org-check", DisplayName: "OrgCheck"}
	r, e := s.BootstrapOwnerAtomic(ctx, OwnerBootstrapCommand{Identity: ident, OrganizationID: "org-inact"})
	if e != nil {
		t.Fatal(e)
	}
	_, e = s.Exec(ctx, "UPDATE organizations SET inactivated_at='2025-01-01T00:00:00Z' WHERE id='org-inact'")
	if e != nil {
		t.Fatal(e)
	}
	ident2 := ExternalIdentityMaterialization{Provider: "pocketbase", Issuer: "https://auth.example.test", CollectionID: "users_id", ExternalUserID: "pb-org-check-2", DisplayName: "OrgCheck2"}
	_, e = s.BootstrapOwnerAtomic(ctx, OwnerBootstrapCommand{Identity: ident2, OrganizationID: "org-inact"})
	if e == nil {
		t.Fatal("inactivated organization accepted new owner bootstrap")
	}
	_ = r
}

func TestBootstrapOwnerAtomicSingleOwnerPerOrg(t *testing.T) {
	ctx := context.Background()
	s, e := OpenStore(ctx, Config{Database: DatabaseConfig{DataDir: t.TempDir()}})
	if e != nil {
		t.Fatal(e)
	}
	defer s.Close()
	if e = s.EnsureBootstrapData(ctx); e != nil {
		t.Fatal(e)
	}
	ident1 := ExternalIdentityMaterialization{Provider: "pocketbase", Issuer: "https://auth.example.test", CollectionID: "users_id", ExternalUserID: "pb-own1", DisplayName: "Owner1"}
	_, e = s.BootstrapOwnerAtomic(ctx, OwnerBootstrapCommand{Identity: ident1, OrganizationID: "org-single"})
	if e != nil {
		t.Fatal(e)
	}
	ident2 := ExternalIdentityMaterialization{Provider: "pocketbase", Issuer: "https://auth.example.test", CollectionID: "users_id", ExternalUserID: "pb-own2", DisplayName: "Owner2"}
	_, e = s.BootstrapOwnerAtomic(ctx, OwnerBootstrapCommand{Identity: ident2, OrganizationID: "org-single"})
	if e == nil {
		t.Fatal("second owner accepted in same organization")
	}
}
