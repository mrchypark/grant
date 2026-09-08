package api

import (
	"context"
	"encoding/json"
	"github.com/Conalog/grant/internal/grant"
	"net/http"
	"net/http/httptest"
	"os"
	"strings"
	"testing"
)

func testServer() *Server {
	return New(grant.NewService(grant.Config{Security: grant.SecurityConfig{MasterKey: "master-secret", AppAuthKey: "app-secret"}, Auth: grant.AuthConfig{ProviderMode: "patch-data"}}, nil), false)
}
func TestCredentialFailures(t *testing.T) {
	s := testServer()
	for _, tc := range []struct {
		name, key string
		want      int
	}{{"missing", "", 401}, {"empty", " ", 400}, {"wrong", "wrong", 401}, {"app-as-master", "app-secret", 401}} {
		t.Run(tc.name, func(t *testing.T) {
			r := httptest.NewRequest("POST", "/api/v1/admin/providers", strings.NewReader(`{"id":"p","name":"p","active":true}`))
			r.Header.Set("Content-Type", "application/json")
			if tc.key != "" {
				r.Header.Set("X-Grant-Master-Key", tc.key)
			}
			w := httptest.NewRecorder()
			s.ServeHTTP(w, r)
			if w.Code != tc.want {
				t.Fatalf("%d: %s", w.Code, w.Body.String())
			}
		})
	}
}
func TestCoarseRejectsEveryViewShape(t *testing.T) {
	s := testServer()
	for _, view := range []string{`null`, `{}`, `false`, `["/name"]`, `"x"`} {
		r := httptest.NewRequest("POST", "/api/v1/authz/check", strings.NewReader(`{"subject":{"type":"user","id":"u","orgId":"o"},"action":"read","resource":{"provider":"p","type":"t","id":"r","view":`+view+`}}`))
		r.Header.Set("Content-Type", "application/json")
		r.Header.Set("X-Grant-Master-Key", "master-secret")
		w := httptest.NewRecorder()
		s.ServeHTTP(w, r)
		if w.Code != 400 || !strings.Contains(w.Body.String(), "resource.view") {
			t.Fatalf("view %s: %d %s", view, w.Code, w.Body.String())
		}
	}
}
func TestStrictDecode(t *testing.T) {
	type sample struct {
		Enabled bool     `json:"enabled"`
		Names   []string `json:"names"`
	}
	for _, raw := range []string{`{"names":[]}`, `{"enabled":null,"names":[]}`, `{"enabled":true,"names":null}`, `{"enabled":true,"enabled":false,"names":[]}`, `{"enabled":true,"names":[],"extra":1}`, `{"Enabled":true,"names":[]}`, `{"enabled":true,"names":[null]}`} {
		r := httptest.NewRequest("POST", "/", strings.NewReader(raw))
		r.Header.Set("Content-Type", "application/json")
		if _, e := decode[sample](r, true); e == nil {
			t.Fatalf("accepted %s", raw)
		}
	}
	r := httptest.NewRequest("POST", "/", strings.NewReader(`{"enabled":false,"names":[]}`))
	r.Header.Set("Content-Type", "application/json")
	if _, e := decode[sample](r, true); e != nil {
		t.Fatal(e)
	}
}
func TestBodyLimitAndMediaType(t *testing.T) {
	for _, tc := range []struct {
		body, media string
		want        int
	}{{`{}`, "text/plain", 415}, {strings.Repeat(" ", 2*1024*1024+1), "application/json", 413}} {
		r := httptest.NewRequest("POST", "/", strings.NewReader(tc.body))
		r.Header.Set("Content-Type", tc.media)
		_, e := decode[map[string]any](r, true)
		if e == nil || grant.ErrorStatus(e) != tc.want {
			t.Fatalf("%v", e)
		}
	}
}
func TestLearnerWriteIsUnavailable(t *testing.T) {
	s := testServer()
	s.learner = true
	r := httptest.NewRequest("POST", "/api/v1/admin/providers", nil)
	w := httptest.NewRecorder()
	s.ServeHTTP(w, r)
	if w.Code != 503 {
		t.Fatal(w.Code)
	}
}
func TestProxyUnavailableForPatchData(t *testing.T) {
	s := testServer()
	r := httptest.NewRequest("GET", "/auth/v1/oidc/authorize", nil)
	w := httptest.NewRecorder()
	s.ServeHTTP(w, r)
	if w.Code != 404 {
		t.Fatal(w.Code)
	}
}
func TestBearerAndConsistency(t *testing.T) {
	for _, v := range []string{"Basic abc", "BEARER abc", "Bearer   ", " "} {
		r := httptest.NewRequest("GET", "/", nil)
		r.Header.Set("Authorization", v)
		if _, e := bearerToken(r); e == nil {
			t.Fatalf("accepted %q", v)
		}
	}
	r := httptest.NewRequest("GET", "/?consistency=local", nil)
	if _, e := readConsistency(r); e == nil {
		t.Fatal("accepted local")
	}
	r = httptest.NewRequest("GET", "/", nil)
	r.Header.Set("Authorization", "bearer exact-token")
	v, e := bearerToken(r)
	if e != nil || v != "bearer exact-token" {
		t.Fatal(v, e)
	}
}
func TestProxyDoesNotFollowRedirectOrLeakTransport(t *testing.T) {
	up := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		w.Header().Add("Set-Cookie", "a=1")
		w.Header().Add("Set-Cookie", "b=2")
		http.Redirect(w, r, "http://unreachable.invalid/", 302)
	}))
	defer up.Close()
	s := New(grant.NewService(grant.Config{Auth: grant.AuthConfig{ProviderMode: "rauthy", Rauthy: grant.RauthyConfig{BaseURL: up.URL}}}, nil), false)
	r := httptest.NewRequest("GET", "/auth/v1/oidc", nil)
	w := httptest.NewRecorder()
	s.ServeHTTP(w, r)
	if w.Code != 302 || len(w.Header().Values("Set-Cookie")) != 2 {
		t.Fatalf("%d %v", w.Code, w.Header())
	}
}

