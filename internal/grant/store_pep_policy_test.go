package grant

import (
	"context"
	"sync"
	"testing"
	"time"
)

func TestPepPolicyCASAndLegacyDefaults(t *testing.T) {
	ctx := context.Background()
	s, err := OpenStore(ctx, Config{Database: DatabaseConfig{DataDir: t.TempDir()}})
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() { _ = s.Close() })
	if err = s.EnsureBootstrapData(ctx); err != nil {
		t.Fatal(err)
	}
	in := UpsertProviderPepClient{Lifecycle: "active"}
	if _, err = s.UpsertProviderPepClientConditional(ctx, "grant", "policy-pep", "operator", in, nil); ErrorStatus(err) != 428 {
		t.Fatalf("missing precondition: %v", err)
	}
	zero := int64(0)
	pep, err := s.UpsertProviderPepClientConditional(ctx, "grant", "policy-pep", "operator", in, &zero)
	if err != nil || pep.AuthorizationMode != "application" || pep.PolicyRevision != 1 {
		t.Fatalf("legacy default: %+v %v", pep, err)
	}
	if _, err = s.UpsertProviderPepClientConditional(ctx, "grant", "policy-pep", "operator", in, &zero); ErrorStatus(err) != 412 {
		t.Fatalf("duplicate create: %v", err)
	}
	user := "user"
	in.AuthorizationMode = &user
	revision := pep.PolicyRevision
	pep, err = s.UpsertProviderPepClientConditional(ctx, "grant", "policy-pep", "operator", in, &revision)
	if err != nil || pep.AuthorizationMode != user || pep.PolicyRevision != 2 {
		t.Fatalf("explicit user: %+v %v", pep, err)
	}
	if _, err = s.UpsertProviderPepClientConditional(ctx, "grant", "policy-pep", "operator", UpsertProviderPepClient{Lifecycle: "disabled"}, &revision); ErrorStatus(err) != 412 {
		t.Fatalf("stale revision: %v", err)
	}
	pep, err = s.UpsertProviderPepClient(ctx, "grant", "policy-pep", "bootstrap", UpsertProviderPepClient{Lifecycle: "disabled"})
	if err != nil || pep.AuthorizationMode != user || pep.PolicyRevision != 3 {
		t.Fatalf("legacy omission must preserve user: %+v %v", pep, err)
	}
	application := "application"
	revision = pep.PolicyRevision
	pep, err = s.UpsertProviderPepClientConditional(ctx, "grant", "policy-pep", "operator", UpsertProviderPepClient{Lifecycle: "active", AuthorizationMode: &application}, &revision)
	if err != nil || pep.AuthorizationMode != application || pep.PolicyRevision != 4 {
		t.Fatalf("CAS rollback: %+v %v", pep, err)
	}
	unknown := "automatic"
	revision = pep.PolicyRevision
	if _, err = s.UpsertProviderPepClientConditional(ctx, "grant", "policy-pep", "operator", UpsertProviderPepClient{Lifecycle: "active", AuthorizationMode: &unknown}, &revision); ErrorStatus(err) != 422 {
		t.Fatalf("unknown mode: %v", err)
	}
	rows, err := s.Query(ctx, ReadStrong, "SELECT policy_revision,authorization_mode FROM provider_pep_client_receipts WHERE pep_client_id=? ORDER BY policy_revision", "policy-pep")
	if err != nil || len(rows) != 4 || rowInt(rows[1], "policy_revision") != 2 || rowString(rows[1], "authorization_mode") != user {
		t.Fatalf("receipts must include committed writes only: %v %v", rows, err)
	}
}

