package main

import (
	"context"
	"encoding/json"
	"errors"
	"os"
	"os/exec"
	"path/filepath"
	"strings"
	"testing"
	"time"

	"github.com/mrchypark/grant/internal/grant"
	"github.com/mrchypark/rhiza"
)

// Run main itself in a child process so os.Exit and the pre-listener dispatch are tested.
func TestRestoreLocalStorageEntrypoint(t *testing.T) {
	if os.Getenv("GRANT_RECOVERY_TEST_CHILD") == "1" {
		os.Args = []string{"grant", "restore-local-storage"}
		if os.Getenv("GRANT_RECOVERY_TEST_EXTRA") == "1" {
			os.Args = append(os.Args, "--unexpected")
		}
		main()
		return
	}
	for _, mode := range []string{"success", "invalid", "extra", "open-error"} {
		t.Run(mode, func(t *testing.T) {
			root := filepath.Join(t.TempDir(), "data")
			archive := t.TempDir()
			r := rhiza.Config{Local: true, ClusterID: "cli", NodeID: "cli-1", ObjStoreProvider: rhiza.ObjectStoreProviderFilesystem, ObjStoreDir: archive, ObjStoreDurability: rhiza.ObjectStoreDurabilityBeforeAck, ObjStoreSecretKey: "test-secret-must-not-appear"}
			if mode == "invalid" {
				r.ObjStoreDurability = ""
			}
			if mode == "open-error" {
				if err := os.WriteFile(root, []byte("blocked"), 0600); err != nil {
					t.Fatal(err)
				}
			}
			raw, err := json.Marshal(r)
			if err != nil {
				t.Fatal(err)
			}
			ctx, cancel := context.WithTimeout(context.Background(), 30*time.Second)
			defer cancel()
			cmd := exec.CommandContext(ctx, os.Args[0], "-test.run=^TestRestoreLocalStorageEntrypoint$")
			// Isolate Grant and Rhiza configuration from the invoking session.
			for _, v := range os.Environ() {
				key, _, _ := strings.Cut(v, "=")
				if !strings.HasPrefix(key, "GRANT_") && !strings.HasPrefix(key, "RHIZA_") && !strings.HasPrefix(key, "HQL_") {
					cmd.Env = append(cmd.Env, v)
				}
			}
			cmd.Env = append(cmd.Env, "GRANT_RECOVERY_TEST_CHILD=1", "GRANT_DATA_DIR="+root, "GRANT_PEER_ADDR=", "GRANT_RHIZA_CONFIG_JSON="+string(raw), "GRANT_BIND_ADDR=invalid-listener-address", "GRANT_APP_AUTH_KEY=test-app-key", "GRANT_RAUTHY_CLIENT_ID=test-client", "GRANT_RAUTHY_CLIENT_SECRET=test-client-secret")
			if mode == "extra" {
				cmd.Env = append(cmd.Env, "GRANT_RECOVERY_TEST_EXTRA=1")
			}
			out, err := cmd.CombinedOutput()
			if strings.Contains(string(out), r.ObjStoreSecretKey) {
				t.Fatalf("secret leaked: %s", out)
			}
			if mode != "success" {
				var exit *exec.ExitError
				if !errors.As(err, &exit) || exit.ExitCode() != 1 {
					t.Fatalf("expected exit 1 for %s: %v %s", mode, err, out)
				}
				if mode != "open-error" {
					if _, e := os.Stat(root); !os.IsNotExist(e) {
						t.Fatalf("created database: %v", e)
					}
				}
				t.Logf("%s exit 1: %s", mode, out)
				return
			}
			if err != nil {
				t.Fatalf("entrypoint: %v %s", err, out)
			}
			if !strings.Contains(string(out), "Grant local storage recovery completed") {
				t.Fatalf("missing completion metadata: %s", out)
			}
			cfg := grant.Config{Database: grant.DatabaseConfig{DataDir: root, Rhiza: r}}
			store, err := grant.OpenStore(context.Background(), cfg)
			if err != nil {
				t.Fatal(err)
			}
			if _, err := store.Exec(ctx, "CREATE TABLE recovery_cli_probe (id TEXT PRIMARY KEY)"); err != nil {
				_ = store.Close()
				t.Fatal(err)
			}
			if _, err := store.Exec(ctx, "INSERT INTO recovery_cli_probe VALUES ('before-loss')"); err != nil {
				_ = store.Close()
				t.Fatal(err)
			}
			if err := store.Close(); err != nil {
				t.Fatal(err)
			}
			if err := os.RemoveAll(root); err != nil {
				t.Fatal(err)
			}
			restore := exec.CommandContext(ctx, os.Args[0], "-test.run=^TestRestoreLocalStorageEntrypoint$")
			restore.Env = cmd.Env
			if output, err := restore.CombinedOutput(); err != nil {
				t.Fatalf("restore after loss: %v %s", err, output)
			}
			store, err = grant.OpenStore(ctx, cfg)
			if err != nil {
				t.Fatal(err)
			}
			rows, err := store.Query(ctx, grant.ReadStrong, "SELECT id FROM recovery_cli_probe")
			if err != nil || len(rows) != 1 || rows[0]["id"] != "before-loss" {
				_ = store.Close()
				t.Fatalf("CLI restore lost SQL: %v %v", rows, err)
			}
			if _, err := store.Exec(ctx, "INSERT INTO recovery_cli_probe VALUES ('after-loss')"); err != nil {
				_ = store.Close()
				t.Fatal(err)
			}
			if err := store.Close(); err != nil {
				t.Fatal(err)
			}
			t.Logf("success exit 0: %s", out)
		})
	}
}
