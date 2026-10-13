package grant

import (
	"context"
	_ "embed"
	"errors"
	"fmt"
	"github.com/mrchypark/rhiza"
	"strings"
	"time"
)

//go:embed store_schema.sql
var schemaSQL string

//go:embed store_indexes.sql
var indexesSQL string

//go:embed store_schema_goauthy_v4.sql
var goauthySchemaV4SQL string

func canonicalSchemaDigest() string {
	return digestBytes([]byte(schemaSQL + indexesSQL + goauthySchemaV4SQL))
}

func legacySchemaDigestV3() string {
	return digestBytes([]byte(schemaSQL + indexesSQL))
}

func (s *Store) Migrate(ctx context.Context) error {
	if e := s.coarsePreflight(ctx, ReadStrong); e != nil {
		return e
	}
	var migrations []rhiza.Migration
	for _, group := range []struct{ name, sql string }{{"grant canonical tables", schemaSQL}, {"grant canonical indexes", indexesSQL}, {"grant restore activation", "CREATE TABLE IF NOT EXISTS grant_restore_activation (id TEXT PRIMARY KEY, snapshot_digest TEXT NOT NULL, cluster_id TEXT NOT NULL, activated_at TEXT NOT NULL)"}, {"grant qualified Goauthy identities", goauthySchemaV4SQL}} {
		var stmts []Statement
		for _, q := range strings.Split(group.sql, ";") {
			if strings.TrimSpace(q) != "" {
				stmts = append(stmts, Statement{SQL: q})
			}
		}
		migrations = append(migrations, rhiza.Migration{Version: int64(len(migrations) + 1), Name: group.name, Statements: stmts})
	}
	// Native Migrate derives each request ID from its immutable migration checksum.
	// Replaying this exact plan resolves unknown commits without issuing new writes.
	if err := retryNativeOutcome(ctx, func(ctx context.Context) error { return s.db.Migrate(ctx, migrations) }); err != nil {
		return fmt.Errorf("migrate Grant schema: %w", err)
	}
	return s.VerifySchema(ctx)
}
func retryNativeOutcome(ctx context.Context, call func(context.Context) error) error {
	bounded, cancel := context.WithTimeout(ctx, 60*time.Second)
	defer cancel()
	for {
		err := call(bounded)
		if err == nil {
			return nil
		}
		if !errors.Is(err, rhiza.ErrCommitUnknown) && !errors.Is(err, rhiza.ErrDurabilityUnavailable) && !errors.Is(err, rhiza.ErrNotReady) && !errors.Is(err, rhiza.ErrQuorumUnavailable) {
			return err
		}
		select {
		case <-bounded.Done():
			return fmt.Errorf("native outcome unresolved (%v): %w", err, bounded.Err())
		case <-time.After(100 * time.Millisecond):
		}
	}
}
func (s *Store) VerifySchema(ctx context.Context) error {
	return s.verifySchemaAt(ctx, ReadStrong)
}
func (s *Store) verifySchemaAt(ctx context.Context, consistency string) error {
	rows, err := s.Query(ctx, consistency, "SELECT (SELECT COUNT(*) FROM actions WHERE visibility_granting<>0)+(SELECT COUNT(*) FROM role_action_field_views)+(SELECT COUNT(*) FROM field_view_policy_audit_outbox) AS invalid")
	if err != nil {
		return err
	}
	if rowInt(rows[0], "invalid") != 0 {
		return fmt.Errorf("active legacy field authorization is incompatible with coarse deployment")
	}
	return s.coarsePreflight(ctx, consistency)
}
func (s *Store) coarsePreflight(ctx context.Context, consistency string) error {
	optional, e := s.Query(ctx, consistency, "SELECT name FROM sqlite_schema WHERE type='table' AND name IN('actions','role_action_field_views','field_view_policy_audit_outbox','field_view_policy_audit_deliveries','mask_catalog_manifests','mask_catalog_active')")
	if e != nil {
		return e
	}
	for _, r := range optional {
		table := rowString(r, "name")
		query := "SELECT COUNT(*) AS n FROM " + quoteIdentifier(table)
		if table == "actions" {
			query += " WHERE visibility_granting<>0"
		}
		counts, e := s.Query(ctx, consistency, query)
		if e != nil {
			return e
		}
		if rowInt(counts[0], "n") > 0 {
			return fmt.Errorf("active legacy %s blocks coarse deployment", table)
		}
	}
	return nil
}
