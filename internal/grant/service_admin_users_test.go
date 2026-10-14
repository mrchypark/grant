package grant

import (
	"context"
	"encoding/json"
	"net/http"
	"net/http/httptest"
	"testing"

	"github.com/mrchypark/rhiza"
)

type directoryReadTransport func(*http.Request) (*http.Response, error)

func (f directoryReadTransport) RoundTrip(r *http.Request) (*http.Response, error) { return f(r) }

func TestListUsersAdminStrongRead(t *testing.T) {
	calls := 0
	fail := false
	original := http.DefaultTransport
	t.Cleanup(func() { http.DefaultTransport = original })
	http.DefaultTransport = directoryReadTransport(func(r *http.Request) (*http.Response, error) {
		w := httptest.NewRecorder()
		calls++
		if r.URL.Path != "/sql/query" {
			t.Errorf("unexpected request: %s", r.URL.Path)
		}
		var query rhiza.QueryRequest
		if err := json.NewDecoder(r.Body).Decode(&query); err != nil {
			t.Error(err)
		}
		if query.Consistency != rhiza.ConsistencyLinearizable {
			t.Errorf("read consistency = %s", query.Consistency)
		}
		if fail {
			w.WriteHeader(http.StatusServiceUnavailable)
			return w.Result(), nil
		}
		json.NewEncoder(w).Encode(rhiza.QueryResponse{Columns: []string{"id", "email", "display_name", "active"}, Rows: [][]any{{"a", nil, "A", int64(1)}, {"b", nil, "B", int64(0)}}})
		return w.Result(), nil
	})
	cfg := Config{Database: DatabaseConfig{DefaultReadConsistency: ReadLocal, RemoteNodes: []string{"http://voter.test"}}}
	svc := NewService(cfg, &Store{cfg: cfg})
	page, err := svc.ListUsersAdmin(context.Background(), "", "", 1)
	if err != nil || len(page.Items) != 1 || page.Items[0].ID != "a" || page.NextCursor != "a" || calls != 1 {
		t.Fatalf("page=%+v err=%v calls=%d", page, err, calls)
	}
	fail = true
	if _, err = svc.ListUsersAdmin(context.Background(), "", "", 1); err == nil || calls != 2 {
		t.Fatalf("strong read failure fell back: err=%v calls=%d", err, calls)
	}
	if _, err = svc.ListUsersAdmin(context.Background(), "", "", -1); ErrorStatus(err) != 400 || calls != 2 {
		t.Fatalf("negative limit: err=%v calls=%d", err, calls)
	}
}
