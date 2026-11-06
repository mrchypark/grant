package grant

import (
	"context"
	"crypto/sha256"
	"encoding/hex"
	"encoding/json"
	"io"
	"os"
	"os/exec"
	"path/filepath"
	"testing"
	"time"

	"github.com/mrchypark/rhiza"
)

// CI supplies /usr/local/bin/grant extracted from the verified v0.9.0 image:
// sha256:da3a5c9c0193870b5c892cb5407f657cc19df88b31593dd7bb8e94b48445e70f
// OCI revision f0ab0e7684f279dc84f82f22c7537d3679780eb9.
// This synthetic Local retained-directory probe establishes neither production
// non-Local compatibility nor recovery after WAL loss, crash, or downgrade.
func TestUpgradeLocalRetainedV090(t *testing.T) {
	binary := os.Getenv("GRANT_UPGRADE_BASELINE_BINARY")
	if binary == "" {
		if os.Getenv("GRANT_UPGRADE_BASELINE_REQUIRED") == "1" {
			t.Fatal("required v0.9.0 baseline binary is missing")
		}
		t.Skip("UNVERIFIED: requires a CI-extracted, source-verified v0.9.0 baseline binary")
	}
	if !filepath.IsAbs(binary) {
		t.Fatal("baseline binary must be an absolute path")
	}
	file, err := os.Open(binary)
	if err != nil {
		t.Fatal(err)
	}
	hash := sha256.New()
	_, readErr := io.Copy(hash, file)
	closeErr := file.Close()
	expected := os.Getenv("GRANT_UPGRADE_BASELINE_SHA256")
	if readErr != nil || closeErr != nil || len(expected) != 64 || hex.EncodeToString(hash.Sum(nil)) != expected {
		t.Fatal("baseline binary checksum missing or mismatched")
	}
	ctx, cancel := context.WithTimeout(context.Background(), 2*time.Minute)
	defer cancel()
	snap := largeRestoreFrozenSource(t, ctx)
	if len(snap.Tables) != 31 {
		t.Fatalf("fixture tables=%d want=31", len(snap.Tables))
	}
	root := t.TempDir()
	snapshotPath := filepath.Join(root, "synthetic-snapshot.json")
	if err := WriteStoreSnapshot(snapshotPath, snap); err != nil {
		t.Fatal(err)
	}
	cfg := Config{Database: DatabaseConfig{
		DataDir: filepath.Join(root, "retained"), ClusterID: "offline", NodeID: "upgrade-local",
		RuntimeMode: "embedded", LocalNodeRole: "voter", DefaultReadConsistency: ReadStrong,
		Rhiza: rhiza.Config{Local: true, ClusterID: "offline", NodeID: "upgrade-local"},
	}}
	// Keep baseline input to fields supported by the published old executable;
	// do not serialize newly introduced candidate-native config fields into it.
	raw, err := json.Marshal(map[string]any{"Local": true, "ClusterID": "offline", "NodeID": "upgrade-local"})
	if err != nil {
		t.Fatal(err)
	}
	command := exec.CommandContext(ctx, binary, "migrate-restore", "--snapshot", snapshotPath, "--data-dir", cfg.Database.DataDir)
	command.WaitDelay = 2 * time.Second
	command.Dir = root
	// Do not inherit production credentials, RHIZA_* overrides, proxy settings,
	// or the caller's authentication provider configuration.
	command.Env = []string{
		"GRANT_DATA_DIR=" + cfg.Database.DataDir,
		"GRANT_CLUSTER_ID=offline", "GRANT_NODE_ID=upgrade-local", "GRANT_PEER_ADDR=",
		"GRANT_RUNTIME_MODE=embedded", "GRANT_LOCAL_NODE_ROLE=voter", "GRANT_READ_CONSISTENCY=strong",
		"GRANT_RHIZA_CONFIG_JSON=" + string(raw),
		"GRANT_BOOTSTRAP_USER_ID=conalog-grant", "GRANT_MASTER_KEY=synthetic-upgrade-master",
		"GRANT_APP_AUTH_KEY=synthetic-upgrade-app", "GRANT_AUTH_PROVIDER=rauthy",
		"GRANT_RAUTHY_CLIENT_ID=synthetic-client", "GRANT_RAUTHY_CLIENT_SECRET=synthetic-secret",
		"TMPDIR=" + root,
	}
	output, err := command.CombinedOutput()
	if err != nil {
		t.Fatalf("baseline import failed: %v\n%s", err, output)
	}
	if _, err := os.Lstat(filepath.Join(cfg.Database.DataDir, "grant-restore-pending.json")); !os.IsNotExist(err) {
		t.Fatal("baseline left a pending restore marker")
	}
	markerBytes, err := os.ReadFile(filepath.Join(cfg.Database.DataDir, "grant-cutover-verified.json"))
	if err != nil {
		t.Fatal(err)
	}
	var marker map[string]string
	if err := json.Unmarshal(markerBytes, &marker); err != nil || marker["sourceDigest"] != snap.Digest || marker["backend"] != "rhiza" || marker["clusterId"] != "offline" || marker["verifiedAt"] == "" {
		t.Fatal("baseline verified marker does not match the synthetic source")
	}
	retained, err := OpenStore(ctx, cfg)
	if err != nil {
		t.Fatalf("candidate failed to reopen old Local directory: %v", err)
	}
	t.Cleanup(func() {
		if retained != nil {
			if err := retained.Close(); err != nil {
				t.Error(err)
			}
		}
	})
	before, err := retained.ExportSnapshot(ctx)
	if err != nil {
		t.Fatal(err)
	}
	assertUpgradeTables(t, snap, before)
	if n, err := retained.Exec(ctx, "INSERT INTO users(id,display_name,active,created_at) VALUES('upgrade-new-write','Upgrade write',1,?)", nowString()); err != nil || n != 1 {
		t.Fatalf("candidate write affected=%d err=%v", n, err)
	}
	after, err := retained.ExportSnapshot(ctx)
	if err != nil {
		t.Fatal(err)
	}
	if err := retained.Close(); err != nil {
		t.Fatal(err)
	}
	retained = nil
	retained, err = OpenStore(ctx, cfg)
	if err != nil {
		t.Fatalf("candidate retained reopen failed: %v", err)
	}
	reopened, err := retained.ExportSnapshot(ctx)
	if err != nil {
		t.Fatal(err)
	}
	assertUpgradeTables(t, after, reopened)
	t.Log("PASS synthetic v0.9.0 Local native directory -> candidate retained reopen, 31-table/schema preservation, new write and reopen; no non-Local/cold-WAL claim")
}

func assertUpgradeTables(t *testing.T, want, got StoreSnapshot) {
	t.Helper()
	if want.SchemaDigest != got.SchemaDigest || len(want.Tables) != 31 || len(got.Tables) != 31 {
		t.Fatal("schema or canonical table count changed")
	}
	byName := make(map[string]SnapshotTable, len(got.Tables))
	for _, table := range got.Tables {
		byName[table.Name] = table
	}
	for _, table := range want.Tables {
		actual, ok := byName[table.Name]
		digest, err := restoredTableDigest(table, want.SchemaDigest)
		if !ok || err != nil || actual.Digest != digest {
			t.Fatalf("retained table %s changed", table.Name)
		}
	}
}