var retiredExposureRoutes = []string{
	"/api/v1/exposure-authz/read",
	"/api/v1/exposure-authz/plant-read",
	"/api/v2/exposure-authz/decide",
}

func TestRouteMethodsAfterExposureRetirement(t *testing.T) {
	raw, err := os.ReadFile("testdata/routes-voter.txt")
	if err != nil {
		t.Fatal(err)
	}
	s := testServer()
	actual := map[string]bool{}
	for _, p := range s.patterns {
		actual[p] = true
	}
	for _, p := range []string{"GET /healthz", "GET /readyz", "GET /auth", "ANY /auth/v1", "ANY /auth/v1/{*rauthyPath}"} {
		actual[p] = true
	}
	for _, line := range strings.Split(strings.TrimSpace(string(raw)), "\n") {
		if !actual[line] {
			t.Errorf("missing baseline method/path %s", line)
		}
		delete(actual, line)
	}
	if len(actual) != 0 {
		t.Errorf("unexpected method/path %v", actual)
	}
	r := httptest.NewRequest("POST", "/sql/query", strings.NewReader(`{}`))
	w := httptest.NewRecorder()
	s.ServeHTTP(w, r)
	if w.Code != 404 {
		t.Fatalf("private SQL exposed on public router: %d", w.Code)
	}
}

func TestRetiredExposureRoutesAreNotFound(t *testing.T) {
	for _, learner := range []bool{false, true} {
		s := testServer()
		s.learner = learner
		for _, path := range retiredExposureRoutes {
			r := httptest.NewRequest("POST", path, strings.NewReader(`{}`))
			r.Header.Set("Content-Type", "application/json")
			w := httptest.NewRecorder()
			s.ServeHTTP(w, r)
			if w.Code != 404 {
				t.Errorf("learner=%v %s: %d %s", learner, path, w.Code, w.Body.String())
			}
		}
	}
}

