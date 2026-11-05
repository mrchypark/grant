package grant

import (
	"context"
	"net/http"
	"net/http/httptest"
	"os"
	"path/filepath"
	"strings"
	"testing"
	"time"

	"github.com/mrchypark/rhiza"
)

func singleVoterRestoreFixture() Config {
	const cluster = "fresh-import"
	const node = "import-voter"
	const token = "import-peer-credential"
	return Config{Database: DatabaseConfig{RuntimeMode: "embedded", LocalNodeRole: "voter", Rhiza: rhiza.Config{
		ClusterID:        cluster, NodeID: node, PeerAddr: "127.0.0.1:8100", PeerToken: token, AdminToken: "import-sync-credential",
		Members:          []rhiza.Member{{ID: node, PeerURL: "quic://127.0.0.1:8100", PublicKey: rhiza.PeerPublicKey(cluster, node, token)}},
		ObjStoreProvider: "gcs", ObjStoreBucket: "owned-import", ObjStorePrefix: "fresh/import", ObjStoreDurability: rhiza.ObjectStoreDurabilityBeforeAck,
	}}}
}

func TestRestoreGCSEmulatorRequiresLocalADC(t *testing.T) {
	t.Setenv("STORAGE_EMULATOR_HOST", "http://127.0.0.1:4588")
	t.Setenv("GOOGLE_APPLICATION_CREDENTIALS", filepath.Join(t.TempDir(), "absent"))
	c := singleVoterRestoreFixture().Database.Rhiza
	if err := restoreGCSEmulatorPreflight(c); err == nil {
		t.Fatal("emulator without disposable ADC accepted")
	}
	c.ObjStoreServiceAccount = `{"type":"service_account","token_uri":"https://oauth2.googleapis.com/token"}`
	if err := restoreGCSEmulatorPreflight(c); err == nil {
		t.Fatal("emulator credentials could contact a live token endpoint")
	}
	c.ObjStoreServiceAccount = `{"type":"service_account","token_uri":"http://127.0.0.1:4588/token"}`
	if err := restoreGCSEmulatorPreflight(c); err != nil {
		t.Fatal(err)
	}
	t.Setenv("STORAGE_EMULATOR_HOST", "https://storage.googleapis.com")
	if err := restoreGCSEmulatorPreflight(c); err == nil {
		t.Fatal("nonlocal emulator endpoint accepted")
	}
}

func TestRestoreNamespaceErrorsDoNotExposeSDKCredentials(t *testing.T) {
	server := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		http.Error(w, "https://signed.invalid/?secret=must-not-leak", http.StatusForbidden)
	}))
	defer server.Close()
	c := singleVoterRestoreFixture().Database.Rhiza
	c.ObjStoreProvider = "s3"
	c.ObjStoreEndpoint = strings.TrimPrefix(server.URL, "http://")
	c.ObjStoreInsecure = true
	c.ObjStoreRegion = "us-east-1"
	c.ObjStoreAccessKey, c.ObjStoreSecretKey = "fake", "must-not-leak"
	ctx, cancel := context.WithTimeout(context.Background(), time.Second)
	defer cancel()
	err := restoreNamespaceEmpty(ctx, c)
	if err == nil || strings.Contains(err.Error(), "signed.invalid") || strings.Contains(err.Error(), "must-not-leak") {
		t.Fatal("SDK failure was accepted or exposed credentials")
	}
}

func TestSingleVoterRestoreRejectsOccupiedDirectoryBeforeSDK(t *testing.T) {
	ctx := context.Background()
	snap := restoreFrozenSource(t, ctx)
	cfg := singleVoterRestoreFixture()
	cfg.Database.DataDir = t.TempDir()
	sentinel := filepath.Join(cfg.Database.DataDir, "keep")
	if err := os.WriteFile(sentinel, []byte("source evidence"), 0600); err != nil {
		t.Fatal(err)
	}
	if err := RestoreStoreSnapshot(ctx, cfg, snap); err == nil {
		t.Fatal("occupied target accepted")
	}
	if raw, err := os.ReadFile(sentinel); err != nil || string(raw) != "source evidence" {
		t.Fatal("occupied target was modified")
	}
	if _, err := os.Stat(filepath.Join(cfg.Database.DataDir, "grant-restore-pending.json")); !os.IsNotExist(err) {
		t.Fatal("occupied target gained a restore marker")
	}
}

