package grant

import (
	"crypto/sha256"
	"encoding/hex"
	"fmt"
	"net/http"
	"net/http/httptest"
	"strings"
	"testing"
)

func TestGoauthyIdentityBindingUsesAuthenticatedBootstrapConfiguration(t *testing.T) {
	ctx, s := serviceFixture(t)
	t.Setenv("HTTPS_PROXY", "http://127.0.0.1:1")
	t.Setenv("HTTP_PROXY", "http://127.0.0.1:1")
	calls := 0
	body := `{"version":"goauthy.client-configuration.v1","issuer":"ISSUER","client_id":"operable-bootstrap","client_kind":"bootstrap","enabled":true,"confidential":true,"redirect_uris":[],"grant_types":["authorization_code","refresh_token","client_credentials","urn:ietf:params:oauth:grant-type:device_code"],"scopes":["openid","grant.read","other"],"default_scopes":[],"audience":["https://operable.example.test/grant","https://other.example.test"],"default_aud":["https://operable.example.test/grant"]}`
	issuer := ""
	up := httptest.NewTLSServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		calls++
		if r.Method != http.MethodGet || r.URL.Path != "/oidc/client-configuration" || r.URL.RawQuery != "" {
			t.Errorf("unexpected request: %s %s", r.Method, r.URL)
		}
		if r.Header.Get("Accept-Encoding") != "identity" {
			t.Errorf("Accept-Encoding=%q want identity", r.Header.Get("Accept-Encoding"))
		}
		user, pass, ok := r.BasicAuth()
		if !ok || user != "operable-bootstrap" || pass != "bootstrap-secret" {
			t.Errorf("unexpected Basic credentials: user=%q present=%v", user, ok)
		}
		responseBody := strings.ReplaceAll(body, "ISSUER", issuer)
		digest := sha256.Sum256([]byte(responseBody))
		w.Header().Set("Content-Type", "application/json; charset=utf-8")
		w.Header().Set("ETag", fmt.Sprintf(`"sha256:%s"`, hex.EncodeToString(digest[:])))
		w.Header().Set("Cache-Control", "no-store")
		_, _ = w.Write([]byte(responseBody))
	}))
	defer up.Close()
	issuer = up.URL
	s.client = up.Client()
	s.cfg.Auth.ProviderMode = "goauthy"
	s.cfg.Auth.Goauthy = &GoauthyConfig{Issuer: up.URL, IntrospectionClientID: "operable-bootstrap", IntrospectionClientSecret: "bootstrap-secret", ExpectedClientID: "operable-bootstrap", Resource: "https://operable.example.test/grant", ApplicationID: "client", RequiredScopes: []string{"openid", "grant.read"}}
	if _, err := s.CreateApplication(ctx, "owner", "owner", CreateApplicationInput{ID: "client", Slug: "client", Name: "Operable", ApplicationType: "confidential", AppType: "server", Active: true}); err != nil {
		t.Fatal(err)
	}
	if _, err := s.BindApplicationIdentityAdmin(ctx, "owner", "client", IdentityBindingInput{ClientID: "operable-bootstrap"}); err != nil {
		t.Fatal(err)
	}
	app, err := s.store.GetApplicationWithConsistency(ctx, "client", ReadStrong)
	if err != nil || app.ProvisioningStatus != "ready" || ptrValue(app.RauthyClientID) != "operable-bootstrap" || calls != 1 {
		t.Fatalf("verified bootstrap not bound: %#v calls=%d err=%v", app, calls, err)
	}
}

