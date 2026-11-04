package api

import (
	"net/http/httptest"
	"strings"
	"testing"
)

func TestLearnerApplicationAuthorizationGateAndClosedInput(t *testing.T) {
	s := testServer()
	s.learner = true
	valid := `{"action":"read","resource":{"provider":"operable","type":"workspace","id":"cypark-default"}}`
	for _, tc := range []struct {
		method, path, body string
		want               int
	}{
		{"POST", "/api/v1/application-authz/check", valid, 401},
		{"POST", "/api/v1/application-authz/batch-check", `{"checks":[]}`, 401},
		{"POST", "/api/v1/application-authz/filter", `{"action":"read","resources":[]}`, 401},
		{"GET", "/api/v1/application-authz/check", valid, 404},
		{"POST", "/api/v1/authz/check", valid, 503},
		{"POST", "/api/v1/admin/providers", `{}`, 503},
		{"POST", "/api/v1/application-authz/check", `{"action":"read","subject":{},"resource":{"provider":"p","type":"w","id":"x"}}`, 422},
		{"POST", "/api/v1/application-authz/check", `{"action":"read","resource":{"provider":"p","type":"w","id":"x","extra":1}}`, 422},
		{"POST", "/api/v1/application-authz/check", `{"action":"read","resource":{"provider":"p","type":"w","id":" "}}`, 422},
		{"POST", "/api/v1/application-authz/filter", `{"action":"read"}`, 422},
		{"POST", "/api/v1/application-authz/batch-check", `{"checks":null}`, 422},
		{"POST", "/api/v1/application-authz/filter", `{"action":"read","action":"write","resources":[]}`, 422},
		{"POST", "/api/v1/application-authz/filter", `{"action":"read","resources":[]} {}`, 400},
		{"POST", "/api/v1/application-authz/check", `{"action":"read","resource":{"provider":"p","type":"w","id":"x","view":null}}`, 400},
	} {
		t.Run(tc.method+tc.path+tc.body, func(t *testing.T) {
			r := httptest.NewRequest(tc.method, tc.path, strings.NewReader(tc.body))
			r.Header.Set("Content-Type", "application/json")
			w := httptest.NewRecorder()
			s.ServeHTTP(w, r)
			if w.Code != tc.want {
				t.Fatalf("got %d want %d: %s", w.Code, tc.want, w.Body.String())
			}
		})
	}
	// Valid extension fields keep reaching authentication on the legacy voter.
	s.learner = false
	r := httptest.NewRequest("POST", "/api/v1/application-authz/check", strings.NewReader(`{"action":"read","extension":true,"resource":{"provider":"p","type":"w","id":"x"}}`))
	r.Header.Set("Content-Type", "application/json")
	w := httptest.NewRecorder()
	s.ServeHTTP(w, r)
	if w.Code != 401 {
		t.Fatalf("voter contract changed: %d %s", w.Code, w.Body.String())
	}
}

func TestLearnerApplicationAuthorizationBounds(t *testing.T) {
	s := testServer()
	s.learner = true
	candidate := `{"provider":"operable","type":"workspace","id":"cypark-default"}`
	for _, n := range []int{1000, 1001} {
		body := `{"action":"read","resources":[` + strings.Repeat(candidate+",", n-1) + candidate + `]}`
		r := httptest.NewRequest("POST", "/api/v1/application-authz/filter", strings.NewReader(body))
		r.Header.Set("Content-Type", "application/json")
		w := httptest.NewRecorder()
		s.ServeHTTP(w, r)
		want := 401
		if n == 1001 {
			want = 400
		}
		if w.Code != want {
			t.Fatalf("n=%d: %d %s", n, w.Code, w.Body.String())
		}
	}
	r := httptest.NewRequest("POST", "/api/v1/application-authz/check", strings.NewReader(strings.Repeat(" ", 2*1024*1024+1)))
	r.Header.Set("Content-Type", "application/json")
	w := httptest.NewRecorder()
	s.ServeHTTP(w, r)
	if w.Code != 413 {
		t.Fatalf("oversize=%d", w.Code)
	}
}

func TestLearnerApplicationAuthorizationHeaders(t *testing.T) {
	for _, name := range []string{"Authorization", "X-Grant-App-Auth-Key", "X-Organization-ID"} {
		s := testServer()
		s.learner = true
		r := httptest.NewRequest("POST", "/api/v1/application-authz/filter", strings.NewReader(`{"action":"read","resources":[]}`))
		r.Header.Set("Content-Type", "application/json")
		r.Header.Set("Authorization", "Bearer token")
		r.Header.Set("X-Grant-App-Auth-Key", "app-secret")
		r.Header.Set("X-Organization-ID", "org")
		r.Header.Add(name, "ambiguous")
		w := httptest.NewRecorder()
		s.ServeHTTP(w, r)
		if w.Code != 400 {
			t.Fatalf("duplicate %s: %d", name, w.Code)
		}
	}
}