func TestPepPolicyCASConcurrentWriters(t *testing.T) {
	ctx := context.Background()
	s, err := OpenStore(ctx, Config{Database: DatabaseConfig{DataDir: t.TempDir()}})
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() { _ = s.Close() })
	if err = s.EnsureBootstrapData(ctx); err != nil {
		t.Fatal(err)
	}
	pep, err := s.UpsertProviderPepClient(ctx, "grant", "race-pep", "bootstrap", UpsertProviderPepClient{Lifecycle: "active"})
	if err != nil {
		t.Fatal(err)
	}
	revision := pep.PolicyRevision
	start := make(chan struct{})
	results := make(chan error, 2)
	var workers sync.WaitGroup
	for _, lifecycle := range []string{"disabled", "deleted"} {
		workers.Add(1)
		go func(lifecycle string) {
			defer workers.Done()
			<-start
			_, err := s.UpsertProviderPepClientConditional(ctx, "grant", "race-pep", "operator", UpsertProviderPepClient{Lifecycle: lifecycle}, &revision)
			results <- err
		}(lifecycle)
	}
	close(start)
	workers.Wait()
	close(results)
	committed, stale := 0, 0
	for err := range results {
		switch {
		case err == nil:
			committed++
		case ErrorStatus(err) == 412:
			stale++
		default:
			t.Fatalf("unexpected race error: %v", err)
		}
	}
	pep, err = s.ProviderPepClientAdmin(ctx, "grant", "race-pep")
	if err != nil || committed != 1 || stale != 1 || pep.PolicyRevision != 2 || pep.AuthorizationMode != "application" {
		t.Fatalf("race was not atomic: committed=%d stale=%d policy=%+v err=%v", committed, stale, pep, err)
	}
	rows, err := s.Query(ctx, ReadStrong, "SELECT COUNT(*) AS n FROM provider_pep_client_receipts WHERE pep_client_id='race-pep'")
	if err != nil || rowInt(rows[0], "n") != 2 {
		t.Fatalf("loser left a receipt: %v %v", rows, err)
	}
}

func TestPepPolicyAdminReadAndOverflow(t *testing.T) {
	ctx := context.Background()
	s, err := OpenStore(ctx, Config{Database: DatabaseConfig{DataDir: t.TempDir()}})
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() { _ = s.Close() })
	if err = s.EnsureBootstrapData(ctx); err != nil {
		t.Fatal(err)
	}
	if _, err = s.ProviderPepClientAdmin(ctx, "grant", "missing"); ErrorStatus(err) != 404 {
		t.Fatalf("missing policy: %v", err)
	}
	expiry := time.Now().UTC().Add(-time.Hour).Format(time.RFC3339Nano)
	for _, lifecycle := range []string{"active", "disabled", "deleted"} {
		pep, err := s.UpsertProviderPepClient(ctx, "grant", "inactive-pep", "operator", UpsertProviderPepClient{Lifecycle: lifecycle, ValidUntil: &expiry})
		if err != nil {
			t.Fatal(err)
		}
		got, err := s.ProviderPepClientAdmin(ctx, "grant", pep.PepClientID)
		if err != nil || got.PolicyRevision != pep.PolicyRevision || got.Lifecycle != lifecycle || got.ValidUntil == nil {
			t.Fatalf("inactive read: %+v %v", got, err)
		}
		if err = requireActivePEP(got, time.Now()); ErrorStatus(err) != 403 {
			t.Fatalf("inactive policy usable at runtime: %v", err)
		}
	}
	if _, err = s.Exec(ctx, "UPDATE service_providers SET active=0 WHERE id='grant'"); err != nil {
		t.Fatal(err)
	}
	if _, err = s.ProviderPepClientAdmin(ctx, "grant", "inactive-pep"); err != nil {
		t.Fatalf("disabled provider blocked admin read: %v", err)
	}
	if _, err = s.ProviderPepClient(ctx, "grant", "inactive-pep"); ErrorStatus(err) != 403 {
		t.Fatalf("disabled provider accepted at runtime: %v", err)
	}
	if _, err = s.Exec(ctx, "UPDATE service_providers SET active=1 WHERE id='grant'"); err != nil {
		t.Fatal(err)
	}
	if _, err = s.Exec(ctx, "UPDATE provider_pep_clients SET policy_revision=9223372036854775807 WHERE pep_client_id='inactive-pep'"); err != nil {
		t.Fatal(err)
	}
	max := int64(9223372036854775807)
	if _, err = s.UpsertProviderPepClientConditional(ctx, "grant", "inactive-pep", "operator", UpsertProviderPepClient{Lifecycle: "active"}, &max); err == nil {
		t.Fatal("revision overflow accepted")
	}
	got, err := s.ProviderPepClientAdmin(ctx, "grant", "inactive-pep")
	if err != nil || got.PolicyRevision != max || got.Lifecycle != "deleted" {
		t.Fatalf("overflow changed policy: %+v %v", got, err)
	}
	for _, value := range []any{nil, int64(0), int64(-1), 1.5, "not-an-integer"} {
		if _, err = s.Exec(ctx, "UPDATE provider_pep_clients SET policy_revision=? WHERE pep_client_id='inactive-pep'", value); err == nil {
			t.Fatalf("invalid revision accepted: %#v", value)
		}
	}
}
