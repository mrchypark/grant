package grant

import (
	"context"
	"errors"
	"fmt"
	"strings"
	"testing"

	"github.com/mrchypark/rhiza"
)

func TestSafeRestoreError(t *testing.T) {
	const secret = "https://signed.invalid/?token=SECRET SQL args config"
	for _, tc := range []struct {
		cause error
		class string
	}{
		{context.DeadlineExceeded, "deadline"},
		{context.Canceled, "canceled"},
		{rhiza.ErrNotReady, "not_ready"},
		{rhiza.ErrQuorumUnavailable, "quorum_unavailable"},
		{rhiza.ErrDurabilityUnavailable, "durability_unavailable"},
		{rhiza.ErrCommitUnknown, "commit_unknown"},
		{errors.New(secret), "unknown"},
	} {
		t.Run(tc.class, func(t *testing.T) {
			cause := fmt.Errorf("%s: %w", secret, tc.cause)
			err := safeRestoreError("import", cause)
			want := "single-voter restore failed; target evidence retained, do not reset or reuse its namespace; phase=import error_class=" + tc.class
			if err.Error() != want || errors.Unwrap(err) != nil {
				t.Fatal("diagnostic must expose only the fixed message, phase and class")
			}
		})
	}
	for _, phase := range []string{"target_binding", "create_directory", "pending_marker", "open_store", "import", "verify_export", "verify_tables", "verify_schema", "verify_receipt", "close", "finalize_marker"} {
		if !strings.Contains(safeRestoreError(phase, errors.New(secret)).Error(), "phase="+phase+" error_class=unknown") {
			t.Fatal("known phase lost")
		}
	}
	if strings.Contains(safeRestoreError(secret, errors.New(secret)).Error(), secret) {
		t.Fatal("unrecognized phase leaked")
	}
}
