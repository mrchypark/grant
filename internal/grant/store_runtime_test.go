package grant

import (
	"context"
	"errors"
	"fmt"
	"github.com/mrchypark/rhiza"
	"testing"
)

func TestStoreSchemaAndAtomicRollback(t *testing.T) {
	ctx := context.Background()
	s, e := OpenStore(ctx, Config{Database: DatabaseConfig{DataDir: t.TempDir()}})
	if e != nil {
		t.Fatal(e)
	}
	defer s.Close()
	if e = s.EnsureBootstrapData(ctx); e != nil {
		t.Fatal(e)
	}
	_, e = s.Execute(ctx, "bad", Statement{SQL: "INSERT INTO users(id,display_name,created_at) VALUES('rollback','x','now')"}, Statement{SQL: "INSERT INTO application_consent_assertions(id,valid) VALUES('bad',0)"})
	if e == nil {
		t.Fatal("expected rollback")
	}
	r, e := s.Query(ctx, ReadStrong, "SELECT id FROM users WHERE id='rollback'")
	if e != nil || len(r) != 0 {
		t.Fatalf("rollback rows=%v err=%v", r, e)
	}
}
func TestExecAffectedRowsAndReturning(t *testing.T) {
	ctx := context.Background()
	s, e := OpenStore(ctx, Config{Database: DatabaseConfig{DataDir: t.TempDir()}})
	if e != nil {
		t.Fatal(e)
	}
	defer s.Close()
	for _, tc := range []struct {
		q    string
		want int64
	}{{"INSERT INTO users(id,display_name,created_at) VALUES('row','Row','now')", 1}, {"UPDATE users SET active=0 WHERE id='row'", 1}, {"UPDATE users SET active=0 WHERE id='missing'", 0}, {"DELETE FROM users WHERE id='row'", 1}, {"DELETE FROM users WHERE id='row'", 0}} {
		n, e := s.Exec(ctx, tc.q)
		if e != nil || n != tc.want {
			t.Fatalf("%s: count=%d want=%d err=%v", tc.q, n, tc.want, e)
		}
	}
	r, e := s.Execute(ctx, "returning", Statement{SQL: "INSERT INTO users(id,display_name,created_at) VALUES('return','Row','now') RETURNING id", WantRows: true})
	if e != nil || len(r.Statements) != 1 || len(r.Statements[0].Rows) != 1 {
		t.Fatalf("RETURNING receipt=%+v err=%v", r, e)
	}
}
func TestBootstrapPreservesDisabledAuthority(t *testing.T) {
	ctx := context.Background()
	s, e := OpenStore(ctx, Config{Database: DatabaseConfig{DataDir: t.TempDir()}})
	if e != nil {
		t.Fatal(e)
	}
	defer s.Close()
	if e = s.EnsureBootstrapData(ctx); e != nil {
		t.Fatal(e)
	}
	if _, e = s.Exec(ctx, "UPDATE organization_memberships SET status='disabled' WHERE user_id='conalog-grant'"); e != nil {
		t.Fatal(e)
	}
	if _, e = s.Exec(ctx, "UPDATE service_providers SET active=0 WHERE id='grant'"); e != nil {
		t.Fatal(e)
	}
	if e = s.EnsureBootstrapData(ctx); e != nil {
		t.Fatal(e)
	}
	r, e := s.Query(ctx, ReadStrong, "SELECT (SELECT COUNT(*) FROM organization_memberships WHERE status='active') AS n,(SELECT active FROM service_providers WHERE id='grant') AS active,(SELECT COUNT(*) FROM application_consent_assertions) AS checks")
	if e != nil || rowInt(r[0], "n") != 0 || rowInt(r[0], "active") != 0 || rowInt(r[0], "checks") != 0 {
		t.Fatalf("bootstrap reenabled authority: %v %v", r, e)
	}
}

func TestNativeOutcomeRetryRetainsOperationAndRejectsPermanentFailure(t *testing.T) {
	attempts := 0
	e := retryNativeOutcome(context.Background(), func(context.Context) error {
		attempts++
		if attempts == 1 {
			return fmt.Errorf("archive head race: %w", rhiza.ErrCommitUnknown)
		}
		return nil
	})
	if e != nil || attempts != 2 {
		t.Fatalf("unknown resolution attempts=%d err=%v", attempts, e)
	}
	attempts = 0
	sentinel := errors.New("invalid migration")
	e = retryNativeOutcome(context.Background(), func(context.Context) error { attempts++; return sentinel })
	if !errors.Is(e, sentinel) || attempts != 1 {
		t.Fatalf("permanent failure retried: %d %v", attempts, e)
	}
}
