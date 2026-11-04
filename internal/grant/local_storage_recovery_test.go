package grant

import (
	"context"
	"os"
	"path/filepath"
	"slices"
	"testing"

	"github.com/mrchypark/rhiza"
)

func recoveryConfig(t *testing.T) Config {
	t.Helper()
	return Config{Database: DatabaseConfig{DataDir: filepath.Join(t.TempDir(), "data"), Rhiza: rhiza.Config{Local: true, ClusterID: "recovery", NodeID: "recovery-1", ObjStoreProvider: rhiza.ObjectStoreProviderFilesystem, ObjStoreDir: t.TempDir(), ObjStoreDurability: rhiza.ObjectStoreDurabilityBeforeAck}}}
}

func TestRestoreLocalStorageNoGrantSQL(t *testing.T) {
	for _, durability := range []rhiza.ObjectStoreDurability{rhiza.ObjectStoreDurabilityBeforeAck, rhiza.ObjectStoreDurabilityAsync} {
		t.Run(string(durability), func(t *testing.T) { testRestoreLocalStorageNoGrantSQL(t, durability) })
	}
}

func testRestoreLocalStorageNoGrantSQL(t *testing.T, durability rhiza.ObjectStoreDurability) {
	cfg := recoveryConfig(t)
	cfg.Database.Rhiza.ObjStoreDurability = durability
	// Grant's directory overrides a conflicting SDK directory in both opens.
	cfg.Database.Rhiza.DataDir = filepath.Join(t.TempDir(), "unused")
	ctx := context.Background()
	if err := RestoreLocalStorage(ctx, cfg); err != nil {
		t.Fatal(err)
	}
	if _, err := os.Stat(cfg.Database.Rhiza.DataDir); !os.IsNotExist(err) {
		t.Fatalf("SDK directory used: %v", err)
	}
	c, err := effectiveRhizaConfig(cfg.Database)
	if err != nil {
		t.Fatal(err)
	}
	db, err := rhiza.Open(ctx, c)
	if err != nil {
		t.Fatal(err)
	}
	defer func() {
		if err := db.Close(); err != nil {
			t.Error(err)
		}
	}()
	rows, err := db.Query(ctx, rhiza.QueryRequest{SQL: "SELECT name FROM sqlite_master WHERE type='table'"})
	if err != nil {
		t.Fatal(err)
	}
	appTables := append(applicationTables(), "grant_import_payloads", "grant_restore_activation")
	for _, row := range rows.Rows {
		if name, ok := row[0].(string); ok && slices.Contains(appTables, name) {
			t.Fatalf("Grant SQL initialized: %s", name)
		}
	}
}

func TestRestoreLocalStorageRejectsBeforeOpen(t *testing.T) {
	for _, tc := range []struct {
		name   string
		change func(*Config)
	}{
		{"not-local", func(c *Config) { c.Database.Rhiza.Local = false }},
		{"default-durability", func(c *Config) { c.Database.Rhiza.ObjStoreDurability = "" }},
		{"invalid-durability", func(c *Config) { c.Database.Rhiza.ObjStoreDurability = "sync" }},
		{"no-store", func(c *Config) { c.Database.Rhiza.ObjStoreProvider = "" }},
		{"remote", func(c *Config) { c.Database.RuntimeMode = "remote" }},
		{"split", func(c *Config) { c.Database.RuntimeMode = "split" }},
		{"learner", func(c *Config) { c.Database.LocalNodeRole = "learner" }},
		{"remote-nodes", func(c *Config) { c.Database.RemoteNodes = []string{"http://unused"} }},
		{"operator", func(c *Config) { c.Database.OperatorBindAddr = "127.0.0.1:0" }},
		{"sdk-operator", func(c *Config) { c.Database.Rhiza.BindAddr = "127.0.0.1:0" }},
		{"cluster-restore", func(c *Config) { c.Database.ClusterRestoreSnapshot = "unused" }},
		{"ha", func(c *Config) { c.Database.Members = []rhiza.Member{{}, {}} }},
		{"one-member", func(c *Config) { c.Database.Rhiza.Members = []rhiza.Member{{}} }},
		{"peer-address", func(c *Config) { c.Database.PeerAddr = "127.0.0.1:0" }},
		{"peer-token", func(c *Config) { c.Database.Rhiza.PeerToken = "unused" }},
		{"admin-token", func(c *Config) { c.Database.Rhiza.AdminToken = "unused" }},
		{"reconfiguration", func(c *Config) { c.Database.Rhiza.EnableReconfiguration = true }},
		{"native-learner", func(c *Config) { c.Database.Rhiza.Learner = &rhiza.Member{} }},
		{"missing-directory", func(c *Config) { c.Database.DataDir = "" }},
	} {
		t.Run(tc.name, func(t *testing.T) {
			cfg := recoveryConfig(t)
			tc.change(&cfg)
			if err := RestoreLocalStorage(context.Background(), cfg); err == nil {
				t.Fatal("accepted invalid configuration")
			}
			if _, err := os.Stat(cfg.Database.DataDir); !os.IsNotExist(err) {
				t.Fatalf("created database: %v", err)
			}
			entries, err := os.ReadDir(cfg.Database.Rhiza.ObjStoreDir)
			if err != nil || len(entries) != 0 {
				t.Fatalf("touched archive: %v %v", entries, err)
			}
		})
	}
}

func TestRestoreLocalStoragePreservesEvidence(t *testing.T) {
	for _, name := range []string{"state_machine", "raft", "raft_log", "grant-restore-pending.json"} {
		t.Run(name, func(t *testing.T) {
			cfg := recoveryConfig(t)
			if err := os.MkdirAll(cfg.Database.DataDir, 0700); err != nil {
				t.Fatal(err)
			}
			path := filepath.Join(cfg.Database.DataDir, name)
			if err := os.WriteFile(path, []byte("evidence"), 0600); err != nil {
				t.Fatal(err)
			}
			if err := RestoreLocalStorage(context.Background(), cfg); err == nil {
				t.Fatal("accepted evidence")
			}
			entries, err := os.ReadDir(cfg.Database.DataDir)
			if err != nil || len(entries) != 1 {
				t.Fatalf("changed directory: %v %v", entries, err)
			}
			got, err := os.ReadFile(path)
			if err != nil || string(got) != "evidence" {
				t.Fatalf("changed evidence: %s %v", got, err)
			}
		})
	}
}

func TestRestoreLocalStorageCancelled(t *testing.T) {
	cfg := recoveryConfig(t)
	ctx, cancel := context.WithCancel(context.Background())
	cancel()
	if err := RestoreLocalStorage(ctx, cfg); err == nil {
		t.Fatal("ignored cancellation")
	}
	if _, err := os.Stat(cfg.Database.DataDir); !os.IsNotExist(err) {
		t.Fatalf("created database: %v", err)
	}
}