func TestSingleVoterRestoreAdmission(t *testing.T) {
	snap := StoreSnapshot{Backend: "rhiza", ClusterID: "source-cluster"}
	for _, provider := range []string{"s3", "gcs", "azure"} {
		cfg := singleVoterRestoreFixture()
		cfg.Database.Rhiza.ObjStoreProvider = provider
		if _, err := singleVoterRestoreConfig(cfg, snap); err != nil {
			t.Fatalf("%s admission: %v", provider, err)
		}
	}
	for _, tc := range []struct {
		name   string
		change func(*Config)
	}{
		{"Local", func(c *Config) { c.Database.Rhiza.Local = true }},
		{"remote", func(c *Config) { c.Database.RuntimeMode = "remote" }},
		{"split", func(c *Config) { c.Database.RuntimeMode = "split" }},
		{"unknown mode", func(c *Config) { c.Database.RuntimeMode = "other" }},
		{"passive learner", func(c *Config) { c.Database.LocalNodeRole = "learner" }},
		{"remote nodes", func(c *Config) { c.Database.RemoteNodes = []string{"https://voter.invalid"} }},
		{"runtime listener", func(c *Config) { c.Database.RuntimeBindAddr = "127.0.0.1:8200" }},
		{"operator listener", func(c *Config) { c.Database.OperatorBindAddr = "127.0.0.1:8081" }},
		{"reconfiguration", func(c *Config) { c.Database.Rhiza.EnableReconfiguration = true }},
		{"cluster restore", func(c *Config) { c.Database.ClusterRestoreSnapshot = "snapshot.json" }},
		{"no explicit member", func(c *Config) { c.Database.Rhiza.Members = nil }},
		{"two voters", func(c *Config) { c.Database.Rhiza.Members = append(c.Database.Rhiza.Members, rhiza.Member{ID: "other"}) }},
		{"conflicting members", func(c *Config) { c.Database.Members = []rhiza.Member{{ID: "other"}} }},
		{"source cluster", func(c *Config) { c.Database.Rhiza.ClusterID = snap.ClusterID }},
		{"implicit cluster", func(c *Config) { c.Database.Rhiza.ClusterID = "" }},
		{"conflicting cluster", func(c *Config) { c.Database.ClusterID = "other" }},
		{"conflicting node", func(c *Config) { c.Database.NodeID = "other" }},
		{"wrong public key", func(c *Config) { c.Database.Rhiza.PeerToken = "different" }},
		{"missing peer token", func(c *Config) { c.Database.Rhiza.PeerToken = "" }},
		{"missing sync credential", func(c *Config) { c.Database.Rhiza.AdminToken = "" }},
		{"shared sync credential", func(c *Config) { c.Database.Rhiza.AdminToken = c.Database.Rhiza.PeerToken }},
		{"business credential", func(c *Config) { c.Security.MasterKey = c.Database.Rhiza.AdminToken }},
		{"wildcard peer", func(c *Config) { c.Database.Rhiza.Members[0].PeerURL = "quic://0.0.0.0:8100" }},
		{"async", func(c *Config) { c.Database.Rhiza.ObjStoreDurability = rhiza.ObjectStoreDurabilityAsync }},
		{"implicit durability", func(c *Config) { c.Database.Rhiza.ObjStoreDurability = "" }},
		{"empty bucket", func(c *Config) { c.Database.Rhiza.ObjStoreBucket = "" }},
		{"filesystem", func(c *Config) { c.Database.Rhiza.ObjStoreProvider = "filesystem" }},
		{"mixed directory", func(c *Config) { c.Database.Rhiza.ObjStoreDir = "/archive" }},
		{"empty prefix", func(c *Config) { c.Database.Rhiza.ObjStorePrefix = "" }},
		{"prefix normalization", func(c *Config) { c.Database.Rhiza.ObjStorePrefix = "fresh/../existing" }},
		{"prefix slash", func(c *Config) { c.Database.Rhiza.ObjStorePrefix = "/existing/" }},
	} {
		t.Run(tc.name, func(t *testing.T) {
			cfg := singleVoterRestoreFixture()
			tc.change(&cfg)
			if _, err := singleVoterRestoreConfig(cfg, snap); err == nil {
				t.Fatal("unsafe target accepted")
			}
		})
	}
	if _, err := singleVoterRestoreConfig(singleVoterRestoreFixture(), StoreSnapshot{Backend: "rhiza"}); err == nil {
		t.Fatal("unknown source cluster accepted")
	}
}
