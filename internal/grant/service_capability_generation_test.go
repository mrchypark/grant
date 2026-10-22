package grant

import (
	"context"
	"encoding/json"
	"math"
	"net/http"
	"net/http/httptest"
	"strings"
	"sync/atomic"
	"testing"

	"github.com/mrchypark/rhiza"
)

func TestHttpCapabilityGenerationAdminStrongRead(t *testing.T) {
	stage := generationReadFixture(t, 0)
	canonical, err := generationJSON("grant", stage)
	if err != nil {
		t.Fatal(err)
	}
	var calls atomic.Int64
	var fail atomic.Bool
	remote := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		calls.Add(1)
		var query rhiza.QueryRequest
		if err := json.NewDecoder(r.Body).Decode(&query); err != nil {
			t.Error(err)
		}
		if r.URL.Path != "/sql/query" || query.Consistency != rhiza.ConsistencyLinearizable || !strings.Contains(query.SQL, "provider_id=? AND generation=?") || len(query.Args) != 2 || query.Args[0] != "grant" {
			t.Errorf("unexpected strong read: %+v", query)
		}
		if fail.Load() {
			w.WriteHeader(http.StatusServiceUnavailable)
			return
		}
		// Split at an arbitrary boundary to exercise exact chunk reassembly.
		_ = json.NewEncoder(w).Encode(rhiza.QueryResponse{Columns: []string{"payload", "catalog_digest"}, Rows: [][]any{{canonical[:20], "stored-digest"}, {canonical[20:], "stored-digest"}}})
	}))
	defer remote.Close()
	cfg := Config{Database: DatabaseConfig{DefaultReadConsistency: ReadLocal, RemoteNodes: []string{remote.URL}}}
	svc := NewService(cfg, &Store{cfg: cfg})
	got, err := svc.HttpCapabilityGenerationAdmin(context.Background(), "grant", 1)
	if err != nil || got.Generation != 1 || got.CatalogDigest != "stored-digest" || got.Capabilities == nil || got.ResponseResources == nil || calls.Load() != 1 {
		t.Fatalf("got=%+v err=%v calls=%d", got, err, calls.Load())
	}
	fail.Store(true)
	if _, err := svc.HttpCapabilityGenerationAdmin(context.Background(), "grant", 1); err == nil || calls.Load() != 2 {
		t.Fatalf("strong read failure did not fail closed: %v", err)
	}
	for _, invalid := range []uint64{0, math.MaxInt64 + 1, math.MaxUint64} {
		if _, err := svc.HttpCapabilityGenerationAdmin(context.Background(), "grant", invalid); ErrorStatus(err) != 400 || calls.Load() != 2 {
			t.Fatalf("invalid generation %d: %v", invalid, err)
		}
	}
}
