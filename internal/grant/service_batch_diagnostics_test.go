package grant

import (
	"bytes"
	"context"
	"encoding/json"
	"errors"
	"log/slog"
	"strings"
	"testing"
	"time"
)

func captureBatchDiagnostics(t *testing.T) *bytes.Buffer {
	t.Helper()
	var output bytes.Buffer
	previous := slog.Default()
	slog.SetDefault(slog.New(slog.NewJSONHandler(&output, nil)))
	t.Cleanup(func() { slog.SetDefault(previous) })
	return &output
}

func TestBatchExpiredFenceDiagnostics(t *testing.T) {
	_, service := serviceFixture(t)
	output := captureBatchDiagnostics(t)
	ctx, cancel := context.WithDeadline(context.Background(), time.Now().Add(-time.Second))
	defer cancel()
	allowed, reason, err := service.batchDecision(ctx, "secret-token", "secret-org", HttpCapabilityDecisionInput{}, applicationAuthorizationContext{})
	if allowed || reason != "" || err == nil || ErrorStatus(err) != 503 || err.Error() != "batch quorum fence failed" {
		t.Fatalf("expired fence must fail closed with unchanged public error: %v %q %v", allowed, reason, err)
	}
	var record map[string]any
	if e := json.Unmarshal(output.Bytes(), &record); e != nil {
		t.Fatal(e)
	}
	if ctx.Err() != context.DeadlineExceeded || record["stage"] != "quorum_fence" || record["context_state"] != "deadline_exceeded" || record["deadline_present"] != true || record["configured_budget_ms"] != float64(3000) || record["deadline_remaining_ms"].(float64) >= 0 || record["elapsed_ms"].(float64) < 0 {
		t.Fatalf("unexpected diagnostic: %s", output)
	}
	if strings.Contains(output.String(), "secret-") || strings.Contains(output.String(), "SELECT") || strings.Contains(err.Error(), "stage") {
		t.Fatalf("diagnostic leaked request or internal details: %s / %v", output, err)
	}
}

func TestBatchDiagnosticsSuppressExpectedRejections(t *testing.T) {
	output := captureBatchDiagnostics(t)
	for _, status := range []int{400, 401, 403, 404, 409} {
		logBatchFailure(context.Background(), "evidence", time.Now(), serviceError(status, "private rejection"))
	}
	if output.Len() != 0 {
		t.Fatalf("ordinary rejections must not warn: %s", output)
	}
	ctx, cancel := context.WithCancel(context.Background())
	cancel()
	logBatchFailure(ctx, "authorization", time.Now(), serviceError(403, "private rejection"))
	var record map[string]any
	if err := json.Unmarshal(output.Bytes(), &record); err != nil {
		t.Fatal(err)
	}
	if record["context_state"] != "canceled" || strings.Contains(output.String(), "private rejection") {
		t.Fatalf("cancellation must warn without raw error text: %s", output)
	}
}

func TestBatchFenceErrorPreservesCauseAndPublicStatus(t *testing.T) {
	cause := errors.New("private SQL identifier and argument")
	public := serviceError(503, "batch quorum fence failed")
	err := &batchFenceError{public: public, cause: cause}
	var typed *Error
	if !errors.Is(err, cause) || !errors.As(err, &typed) || typed != public || ErrorStatus(err) != 503 || err.Error() != public.Error() {
		t.Fatal("fence error lost cause or changed public contract")
	}
}

func TestBatchReadFailureDiagnostics(t *testing.T) {
	_, service := serviceFixture(t)
	output := captureBatchDiagnostics(t)
	// A caller-owned fence takes the read path; missing PEP configuration fails
	// deterministically before any token or request data can reach the log.
	ctx := context.WithValue(context.Background(), replicaReadKey{}, &replicaRead{})
	want := func() error { _, err := service.configuredPEP(ctx, "private-provider"); return err }()
	allowed, reason, err := service.batchDecision(ctx, "secret-token", "secret-org", HttpCapabilityDecisionInput{ProviderID: "private-provider"}, applicationAuthorizationContext{})
	if allowed || reason != "" || err == nil || want == nil || err.Error() != want.Error() || ErrorStatus(err) != ErrorStatus(want) {
		t.Fatalf("read failure changed public behavior: %v %q %v (want %v)", allowed, reason, err, want)
	}
	var record map[string]any
	if e := json.Unmarshal(output.Bytes(), &record); e != nil {
		t.Fatal(e)
	}
	if record["stage"] != "pep" || record["context_state"] != "not_done" || record["deadline_present"] != false || strings.Contains(output.String(), "private-provider") || strings.Contains(output.String(), "secret-") {
		t.Fatalf("unexpected or unsafe diagnostic: %s", output)
	}
}

func TestBatchSnapshotRetryDiagnostics(t *testing.T) {
	for _, exhausted := range []bool{false, true} {
		name := "success"
		if exhausted {
			name = "exhausted"
		}
		t.Run(name, func(t *testing.T) {
			output := captureBatchDiagnostics(t)
			ctx := context.Background()
			diagnostic := batchReadDiagnostic{ctx: ctx, stage: "read_fence"}
			attempts := 0
			// Exercise the real retry loop with controlled snapshot changes. The
			// observation is the same final-outcome helper used by batchDecision.
			allowed, err := readAtFence(ctx, &Service{store: &Store{}}, 0, func(ctx context.Context) (bool, error) {
				attempts++
				diagnostic.ctx, diagnostic.stage = ctx, "authorization"
				if exhausted || attempts == 1 {
					readScope(ctx).changed = true
					return false, errors.New("private discarded snapshot error")
				}
				return true, nil
			})
			diagnostic.finish(time.Now(), err)
			if !exhausted {
				if attempts != 2 || !allowed || err != nil || output.Len() != 0 {
					t.Fatalf("recovered retry must be silent: attempts=%d allowed=%v err=%v logs=%s", attempts, allowed, err, output)
				}
				return
			}
			if attempts != 3 || allowed || err == nil || ErrorStatus(err) != 503 || bytes.Count(output.Bytes(), []byte("\n")) != 1 {
				t.Fatalf("exhaustion must fail closed and warn once: attempts=%d allowed=%v err=%v logs=%s", attempts, allowed, err, output)
			}
			var record map[string]any
			if e := json.Unmarshal(output.Bytes(), &record); e != nil {
				t.Fatal(e)
			}
			if record["stage"] != "read_fence" || record["context_state"] != "not_done" || strings.Contains(output.String(), "private") {
				t.Fatalf("wrong final retry diagnostic: %s", output)
			}
		})
	}
}
