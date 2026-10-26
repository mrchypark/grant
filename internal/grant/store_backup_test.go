package grant

import (
	"context"
	"io"
	"net/http"
	"net/http/httptest"
	"os"
	"path/filepath"
	"strings"
	"testing"
)

func TestSnapshotRestoreAndReopen(t *testing.T) {
	ctx := context.Background()
	root := t.TempDir()
	cfg := Config{Database: DatabaseConfig{DataDir: filepath.Join(root, "source")}}
	s, e := OpenStore(ctx, cfg)
	if e != nil {
		t.Fatal(e)
	}
	if e = s.EnsureBootstrapData(ctx); e != nil {
		t.Fatal(e)
	}
	snap, e := s.ExportSnapshot(ctx)
	if e != nil {
		t.Fatal(e)
	}
	if e = s.Close(); e != nil {
		t.Fatal(e)
	}
	s, e = OpenStore(ctx, cfg)
	if e != nil {
		t.Fatal(e)
	}
	rows, e := s.Query(ctx, ReadStrong, "SELECT COUNT(*) AS n FROM users")
	if e != nil || rowInt(rows[0], "n") != 1 {
		t.Fatalf("reopen failed: %v %v", rows, e)
	}
	s.Close()
	cfg.Database.DataDir = filepath.Join(root, "restored")
	if e = RestoreStoreSnapshot(ctx, cfg, snap); e != nil {
		t.Fatal(e)
	}
	if _, e = os.Stat(filepath.Join(cfg.Database.DataDir, "grant-cutover-verified.json")); e != nil {
		t.Fatal(e)
	}
	s, e = OpenStore(ctx, cfg)
	if e != nil {
		t.Fatal(e)
	}
	defer s.Close()
	if e = s.VerifySchema(ctx); e != nil {
		t.Fatal(e)
	}
}

func TestPEPIdentitySnapshotCompatibility(t *testing.T) {
	ctx := context.Background()
	root := t.TempDir()
	s, err := OpenStore(ctx, Config{Database: DatabaseConfig{DataDir: filepath.Join(root, "source")}})
	if err != nil {
		t.Fatal(err)
	}
	defer s.Close()
	if _, err = s.Exec(ctx, "INSERT INTO service_providers(id,name,created_at,updated_at) VALUES('snapshot-provider','Snapshot',?,?)", nowString(), nowString()); err != nil {
		t.Fatal(err)
	}
	if _, err = s.UpsertProviderPepClient(ctx, "snapshot-provider", "snapshot-pep", "operator", UpsertProviderPepClient{Lifecycle: "disabled"}); err != nil {
		t.Fatal(err)
	}
	snapshot, err := s.ExportSnapshot(ctx)
	if err != nil {
		t.Fatal(err)
	}
	if snapshot.SchemaDigest != canonicalSchemaDigest() || snapshot.SchemaDigest == legacySchemaDigestV4() {
		t.Fatal("export does not identify the v5 schema")
	}
	for _, table := range snapshot.Tables {
		if table.Name == "applications" && table.Count != 0 {
			t.Fatal("PEP snapshot unexpectedly requires an application")
		}
	}
	// The previous digest also labeled early v5 exports. Both those exports
	// and actual v4 rows retain the same column layout and fit v5 constraints.
	for _, version := range []struct{ name, digest string }{{"v5", canonicalSchemaDigest()}, {"v4", legacySchemaDigestV4()}} {
		t.Run(version.name, func(t *testing.T) {
			compatible := snapshot
			compatible.SchemaDigest = version.digest
			compatible.Digest, err = snapshotDigest(compatible)
			if err != nil {
				t.Fatal(err)
			}
			cfg := Config{Database: DatabaseConfig{DataDir: filepath.Join(root, version.name)}}
			if err = RestoreStoreSnapshot(ctx, cfg, compatible); err != nil {
				t.Fatal(err)
			}
			restored, err := OpenStore(ctx, cfg)
			if err != nil {
				t.Fatal(err)
			}
			defer restored.Close()
			pep, err := restored.ProviderPepClient(ctx, "snapshot-provider", "snapshot-pep")
			if err != nil || pep.Lifecycle != "disabled" || pep.ApplicationID != "" {
				t.Fatalf("independent PEP changed on restore: %#v %v", pep, err)
			}
		})
	}
	for _, digest := range []string{canonicalSchemaDigest(), legacySchemaDigestV4(), legacySchemaDigestV3()} {
		for _, name := range []string{"provider_pep_clients", "provider_pep_client_receipts"} {
			malformed := snapshot
			malformed.SchemaDigest = digest
			malformed.Tables = append([]SnapshotTable(nil), snapshot.Tables...)
			for i := range malformed.Tables {
				table := &malformed.Tables[i]
				if table.Name != name {
					continue
				}
				table.Columns = append([]string(nil), table.Columns...)
				table.Columns[0] = "unexpected_column"
				table.Digest, err = tableDigest(*table)
				if err != nil {
					t.Fatal(err)
				}
			}
			malformed.Digest, err = snapshotDigest(malformed)
			if err != nil {
				t.Fatal(err)
			}
			if err = ValidateStoreSnapshot(malformed); err == nil || !strings.Contains(err.Error(), "PEP columns mismatch") {
				t.Fatalf("wrong %s columns admitted for %s: %v", name, digest, err)
			}
		}
	}
	snapshot.SchemaDigest = "unknown-schema"
	snapshot.Digest, err = snapshotDigest(snapshot)
	if err != nil {
		t.Fatal(err)
	}
	if err = ValidateStoreSnapshot(snapshot); err == nil {
		t.Fatal("unknown schema admitted")
	}
}

