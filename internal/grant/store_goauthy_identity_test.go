package grant

import (
	"context"
	"strings"
	"sync"
	"testing"

	"github.com/mrchypark/rhiza"
)

const testGoauthyIssuer = "https://id.ied.example.test"

func TestGoauthyIdentityMigrationPreservesV3RowsAndIndex(t *testing.T) {
	ctx := context.Background()
	dir := t.TempDir()
	db, err := rhiza.Open(ctx, rhiza.Config{DataDir: dir, NodeID: "grant-1", ClusterID: "grant"})
	if err != nil {
		t.Fatal(err)
	}
	legacy := []rhiza.Migration{}
	for i, group := range []struct{ name, sql string }{{"grant canonical tables", schemaSQL}, {"grant canonical indexes", indexesSQL}, {"grant restore activation", "CREATE TABLE IF NOT EXISTS grant_restore_activation (id TEXT PRIMARY KEY, snapshot_digest TEXT NOT NULL, cluster_id TEXT NOT NULL, activated_at TEXT NOT NULL)"}} {
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
	now := "2026-10-13T00:00:00Z"
	for _, statement := range []string{
		"INSERT INTO users(id,email,display_name,active,created_at) VALUES('pb-user','pb@example.test','PB',1,'" + now + "')",
		"INSERT INTO users(id,email,display_name,active,created_at) VALUES('rauthy-user','rauthy@example.test','Rauthy',1,'" + now + "')",
		"INSERT INTO user_identities(id,user_id,provider,issuer,collection_id,external_user_id,email_snapshot,display_name_snapshot,last_seen_at,created_at,updated_at) VALUES('pb-id','pb-user','pocketbase','https://pb.example.test','users','pb-sub','pb@example.test','PB','" + now + "','" + now + "','" + now + "')",
		"INSERT INTO user_identities(id,user_id,provider,issuer,collection_id,external_user_id,email_snapshot,display_name_snapshot,last_seen_at,created_at,updated_at) VALUES('rauthy-id','rauthy-user','rauthy','','','rauthy-sub','rauthy@example.test','Rauthy','" + now + "','" + now + "','" + now + "')",
	} {
		if _, err = db.Execute(ctx, rhiza.ExecuteRequest{RequestID: requestID(), SQL: statement}); err != nil {
			t.Fatal(err)
		}
	}
	if err = db.Close(); err != nil {
		t.Fatal(err)
	}

	s, err := OpenStore(ctx, Config{Database: DatabaseConfig{DataDir: dir}})
	if err != nil {
		t.Fatal(err)
	}
	defer s.Close()
	rows, err := s.Query(ctx, ReadStrong, "SELECT provider,issuer,collection_id,external_user_id FROM user_identities ORDER BY id")
	if err != nil || len(rows) != 2 || rowString(rows[0], "provider") != "pocketbase" || rowString(rows[0], "issuer") != "https://pb.example.test" || rowString(rows[1], "provider") != "rauthy" || rowString(rows[1], "issuer") != "" {
		t.Fatalf("v3 identities changed: %#v %v", rows, err)
	}
	indexes, err := s.Query(ctx, ReadStrong, "SELECT name FROM sqlite_schema WHERE type='index' AND name='idx_user_identities_provider_external'")
	if err != nil || len(indexes) != 1 {
		t.Fatalf("qualified identity index missing: %#v %v", indexes, err)
	}
	if err = s.Migrate(ctx); err != nil {
		t.Fatalf("v4 migration was not idempotent: %v", err)
	}
}

func TestGoauthyIdentityLinkingIsExplicitAndDoesNotRemap(t *testing.T) {
	ctx := context.Background()
	cfg := Config{Auth: AuthConfig{ProviderMode: "goauthy", Goauthy: &GoauthyConfig{Issuer: testGoauthyIssuer, ApplicationID: "ied-management"}}, Database: DatabaseConfig{DataDir: t.TempDir()}}
	s, err := OpenStore(ctx, cfg)
	if err != nil {
		t.Fatal(err)
	}
	defer s.Close()
	now := nowString()
	for _, row := range []struct {
		id, email string
		active    int
	}{{"265db51c7dcd71ff36cc94dc5c6beb53", "ied-owner@example.test", 1}, {"565f0afffb8faf001cffa27d6839fe56", "ied-denied@example.test", 1}, {"ied-inactive", "inactive@example.test", 0}} {
		if _, err = s.Exec(ctx, "INSERT INTO users(id,email,display_name,active,created_at) VALUES(?,?,?,?,?)", row.id, row.email, row.id, row.active, now); err != nil {
			t.Fatal(err)
		}
	}
	service := NewService(cfg, s)
	owner := GoauthyIdentityLinkInput{ExistingUserID: "265db51c7dcd71ff36cc94dc5c6beb53", Issuer: testGoauthyIssuer, Subject: "allocated-owner-sub", ExpectedEmail: "ied-owner@example.test"}
	if err = service.LinkGoauthyIdentityAdmin(ctx, owner); err != nil {
		t.Fatal(err)
	}
	if err = service.LinkGoauthyIdentityAdmin(ctx, owner); err != nil {
		t.Fatalf("idempotent link: %v", err)
	}
	if err = service.LinkGoauthyIdentityAdmin(ctx, GoauthyIdentityLinkInput{ExistingUserID: "565f0afffb8faf001cffa27d6839fe56", Issuer: testGoauthyIssuer, Subject: "allocated-owner-sub", ExpectedEmail: "ied-denied@example.test"}); ErrorStatus(err) != 409 {
		t.Fatalf("subject remap error=%v", err)
	}
	if err = service.LinkGoauthyIdentityAdmin(ctx, GoauthyIdentityLinkInput{ExistingUserID: "265db51c7dcd71ff36cc94dc5c6beb53", Issuer: testGoauthyIssuer, Subject: "wrong-email", ExpectedEmail: "ied-denied@example.test"}); ErrorStatus(err) != 409 {
		t.Fatalf("email mismatch error=%v", err)
	}
	if err = service.LinkGoauthyIdentityAdmin(ctx, GoauthyIdentityLinkInput{ExistingUserID: "ied-inactive", Issuer: testGoauthyIssuer, Subject: "inactive-sub", ExpectedEmail: "inactive@example.test"}); ErrorStatus(err) != 409 {
		t.Fatalf("inactive target error=%v", err)
	}
	if err = service.LinkGoauthyIdentityAdmin(ctx, GoauthyIdentityLinkInput{ExistingUserID: "265db51c7dcd71ff36cc94dc5c6beb53", Issuer: "https://other.example.test", Subject: "bad-issuer", ExpectedEmail: "ied-owner@example.test"}); ErrorStatus(err) != 422 {
		t.Fatalf("issuer mismatch error=%v", err)
	}
	created, err := s.EnsureExternalIdentity(ctx, ExternalIdentityMaterialization{Provider: "goauthy", Issuer: testGoauthyIssuer, ExternalUserID: "unlinked-same-email", Email: ptr("ied-owner@example.test"), DisplayName: "Unlinked"})
	if err != nil || created.ID == "265db51c7dcd71ff36cc94dc5c6beb53" {
		t.Fatalf("email adopted existing user: %#v %v", created, err)
	}

	var wg sync.WaitGroup
	errs := make(chan error, 2)
	for _, user := range []string{"265db51c7dcd71ff36cc94dc5c6beb53", "565f0afffb8faf001cffa27d6839fe56"} {
		wg.Add(1)
		go func(user string) {
			defer wg.Done()
			email := "ied-owner@example.test"
			if user == "565f0afffb8faf001cffa27d6839fe56" {
				email = "ied-denied@example.test"
			}
			errs <- service.LinkGoauthyIdentityAdmin(ctx, GoauthyIdentityLinkInput{ExistingUserID: user, Issuer: testGoauthyIssuer, Subject: "one-concurrent-subject", ExpectedEmail: email})
		}(user)
	}
	wg.Wait()
	close(errs)
	successes := 0
	for err := range errs {
		if err == nil {
			successes++
		} else if ErrorStatus(err) != 409 {
			t.Fatalf("concurrent link error=%v", err)
		}
	}
	if successes != 1 {
		t.Fatalf("concurrent subject links=%d", successes)
	}
}

func TestLegacyV3SnapshotRejectsGoauthyIdentityRows(t *testing.T) {
	ctx := context.Background()
	s, err := OpenStore(ctx, Config{Database: DatabaseConfig{DataDir: t.TempDir()}})
	if err != nil {
		t.Fatal(err)
	}
	defer s.Close()
	if _, err = s.EnsureExternalIdentity(ctx, ExternalIdentityMaterialization{Provider: "rauthy", ExternalUserID: "legacy-sub", DisplayName: "Legacy"}); err != nil {
		t.Fatal(err)
	}
	snapshot, err := s.ExportSnapshot(ctx)
	if err != nil {
		t.Fatal(err)
	}
	snapshot = pepSnapshotFixtureVersion(t, snapshot, legacySchemaDigestV3())
	if snapshot.Digest, err = snapshotDigest(snapshot); err != nil {
		t.Fatal(err)
	}
	if err = ValidateStoreSnapshot(snapshot); err != nil {
		t.Fatalf("frozen v3-compatible snapshot rejected: %v", err)
	}
	for i := range snapshot.Tables {
		if snapshot.Tables[i].Name != "user_identities" {
			continue
		}
		for column, name := range snapshot.Tables[i].Columns {
			if name == "provider" {
				snapshot.Tables[i].Rows[0][column] = "goauthy"
			}
		}
		snapshot.Tables[i].Digest, err = tableDigest(snapshot.Tables[i])
		if err != nil {
			t.Fatal(err)
		}
	}
	if snapshot.Digest, err = snapshotDigest(snapshot); err != nil {
		t.Fatal(err)
	}
	if err = ValidateStoreSnapshot(snapshot); err == nil {
		t.Fatal("legacy snapshot with an unqualified Goauthy row was accepted")
	}
}

func TestGoauthyApplicationContextsUseIssuerQualifiedSubject(t *testing.T) {
	ctx := context.Background()
	cfg := Config{Security: SecurityConfig{ManagementApplicationID: "ied-management"}, Auth: AuthConfig{ProviderMode: "goauthy", Goauthy: &GoauthyConfig{Issuer: testGoauthyIssuer, ApplicationID: "ied-management"}}, Database: DatabaseConfig{DataDir: t.TempDir()}}
	store, err := OpenStore(ctx, cfg)
	if err != nil {
		t.Fatal(err)
	}
	defer store.Close()
	if _, err = store.EnsureExternalIdentity(ctx, ExternalIdentityMaterialization{Provider: "goauthy", Issuer: testGoauthyIssuer, ExternalUserID: "context-sub", DisplayName: "Context"}); err != nil {
		t.Fatal(err)
	}
	now := nowString()
	for _, statement := range []Statement{
		{SQL: "INSERT INTO organizations(id,name,created_at,updated_at) VALUES('ied-org','IED',?,?)", Args: []any{now, now}},
		{SQL: "INSERT INTO applications(id,organization_id,slug,name,application_type,app_type,first_party,consent_policy,active,provisioning_status,created_by_user_id,created_at,updated_at) VALUES('ied-management','ied-org','ied','IED','confidential','server',1,'auto_approve',1,'ready','system',?,?)", Args: []any{now, now}},
		{SQL: "INSERT INTO service_providers(id,name,active,created_at,updated_at) VALUES('grant-management','Grant Management',1,?,?)", Args: []any{now, now}},
		{SQL: "INSERT INTO actions(id,provider_id,code,name,risk_level,visibility_granting,grantable_to_applications,created_at,updated_at) VALUES('grant-management.user.materialize','grant-management','user.materialize','Ensure','low',0,1,?,?)", Args: []any{now, now}},
		{SQL: "INSERT INTO application_actions(application_id,action_id) VALUES('ied-management','grant-management.user.materialize')"},
	} {
		if err = store.Transaction(ctx, []Statement{statement}); err != nil {
			t.Fatal(err)
		}
	}
	identity := ExternalIdentity{Provider: "goauthy", Issuer: testGoauthyIssuer, ExternalUserID: "context-sub", ClientID: "ied-management", Active: true}
	verified := context.WithValue(ctx, replicaIdentityKey{}, identity)
	service := NewService(cfg, store)
	decision, err := service.resolveApplicationAuthorizationContext(verified, "ignored", "ied-org")
	if err != nil || decision.ExternalSubject != "goauthy:https://id.ied.example.test/context-sub" || decision.ProviderAudience != "ied-management" {
		t.Fatalf("decision context=%+v err=%v", decision, err)
	}
	onboarding, err := service.resolveOnboardingContext(verified, "ignored", "grant-management.user.materialize")
	if err != nil || onboarding.ExternalSubject != decision.ExternalSubject || onboarding.ProviderAudience != "ied-management" {
		t.Fatalf("onboarding context=%+v err=%v", onboarding, err)
	}
}

func ptr(value string) *string { return &value }
