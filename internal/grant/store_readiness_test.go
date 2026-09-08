package grant

import (
	"bytes"
	"context"
	"encoding/json"
	"io"
	"net/http"
	"net/http/httptest"
	"strings"
	"testing"
	"time"
)

func TestReadinessHonorsConfiguredConsistencyWithoutQuorum(t *testing.T) {
	secret := strings.Repeat("r", 40)
	local, e := OpenStore(context.Background(), Config{Database: DatabaseConfig{DataDir: t.TempDir(), RemoteAPISecret: secret}})
	if e != nil {
		t.Fatal(e)
	}
	defer local.Close()
	native := local.RuntimeHandler()
	server := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		b, e := io.ReadAll(r.Body)
		if e != nil {
			t.Error(e)
			return
		}
		var req struct {
			Consistency string `json:"consistency"`
		}
		if e = json.Unmarshal(b, &req); e != nil {
			t.Error(e)
			return
		}
		if req.Consistency == "linearizable" {
			http.Error(w, "quorum unavailable", 503)
			return
		}
		r.Body = io.NopCloser(bytes.NewReader(b))
		native.ServeHTTP(w, r)
	}))
	defer server.Close()
	for _, tc := range []struct {
		mode  string
		ready bool
	}{{ReadStrong, false}, {ReadLocal, true}, {ReadPreferStrong, true}} {
		t.Run(tc.mode, func(t *testing.T) {
			cfg := Config{Database: DatabaseConfig{RuntimeMode: "remote", DefaultReadConsistency: tc.mode, RemoteNodes: []string{server.URL}, RemoteAPISecret: secret}}
			service := NewService(cfg, &Store{cfg: cfg})
			health, ready := service.Readiness(context.Background())
			if ready != tc.ready || !health.ReadReady || health.WriteReady {
				t.Fatalf("health=%+v ready=%v want=%v", health, ready, tc.ready)
			}
		})
	}
}
func TestReadinessDeadlineBoundsUnresponsiveRuntime(t *testing.T) {
	server := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) { io.Copy(io.Discard, r.Body); <-r.Context().Done() }))
	defer server.Close()
	cfg := Config{Database: DatabaseConfig{RuntimeMode: "remote", DefaultReadConsistency: ReadStrong, RemoteNodes: []string{server.URL}, RemoteAPISecret: strings.Repeat("s", 40)}}
	service := NewService(cfg, &Store{cfg: cfg})
	start := time.Now()
	_, ready := service.Readiness(context.Background())
	if ready || time.Since(start) > 2500*time.Millisecond {
		t.Fatalf("unavailable readiness=%v elapsed=%v", ready, time.Since(start))
	}
}
