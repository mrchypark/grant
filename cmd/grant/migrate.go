package main

import (
	"context"
	"fmt"
	"github.com/mrchypark/grant/internal/grant"
	"os"
)

func migration(ctx context.Context, command string, values []string) error {
	m := map[string]string{}
	stopped := false
	for i := 0; i < len(values); i++ {
		key := values[i]
		if key == "--writes-stopped" {
			if stopped {
				return fmt.Errorf("duplicate --writes-stopped")
			}
			stopped = true
			continue
		}
		if key != "--sqlite" && key != "--out" && key != "--snapshot" && key != "--data-dir" {
			return fmt.Errorf("unknown migration argument %s", key)
		}
		if i+1 == len(values) {
			return fmt.Errorf("missing value for %s", key)
		}
		if _, ok := m[key]; ok {
			return fmt.Errorf("duplicate %s", key)
		}
		i++
		m[key] = values[i]
	}
	switch command {
	case "migrate-export":
		if !stopped {
			return fmt.Errorf("migrate-export requires --writes-stopped after stopping all source writers")
		}
		if len(m) != 2 || m["--sqlite"] == "" || m["--out"] == "" {
			return fmt.Errorf("usage: grant migrate-export --writes-stopped --sqlite PATH --out FILE")
		}
		snap, e := grant.ExportLegacySQLite(ctx, m["--sqlite"])
		if e != nil {
			return e
		}
		return grant.WriteStoreSnapshot(m["--out"], snap)
	case "migrate-prepare-cluster":
		if stopped || len(m) != 1 || m["--snapshot"] == "" {
			return fmt.Errorf("usage: grant migrate-prepare-cluster --snapshot FILE")
		}
		snap, e := grant.ReadStoreSnapshot(m["--snapshot"])
		if e != nil {
			return e
		}
		cfg, e := grant.ConfigFromEnv()
		if e != nil {
			return e
		}
		return grant.PrepareClusterRestore(ctx, cfg, snap)
	case "migrate-restore":
		if stopped || len(m) != 2 || m["--snapshot"] == "" || m["--data-dir"] == "" {
			return fmt.Errorf("usage: grant migrate-restore --snapshot FILE --data-dir NEW")
		}
		snap, e := grant.ReadStoreSnapshot(m["--snapshot"])
		if e != nil {
			return e
		}
		cfg, e := grant.ConfigFromEnv()
		if e != nil {
			return e
		}
		cfg.Database.DataDir = m["--data-dir"]
		return grant.RestoreStoreSnapshot(ctx, cfg, snap)
	default:
		return fmt.Errorf("unknown command %s", command)
	}
}
func commandLine(ctx context.Context) (bool, error) {
	if len(os.Args) == 1 {
		return false, nil
	}
	if os.Args[1] == "restore-local-storage" {
		if len(os.Args) != 2 {
			return true, fmt.Errorf("usage: grant restore-local-storage")
		}
		cfg, err := grant.ConfigFromEnv()
		if err != nil {
			return true, err
		}
		if err := grant.RestoreLocalStorage(ctx, cfg); err != nil {
			return true, err
		}
		_, err = fmt.Fprintln(os.Stdout, "Grant local storage recovery completed")
		return true, err
	}
	return true, migration(ctx, os.Args[1], os.Args[2:])
}