func TestPrivateRuntimeAuthorizationAndStrongFailure(t *testing.T) {
	ctx := context.Background()
	secret := strings.Repeat("r", 40)
	s, e := OpenStore(ctx, Config{Database: DatabaseConfig{DataDir: t.TempDir(), RemoteAPISecret: secret}})
	if e != nil {
		t.Fatal(e)
	}
	defer s.Close()
	server := httptest.NewServer(s.RuntimeHandler())
	defer server.Close()
	res, e := http.Post(server.URL+"/sql/query", "application/json", strings.NewReader(`{"sql":"SELECT 1","consistency":"linearizable"}`))
	if e != nil {
		t.Fatal(e)
	}
	res.Body.Close()
	if res.StatusCode != 401 {
		t.Fatalf("unauthenticated status=%d", res.StatusCode)
	}
	remote := &Store{cfg: Config{Database: DatabaseConfig{RemoteNodes: []string{server.URL}, RemoteAPISecret: secret, RuntimeMode: "remote"}}}
	r, e := remote.Query(ctx, ReadStrong, "SELECT ? AS n", int64(9007199254740993))
	if e != nil || rowInt(r[0], "n") != 9007199254740993 {
		t.Fatalf("integer precision: %v %v", r, e)
	}
	req, _ := http.NewRequest("POST", server.URL+"/sql/query", strings.NewReader(`{"sql":"SELECT 1","consistency":"typo"}`))
	req.Header.Set("Authorization", "Bearer "+secret)
	res, e = http.DefaultClient.Do(req)
	if e != nil {
		t.Fatal(e)
	}
	io.Copy(io.Discard, res.Body)
	res.Body.Close()
	if res.StatusCode == 200 {
		t.Fatal("invalid consistency allowed")
	}
	server.Close()
	if _, e = remote.Query(ctx, ReadStrong, "SELECT 1"); e == nil {
		t.Fatal("strong failure hidden")
	}
}
func TestPrivateRuntimeRejectsTrailingAndDuplicateBeforeMutation(t *testing.T) {
	ctx := context.Background()
	secret := strings.Repeat("s", 40)
	s, e := OpenStore(ctx, Config{Database: DatabaseConfig{DataDir: t.TempDir(), RemoteAPISecret: secret}})
	if e != nil {
		t.Fatal(e)
	}
	defer s.Close()
	h := s.RuntimeHandler()
	for _, payload := range []string{`{"request_id":"trailing","sql":"INSERT INTO users(id,display_name,created_at) VALUES('bad','bad','now')"} {}`, `{"request_id":"first","request_id":"second","sql":"INSERT INTO users(id,display_name,created_at) VALUES('bad','bad','now')"}`} {
		req := httptest.NewRequest("POST", "/sql/execute", strings.NewReader(payload))
		req.Header.Set("Authorization", "Bearer "+secret)
		w := httptest.NewRecorder()
		h.ServeHTTP(w, req)
		if w.Code == 200 {
			t.Fatal("invalid JSON executed")
		}
	}
	r, e := s.Query(ctx, ReadStrong, "SELECT id FROM users WHERE id='bad'")
	if e != nil || len(r) > 0 {
		t.Fatalf("invalid request changed rows: %v %v", r, e)
	}
}
func TestLegacyDetectionAndInterruptedRestoreBlock(t *testing.T) {
	ctx := context.Background()
	dir := t.TempDir()
	if e := os.Mkdir(filepath.Join(dir, "state_machine"), 0700); e != nil {
		t.Fatal(e)
	}
	if _, e := OpenStore(ctx, Config{Database: DatabaseConfig{DataDir: dir}}); e == nil {
		t.Fatal("legacy directory accepted")
	}
	entries, _ := os.ReadDir(dir)
	if len(entries) != 1 {
		t.Fatal("created files in legacy directory")
	}
	pending := t.TempDir()
	os.WriteFile(filepath.Join(pending, "grant-restore-pending.json"), []byte("pending"), 0600)
	if _, e := OpenStore(ctx, Config{Database: DatabaseConfig{DataDir: pending}}); e == nil {
		t.Fatal("partial restore accepted")
	}
}