func TestGenericAuthorizationAfterExposureRetirement(t *testing.T) {
	ctx := context.Background()
	cfg := grant.Config{Security: grant.SecurityConfig{MasterKey: "master-secret"}, Database: grant.DatabaseConfig{DataDir: t.TempDir(), DefaultReadConsistency: grant.ReadStrong}}
	store, err := grant.OpenStore(ctx, cfg)
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() { _ = store.Close() })
	if err := store.EnsureBootstrapData(ctx); err != nil {
		t.Fatal(err)
	}
	s := New(grant.NewService(cfg, store), false)
	for _, tc := range []struct {
		user    string
		allowed bool
	}{{"conalog-grant", true}, {"unknown-user", false}} {
		r := httptest.NewRequest("POST", "/api/v1/authz/check?consistency=strong", strings.NewReader(`{"subject":{"type":"user","id":"`+tc.user+`","orgId":"conalog-grant"},"action":"organization.profile.read","resource":{"provider":"grant","type":"organization","id":"conalog-grant"}}`))
		r.Header.Set("Content-Type", "application/json")
		r.Header.Set("X-Grant-Master-Key", "master-secret")
		w := httptest.NewRecorder()
		s.ServeHTTP(w, r)
		var decision grant.AuthorizationDecision
		if w.Code != http.StatusOK || json.Unmarshal(w.Body.Bytes(), &decision) != nil || decision.Allowed != tc.allowed {
			t.Errorf("%s: %d %s", tc.user, w.Code, w.Body.String())
		}
	}
}
func TestRustEnumRejection(t *testing.T) {
	r := httptest.NewRequest("POST", "/", strings.NewReader(`{"id":"a","slug":"a","name":"a","applicationType":"bogus","appType":"web","active":true}`))
	r.Header.Set("Content-Type", "application/json")
	_, e := decode[grant.CreateApplicationInput](r, false)
	if e == nil || grant.ErrorStatus(e) != 422 {
		t.Fatal(e)
	}
}

func TestMalformedMutationDoesNotAuthenticate(t *testing.T) {
	hits := 0
	up := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) { hits++; w.WriteHeader(500) }))
	defer up.Close()
	s := New(grant.NewService(grant.Config{Auth: grant.AuthConfig{ProviderMode: "patch-data", PatchData: &grant.PatchDataConfig{BaseURL: up.URL}}}, nil), false)
	for _, tc := range []struct {
		raw  string
		want int
	}{{`{"id":`, 400}, {`{}`, 422}, {`{"id":null,"name":"g"}`, 422}} {
		r := httptest.NewRequest("POST", "/api/v1/orgs/o/groups", strings.NewReader(tc.raw))
		r.Header.Set("Content-Type", "application/json")
		r.Header.Set("Authorization", "Bearer token")
		r.Header.Set("X-Organization-ID", "o")
		w := httptest.NewRecorder()
		s.ServeHTTP(w, r)
		if w.Code != tc.want {
			t.Fatalf("%d %s", w.Code, w.Body.String())
		}
	}
	if hits != 0 {
		t.Fatalf("malformed body performed %d identity requests", hits)
	}
}
func TestInactivePatchDataSelectionBoundary(t *testing.T) {
	s := testServer()
	for _, tc := range []struct {
		org  string
		want int
	}{{"", 400}, {"other", 403}} {
		r := httptest.NewRequest("POST", "/api/v1/orgs/o/reactivate", nil)
		r.Header.Set("Authorization", "Bearer token")
		if tc.org != "" {
			r.Header.Set("X-Organization-ID", tc.org)
		}
		w := httptest.NewRecorder()
		s.ServeHTTP(w, r)
		if w.Code != tc.want {
			t.Fatalf("%d %s", w.Code, w.Body.String())
		}
	}
}
func TestLearnerWildcardAdminBoundary(t *testing.T) {
	s := New(grant.NewService(grant.Config{}, nil), true)
	for _, method := range []string{"GET", "POST", "PUT", "DELETE", "OPTIONS", "PATCH"} {
		r := httptest.NewRequest(method, "/api/v1/admin/providers", nil)
		w := httptest.NewRecorder()
		s.ServeHTTP(w, r)
		if w.Code != 503 {
			t.Errorf("%s %d", method, w.Code)
		}
	}
}

func TestJSONUnicodeBoundary(t *testing.T) {
	type input struct {
		Name string `json:"name"`
	}
	for _, raw := range []string{`{"name":"\ud800"}`, `{"name":"\udc00"}`, `{"name":"\ud800x"}`, `{"name":"ok","unknown":"\ud800"}`, "{\"name\":\"" + string([]byte{0xff}) + "\"}"} {
		req := httptest.NewRequest("POST", "/", strings.NewReader(raw))
		req.Header.Set("Content-Type", "application/json")
		if _, err := decode[input](req, false); err == nil || grant.ErrorStatus(err) != 400 {
			t.Fatalf("accepted invalid Unicode %q: %v", raw, err)
		}
	}
	for _, raw := range []string{`{"name":"\ud83d\ude00"}`, `{"name":"\\ud800"}`, `{"name":"ok","unknown":null}`} {
		req := httptest.NewRequest("POST", "/", strings.NewReader(raw))
		req.Header.Set("Content-Type", "application/json")
		if _, err := decode[input](req, false); err != nil {
			t.Fatalf("rejected valid Unicode %q: %v", raw, err)
		}
	}
}
