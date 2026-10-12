package main

import (
	"net/http"
	"net/http/httptest"
	"testing"
)

func TestPromotionGate(t *testing.T) {
	for _, tc := range []struct {
		path string
		want int
	}{
		{"/healthz", http.StatusOK},
		{"/readyz", http.StatusServiceUnavailable},
		{"/api/v1/me", http.StatusServiceUnavailable},
	} {
		r := httptest.NewRecorder()
		promotionGate{}.ServeHTTP(r, httptest.NewRequest(http.MethodGet, tc.path, nil))
		if r.Code != tc.want {
			t.Fatalf("%s: got %d, want %d", tc.path, r.Code, tc.want)
		}
		private := httptest.NewRecorder()
		promotionGate{private: true}.ServeHTTP(private, httptest.NewRequest(http.MethodGet, tc.path, nil))
		if private.Code != http.StatusServiceUnavailable {
			t.Fatalf("private %s: got %d, want 503", tc.path, private.Code)
		}
	}
}
