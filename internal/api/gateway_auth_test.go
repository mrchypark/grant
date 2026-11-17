package api

import (
	"net/http"
	"net/http/httptest"
	"strings"
	"testing"

	"github.com/mrchypark/grant/internal/grant"
)

func TestTrustedGatewayAuthentication(t *testing.T) {
	store, err := grant.OpenStore(t.Context(), grant.Config{Database: grant.DatabaseConfig{DataDir: t.TempDir(), DefaultReadConsistency: grant.ReadStrong}})
	if err != nil {
		t.Fatal(err)
	}
	defer store.Close()

	gatewayKey := strings.Repeat("g", 32)
	srv := New(grant.NewService(grant.Config{
		Security: grant.SecurityConfig{
			MasterKey:         strings.Repeat("m", 32),
			AppAuthKey:        strings.Repeat("a", 32),
			TrustedGatewayKey: gatewayKey,
		},
		Auth: grant.AuthConfig{ProviderMode: "oidc"},
	}, store), false)

	req := httptest.NewRequest("POST", "/api/v1/me/ensure", nil)
	req.Header.Set("X-Grant-Gateway-Key", gatewayKey)
	req.Header.Set("X-Grant-Subject", "gw-user-1")
	req.Header.Set("X-Grant-Client-Id", "gw-app")
	rec := httptest.NewRecorder()
	srv.ServeHTTP(rec, req)
	if rec.Code != http.StatusOK {
		t.Fatalf("materialize expected 200, got %d: %s", rec.Code, rec.Body.String())
	}

	req = httptest.NewRequest("GET", "/api/v1/me", nil)
	req.Header.Set("X-Grant-Gateway-Key", gatewayKey)
	req.Header.Set("X-Grant-Subject", "gw-user-1")
	req.Header.Set("X-Grant-Client-Id", "gw-app")
	rec = httptest.NewRecorder()
	srv.ServeHTTP(rec, req)
	if rec.Code != http.StatusOK {
		t.Fatalf("expected 200, got %d: %s", rec.Code, rec.Body.String())
	}

	req = httptest.NewRequest("GET", "/api/v1/me", nil)
	req.Header.Set("X-Grant-Gateway-Key", gatewayKey)
	rec = httptest.NewRecorder()
	srv.ServeHTTP(rec, req)
	if rec.Code != http.StatusUnauthorized {
		t.Fatalf("expected 401, got %d", rec.Code)
	}

	req = httptest.NewRequest("GET", "/api/v1/me", nil)
	req.Header.Set("X-Grant-Gateway-Key", "wrong-key")
	req.Header.Set("X-Grant-Subject", "gw-user-1")
	rec = httptest.NewRecorder()
	srv.ServeHTTP(rec, req)
	if rec.Code != http.StatusUnauthorized {
		t.Fatalf("expected 401, got %d", rec.Code)
	}

	disabledSrv := New(grant.NewService(grant.Config{
		Security: grant.SecurityConfig{MasterKey: "m", AppAuthKey: "a"},
		Auth:     grant.AuthConfig{ProviderMode: "oidc"},
	}, store), false)
	req = httptest.NewRequest("GET", "/api/v1/me", nil)
	req.Header.Set("X-Grant-Gateway-Key", gatewayKey)
	req.Header.Set("X-Grant-Subject", "gw-user-1")
	rec = httptest.NewRecorder()
	disabledSrv.ServeHTTP(rec, req)
	if rec.Code != http.StatusUnauthorized {
		t.Fatalf("expected 401 when gateway auth disabled, got %d", rec.Code)
	}
}