func TestGoauthyIdentityBindingFailsClosed(t *testing.T) {
	ctx, s := serviceFixture(t)
	mode := "valid"
	good := `{"version":"goauthy.client-configuration.v1","issuer":"ISSUER","client_id":"bootstrap","client_kind":"bootstrap","enabled":true,"confidential":true,"redirect_uris":[],"grant_types":["authorization_code","refresh_token","client_credentials"],"scopes":["openid","grant.read"],"default_scopes":["openid"],"audience":["https://grant.example.test/"],"default_aud":["https://grant.example.test/"]}`
	issuer := ""
	up := httptest.NewTLSServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		if r.Method != http.MethodGet || r.URL.Path != "/oidc/client-configuration" {
			t.Errorf("unexpected endpoint %s %s", r.Method, r.URL)
		}
		body := strings.ReplaceAll(good, "ISSUER", issuer)
		switch mode {
		case "not-found":
			http.NotFound(w, r)
			return
		case "unavailable":
			w.WriteHeader(http.StatusServiceUnavailable)
			return
		case "redirect":
			http.Redirect(w, r, "/other", http.StatusTemporaryRedirect)
			return
		case "wrong-etag":
			body += " "
		case "wrong-version":
			body = strings.Replace(body, "goauthy.client-configuration.v1", "goauthy.client-configuration.v2", 1)
		case "wrong-issuer":
			body = strings.Replace(body, issuer, "https://other.example.test", 1)
		case "wrong-id":
			body = strings.Replace(body, `"client_id":"bootstrap"`, `"client_id":"other"`, 1)
		case "wrong-kind":
			body = strings.Replace(body, `"client_kind":"bootstrap"`, `"client_kind":"managed"`, 1)
		case "disabled":
			body = strings.Replace(body, `"enabled":true`, `"enabled":false`, 1)
		case "public":
			body = strings.Replace(body, `"confidential":true`, `"confidential":false`, 1)
		case "wrong-redirect":
			body = strings.Replace(body, `"redirect_uris":[]`, `"redirect_uris":["https://other.example.test/callback"]`, 1)
		case "missing-resource":
			body = strings.Replace(body, `"audience":["https://grant.example.test/"]`, `"audience":["https://other.example.test"]`, 1)
		case "missing-default-resource":
			body = strings.Replace(body, `"default_aud":["https://grant.example.test/"]`, `"default_aud":["https://other.example.test"]`, 1)
		case "missing-scope":
			body = strings.Replace(body, `"scopes":["openid","grant.read"]`, `"scopes":["openid"]`, 1)
		case "missing-grant-type":
			body = strings.Replace(body, `"grant_types":["authorization_code","refresh_token","client_credentials"]`, `"grant_types":["client_credentials"]`, 1)
		case "default-scope-not-allowed":
			body = strings.Replace(body, `"default_scopes":["openid"]`, `"default_scopes":["other"]`, 1)
		case "duplicate-field":
			body = strings.Replace(body, `"enabled":true`, `"enabled":true,"enabled":true`, 1)
		case "extra-field":
			body = strings.TrimSuffix(body, "}") + `,"secret":"not allowed"}`
		case "revision":
			body = strings.TrimSuffix(body, "}") + `,"revision":1}`
		case "bad-array":
			body = strings.Replace(body, `"scopes":["openid","grant.read"]`, `"scopes":null`, 1)
		case "bad-default-array":
			body = strings.Replace(body, `"default_aud":["https://grant.example.test/"]`, `"default_aud":"https://grant.example.test/"`, 1)
		case "duplicate-scope":
			body = strings.Replace(body, `"scopes":["openid","grant.read"]`, `"scopes":["openid","grant.read","grant.read"]`, 1)
		case "bad-content-type":
			w.Header().Set("Content-Type", "text/plain")
		case "missing-content-type":
			w.Header().Del("Content-Type")
		}
		digest := sha256.Sum256([]byte(body))
		etag := fmt.Sprintf(`"sha256:%s"`, hex.EncodeToString(digest[:]))
		if mode == "wrong-etag" {
			etag = `"sha256:incorrect"`
		}
		if mode != "bad-content-type" && mode != "missing-content-type" {
			w.Header().Set("Content-Type", "application/json")
		}
		w.Header().Set("ETag", etag)
		w.Header().Set("Cache-Control", "no-store")
		_, _ = w.Write([]byte(body))
	}))
	defer up.Close()
	issuer = up.URL
	s.client = up.Client()
	s.cfg.Auth.ProviderMode = "goauthy"
	s.cfg.Auth.Goauthy = &GoauthyConfig{Issuer: up.URL, IntrospectionClientID: "bootstrap", IntrospectionClientSecret: "secret", ExpectedClientID: "bootstrap", Resource: "https://grant.example.test/", ApplicationID: "client", RequiredScopes: []string{"openid", "grant.read"}}
	if _, err := s.CreateApplication(ctx, "owner", "owner", CreateApplicationInput{ID: "client", Slug: "client", Name: "App", ApplicationType: "confidential", AppType: "server", Active: true}); err != nil {
		t.Fatal(err)
	}
	for _, tc := range []struct {
		name string
		want int
	}{
		{"wrong-etag", 503}, {"wrong-version", 409}, {"wrong-issuer", 409}, {"wrong-id", 409}, {"wrong-kind", 409}, {"disabled", 409}, {"public", 409}, {"wrong-redirect", 409}, {"missing-resource", 409}, {"missing-default-resource", 409}, {"missing-scope", 409}, {"missing-grant-type", 409}, {"default-scope-not-allowed", 503}, {"duplicate-field", 503}, {"duplicate-scope", 503}, {"extra-field", 503}, {"revision", 503}, {"bad-array", 503}, {"bad-default-array", 503}, {"bad-content-type", 503}, {"missing-content-type", 503}, {"not-found", 503}, {"unavailable", 503}, {"redirect", 503},
	} {
		t.Run(tc.name, func(t *testing.T) {
			mode = tc.name
			_, err := s.BindApplicationIdentityAdmin(ctx, "owner", "client", IdentityBindingInput{ClientID: "bootstrap"})
			if ErrorStatus(err) != tc.want {
				t.Fatalf("status=%d want=%d err=%v", ErrorStatus(err), tc.want, err)
			}
		})
	}
	app, err := s.store.GetApplicationWithConsistency(ctx, "client", ReadStrong)
	if err != nil || app.ProvisioningStatus != "unprovisioned" || app.RauthyClientID != nil {
		t.Fatalf("rejected metadata mutated app: %#v %v", app, err)
	}
}

