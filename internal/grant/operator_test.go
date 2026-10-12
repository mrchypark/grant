package grant

import (
	"context"
	"net/http"
	"net/http/httptest"
	"testing"

	"github.com/mrchypark/rhiza"
)

func TestOperatorHandlerOnlyExposesNativeRecoveryRoutes(t *testing.T) {
	db, err := rhiza.Open(context.Background(), rhiza.Config{Local: true, NodeID: "test", DataDir: t.TempDir()})
	if err != nil {
		t.Fatal(err)
	}
	defer db.Close()
	h, err := (&Store{db: db}).OperatorHandler()
	if err != nil {
		t.Fatal(err)
	}
	for _, path := range []string{"/sql/query", "/ready", "/other"} {
		r := httptest.NewRecorder()
		h.ServeHTTP(r, httptest.NewRequest(http.MethodGet, path, nil))
		if r.Code != http.StatusNotFound {
			t.Fatalf("%s: got %d, want 404", path, r.Code)
		}
	}
}