func TestGoauthyIdentityBindingOnlyVerifiesConfiguredSelfClient(t *testing.T) {
	ctx, s := serviceFixture(t)
	calls := 0
	up := httptest.NewTLSServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) { calls++ }))
	defer up.Close()
	s.client = up.Client()
	s.cfg.Auth.ProviderMode = "goauthy"
	s.cfg.Auth.Goauthy = &GoauthyConfig{Issuer: up.URL, IntrospectionClientID: "bootstrap", IntrospectionClientSecret: "secret", ExpectedClientID: "login", Resource: "https://grant.example.test/", ApplicationID: "client", RequiredScopes: []string{"openid"}}
	if _, err := s.CreateApplication(ctx, "owner", "owner", CreateApplicationInput{ID: "client", Slug: "client", Name: "App", ApplicationType: "confidential", AppType: "server", Active: true}); err != nil {
		t.Fatal(err)
	}
	_, err := s.BindApplicationIdentityAdmin(ctx, "owner", "client", IdentityBindingInput{ClientID: "bootstrap"})
	if ErrorStatus(err) != 409 || calls != 0 {
		t.Fatalf("non-self client must be rejected before request: calls=%d err=%v", calls, err)
	}
	s.cfg.Auth.Goauthy.ExpectedClientID = "bootstrap"
	s.cfg.Auth.Goauthy.ApplicationID = "another-grant-app"
	_, err = s.BindApplicationIdentityAdmin(ctx, "owner", "client", IdentityBindingInput{ClientID: "bootstrap"})
	if ErrorStatus(err) != 409 || calls != 0 {
		t.Fatalf("bootstrap identity must be pinned to configured Grant application: calls=%d err=%v", calls, err)
	}
}
