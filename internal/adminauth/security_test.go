package adminauth

import (
	"crypto/sha256"
	"encoding/base64"
	"encoding/json"
	"net/http"
	"net/http/httptest"
	"net/url"
	"strings"
	"testing"
	"time"
)

const (
	testClientID     = "grant-admin"
	testClientSecret = "client-secret-value"
	testAccessToken  = "access-token-value"
	testResource     = "https://grant.example.test/management"
	testPublicURL    = "https://admin.example.test"
)

type issuerFixture struct {
	server          *httptest.Server
	challenge       string
	tokenCalls      int
	tokenResource   string
	expiresIn       int
	tokenErrorBody  string
	clientAuthValid bool
	omitUserinfo    bool
}

func newIssuerFixture(t *testing.T) *issuerFixture {
	t.Helper()
	f := &issuerFixture{expiresIn: 300}
	f.server = httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		switch r.URL.Path {
		case "/.well-known/openid-configuration":
			metadata := map[string]string{
				"issuer": f.server.URL, "authorization_endpoint": f.server.URL + "/authorize", "token_endpoint": f.server.URL + "/token", "userinfo_endpoint": f.server.URL + "/userinfo",
			}
			if f.omitUserinfo {
				delete(metadata, "userinfo_endpoint")
			}
			_ = json.NewEncoder(w).Encode(metadata)
		case "/token":
			f.tokenCalls++
			_ = r.ParseForm()
			f.tokenResource = r.PostForm.Get("resource")
			user, pass, ok := r.BasicAuth()
			f.clientAuthValid = ok && user == testClientID && pass == testClientSecret
			verifier := r.PostForm.Get("code_verifier")
			hash := sha256.Sum256([]byte(verifier))
			if !f.clientAuthValid || base64.RawURLEncoding.EncodeToString(hash[:]) != f.challenge || r.PostForm.Get("grant_type") != "authorization_code" || f.tokenResource != "" {
				http.Error(w, "bad exchange", http.StatusBadRequest)
				return
			}
			if f.tokenErrorBody != "" {
				w.WriteHeader(http.StatusBadGateway)
				_, _ = w.Write([]byte(f.tokenErrorBody))
				return
			}
			_ = json.NewEncoder(w).Encode(map[string]any{"access_token": testAccessToken, "token_type": "Bearer", "expires_in": f.expiresIn, "id_token": "must-not-be-trusted"})
		case "/userinfo":
			if r.Header.Get("Authorization") != "Bearer "+testAccessToken {
				http.Error(w, "unauthorized", http.StatusUnauthorized)
				return
			}
			_ = json.NewEncoder(w).Encode(map[string]string{"sub": "user-123", "email": "admin@example.test", "name": "Admin User"})
		default:
			http.NotFound(w, r)
		}
	}))
	t.Cleanup(f.server.Close)
	return f
}

func assertSecurityHeaders(t *testing.T, w *httptest.ResponseRecorder) {
	t.Helper()
	if w.Header().Get("Cache-Control") != "no-store" || w.Header().Get("Referrer-Policy") != "same-origin" || w.Header().Get("Content-Security-Policy") != "frame-ancestors 'none'" {
		t.Fatalf("missing response security headers: %v", w.Header())
	}
}

func (f *issuerFixture) handler(t *testing.T, next http.Handler) http.Handler {
	t.Helper()
	h, err := New(Config{Issuer: f.server.URL, ClientID: testClientID, ClientSecret: testClientSecret, Resource: testResource, PublicURL: testPublicURL}, next)
	if err != nil {
		t.Fatal(err)
	}
	return h
}

func beginLogin(t *testing.T, h http.Handler, f *issuerFixture) (*url.URL, *http.Cookie) {
	t.Helper()
	w := httptest.NewRecorder()
	h.ServeHTTP(w, httptest.NewRequest(http.MethodGet, "/login", nil))
	if w.Code != http.StatusFound {
		t.Fatalf("login status=%d body=%q", w.Code, w.Body.String())
	}
	assertSecurityHeaders(t, w)
	location, err := url.Parse(w.Header().Get("Location"))
	if err != nil {
		t.Fatal(err)
	}
	f.challenge = location.Query().Get("code_challenge")
	if location.Query().Get("resource") != testResource || location.Query().Get("code_challenge_method") != "S256" {
		t.Fatalf("authorization parameters missing: %v", location.Query())
	}
	if strings.Contains(w.Header().Get("Location"), testClientSecret) || strings.Contains(w.Header().Get("Location"), testAccessToken) {
		t.Fatal("credentials appeared in authorization URL")
	}
	var cookie *http.Cookie
	for _, candidate := range w.Result().Cookies() {
		if candidate.Name == loginCookie {
			cookie = candidate
		}
	}
	if cookie == nil || !cookie.HttpOnly || cookie.SameSite != http.SameSiteLaxMode || !cookie.Secure {
		t.Fatalf("OAuth transaction cookie has unsafe attributes: %#v", cookie)
	}
	return location, cookie
}

func completeLogin(t *testing.T, h http.Handler, f *issuerFixture, state, issuer string, cookie *http.Cookie) *http.Cookie {
	t.Helper()
	query := url.Values{"code": {"one-use-code"}, "state": {state}}
	if issuer != "" {
		query.Set("iss", issuer)
	}
	req := httptest.NewRequest(http.MethodGet, "/oauth/callback?"+query.Encode(), nil)
	req.AddCookie(cookie)
	w := httptest.NewRecorder()
	h.ServeHTTP(w, req)
	if w.Code != http.StatusSeeOther {
		t.Fatalf("callback status=%d body=%q", w.Code, w.Body.String())
	}
	assertSecurityHeaders(t, w)
	for _, candidate := range w.Result().Cookies() {
		if candidate.Name == sessionCookie {
			if !candidate.HttpOnly || candidate.SameSite != http.SameSiteLaxMode || !candidate.Secure {
				t.Fatalf("session cookie has unsafe attributes: %#v", candidate)
			}
			return candidate
		}
	}
	t.Fatal("session cookie missing")
	return nil
}

func TestAdminAuthLoginSessionCSRFLogoutAndResource(t *testing.T) {
	f := newIssuerFixture(t)
	var gotToken, gotCSRF string
	next := http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		gotToken = AccessToken(r.Context())
		gotCSRF = CSRFToken(r.Context())
		if r.Method == http.MethodPost && r.Form.Get("payload") != "kept" {
			t.Errorf("parsed form was not preserved for handler: %#v", r.Form)
		}
		_, _ = w.Write([]byte("admin page"))
	})
	h := f.handler(t, next)

	loginURL, transactionCookie := beginLogin(t, h, f)
	sessionCookieValue := completeLogin(t, h, f, loginURL.Query().Get("state"), f.server.URL, transactionCookie)
	if f.tokenCalls != 1 || !f.clientAuthValid || f.tokenResource != "" {
		t.Fatalf("exchange not configured as expected: calls=%d clientAuth=%v token resource=%q", f.tokenCalls, f.clientAuthValid, f.tokenResource)
	}
	if got := sessionCookieValue.Value; got == transactionCookie.Value {
		t.Fatal("login did not rotate the opaque session identifier")
	}

	w := httptest.NewRecorder()
	req := httptest.NewRequest(http.MethodGet, "/", nil)
	req.AddCookie(sessionCookieValue)
	h.ServeHTTP(w, req)
	if w.Code != http.StatusOK || w.Body.String() != "admin page" || gotToken != testAccessToken || gotCSRF == "" {
		t.Fatalf("authenticated request failed: status=%d body=%q token=%q csrf-empty=%v", w.Code, w.Body.String(), gotToken, gotCSRF == "")
	}
	assertSecurityHeaders(t, w)
	if strings.Contains(w.Body.String(), testAccessToken) || strings.Contains(w.Header().Get("Location"), testAccessToken) {
		t.Fatal("access token was rendered to the browser")
	}

	form := url.Values{"csrf_token": {gotCSRF}, "payload": {"kept"}}
	post := httptest.NewRequest(http.MethodPost, "/mutate", strings.NewReader(form.Encode()))
	post.Header.Set("Content-Type", "application/x-www-form-urlencoded")
	post.Header.Set("Origin", testPublicURL)
	post.AddCookie(sessionCookieValue)
	w = httptest.NewRecorder()
	h.ServeHTTP(w, post)
	if w.Code != http.StatusOK {
		t.Fatalf("valid CSRF post status=%d body=%q", w.Code, w.Body.String())
	}

	form.Set("csrf_token", "wrong")
	post = httptest.NewRequest(http.MethodPost, "/mutate", strings.NewReader(form.Encode()))
	post.Header.Set("Content-Type", "application/x-www-form-urlencoded")
	post.AddCookie(sessionCookieValue)
	w = httptest.NewRecorder()
	h.ServeHTTP(w, post)
	if w.Code != http.StatusForbidden {
		t.Fatalf("invalid CSRF status=%d", w.Code)
	}
	assertSecurityHeaders(t, w)
	queryToken := httptest.NewRequest(http.MethodPost, "/mutate?csrf_token="+url.QueryEscape(gotCSRF), strings.NewReader("payload=kept"))
	queryToken.Header.Set("Content-Type", "application/x-www-form-urlencoded")
	queryToken.AddCookie(sessionCookieValue)
	w = httptest.NewRecorder()
	h.ServeHTTP(w, queryToken)
	if w.Code != http.StatusForbidden {
		t.Fatalf("query string CSRF token accepted: status=%d", w.Code)
	}
	badOrigin := httptest.NewRequest(http.MethodPost, "/mutate", strings.NewReader(url.Values{"csrf_token": {gotCSRF}}.Encode()))
	badOrigin.Header.Set("Content-Type", "application/x-www-form-urlencoded")
	badOrigin.Header.Set("Origin", "https://attacker.example")
	badOrigin.AddCookie(sessionCookieValue)
	w = httptest.NewRecorder()
	h.ServeHTTP(w, badOrigin)
	if w.Code != http.StatusForbidden {
		t.Fatalf("cross-origin POST accepted: status=%d", w.Code)
	}

	logout := httptest.NewRequest(http.MethodPost, "/logout", strings.NewReader(url.Values{"csrf_token": {gotCSRF}}.Encode()))
	logout.Header.Set("Content-Type", "application/x-www-form-urlencoded")
	logout.AddCookie(sessionCookieValue)
	w = httptest.NewRecorder()
	h.ServeHTTP(w, logout)
	if w.Code != http.StatusSeeOther {
		t.Fatalf("logout status=%d", w.Code)
	}
	w = httptest.NewRecorder()
	req = httptest.NewRequest(http.MethodGet, "/", nil)
	req.AddCookie(sessionCookieValue)
	h.ServeHTTP(w, req)
	if w.Code != http.StatusSeeOther || w.Header().Get("Location") != "/login" {
		t.Fatalf("logged-out session remained valid: status=%d location=%q", w.Code, w.Header().Get("Location"))
	}
}

func TestAdminAuthCallbackRejectsInvalidStateIssuerAndLeakyErrors(t *testing.T) {
	for _, tc := range []struct {
		name   string
		state  string
		issuer string
		body   string
	}{
		{name: "state", state: "wrong", issuer: ""},
		{name: "issuer", issuer: "https://attacker.example"},
		{name: "upstream error body", issuer: "", body: "upstream-secret-response"},
	} {
		t.Run(tc.name, func(t *testing.T) {
			f := newIssuerFixture(t)
			f.tokenErrorBody = tc.body
			h := f.handler(t, http.HandlerFunc(func(http.ResponseWriter, *http.Request) {}))
			loginURL, cookie := beginLogin(t, h, f)
			state := loginURL.Query().Get("state")
			if tc.state != "" {
				state = tc.state
			}
			query := url.Values{"code": {"one-use-code"}, "state": {state}}
			if tc.issuer != "" {
				query.Set("iss", tc.issuer)
			}
			req := httptest.NewRequest(http.MethodGet, "/oauth/callback?"+query.Encode(), nil)
			req.AddCookie(cookie)
			w := httptest.NewRecorder()
			h.ServeHTTP(w, req)
			if w.Code != http.StatusUnauthorized || strings.Contains(w.Body.String(), testClientSecret) || strings.Contains(w.Body.String(), testAccessToken) || tc.body != "" && strings.Contains(w.Body.String(), tc.body) {
				t.Fatalf("callback leaked details or accepted bad response: status=%d body=%q", w.Code, w.Body.String())
			}
			if tc.name != "upstream error body" && f.tokenCalls != 0 {
				t.Fatalf("invalid callback reached token endpoint %d times", f.tokenCalls)
			}
		})
	}
}

func TestAdminAuthProtectedRoutesAndSessionExpiry(t *testing.T) {
	f := newIssuerFixture(t)
	f.expiresIn = 1
	h := f.handler(t, http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		_, _ = w.Write([]byte("ok"))
	}))

	for _, path := range []string{"/", "/api", "/secret"} {
		w := httptest.NewRecorder()
		h.ServeHTTP(w, httptest.NewRequest(http.MethodGet, path, nil))
		if w.Code != http.StatusSeeOther || w.Header().Get("Location") != "/login" {
			t.Fatalf("unauthenticated %s status=%d location=%q", path, w.Code, w.Header().Get("Location"))
		}
		assertSecurityHeaders(t, w)
	}
	w := httptest.NewRecorder()
	htmx := httptest.NewRequest(http.MethodGet, "/", nil)
	htmx.Header.Set("HX-Request", "true")
	h.ServeHTTP(w, htmx)
	if w.Code != http.StatusOK || w.Header().Get("HX-Redirect") != "/login" || w.Header().Get("Location") != "" {
		t.Fatalf("HTMX auth redirect status=%d hx-redirect=%q location=%q", w.Code, w.Header().Get("HX-Redirect"), w.Header().Get("Location"))
	}
	assertSecurityHeaders(t, w)
	for _, path := range []string{"/assets/app.js", "/healthz"} {
		w := httptest.NewRecorder()
		h.ServeHTTP(w, httptest.NewRequest(http.MethodGet, path, nil))
		if w.Code != http.StatusOK {
			t.Fatalf("public route %s status=%d", path, w.Code)
		}
	}
	loginURL, transactionCookie := beginLogin(t, h, f)
	sessionCookieValue := completeLogin(t, h, f, loginURL.Query().Get("state"), "", transactionCookie)
	time.Sleep(1100 * time.Millisecond)
	w = httptest.NewRecorder()
	req := httptest.NewRequest(http.MethodGet, "/", nil)
	req.AddCookie(sessionCookieValue)
	req.Header.Set("HX-Request", "true")
	h.ServeHTTP(w, req)
	if w.Code != http.StatusOK || w.Header().Get("HX-Redirect") != "/login" {
		t.Fatalf("expired HTMX session redirect status=%d hx-redirect=%q", w.Code, w.Header().Get("HX-Redirect"))
	}
	assertSecurityHeaders(t, w)
}

func TestAdminAuthOmitsResourceWhenUsingIssuerDefaultAudience(t *testing.T) {
	f := newIssuerFixture(t)
	h, err := New(Config{Issuer: f.server.URL, ClientID: testClientID, ClientSecret: testClientSecret, PublicURL: testPublicURL}, http.HandlerFunc(func(http.ResponseWriter, *http.Request) {}))
	if err != nil {
		t.Fatal(err)
	}
	w := httptest.NewRecorder()
	h.ServeHTTP(w, httptest.NewRequest(http.MethodGet, "/login", nil))
	location, err := url.Parse(w.Header().Get("Location"))
	if err != nil || w.Code != http.StatusFound {
		t.Fatalf("login failed: status=%d err=%v", w.Code, err)
	}
	if location.Query().Has("resource") || location.Query().Get("code_challenge_method") != "S256" {
		t.Fatalf("unexpected authorization parameters: %v", location.Query())
	}
}

func TestAdminAuthRejectsUnsafeConfiguration(t *testing.T) {
	f := newIssuerFixture(t)
	for _, cfg := range []Config{
		{Issuer: "http://example.test", ClientID: testClientID, ClientSecret: testClientSecret, PublicURL: testPublicURL},
		{Issuer: f.server.URL + "?x=1", ClientID: testClientID, ClientSecret: testClientSecret, PublicURL: testPublicURL},
		{Issuer: f.server.URL, ClientID: testClientID, ClientSecret: testClientSecret, PublicURL: "https://admin.example.test/path"},
		{Issuer: f.server.URL, ClientID: testClientID, ClientSecret: testClientSecret, PublicURL: "https://user:pass@admin.example.test"},
		{Issuer: f.server.URL, ClientID: testClientID, ClientSecret: testClientSecret, Resource: "http://resource.example.test", PublicURL: testPublicURL},
	} {
		if _, err := New(cfg, http.HandlerFunc(func(http.ResponseWriter, *http.Request) {})); err == nil {
			t.Fatalf("unsafe configuration accepted: %+v", cfg)
		}
	}
}

func TestAdminAuthStoresUserinfoInSession(t *testing.T) {
	f := newIssuerFixture(t)
	var gotSubject, gotEmail, gotName string
	h, err := New(Config{Issuer: f.server.URL, ClientID: testClientID, ClientSecret: testClientSecret, PublicURL: testPublicURL}, http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		gotSubject = Subject(r.Context())
		gotEmail = Email(r.Context())
		gotName = Name(r.Context())
	}))
	if err != nil {
		t.Fatal(err)
	}
	login := httptest.NewRecorder()
	h.ServeHTTP(login, httptest.NewRequest(http.MethodGet, "/login", nil))
	location, err := url.Parse(login.Header().Get("Location"))
	if err != nil || login.Code != http.StatusFound {
		t.Fatalf("login failed: status=%d err=%v", login.Code, err)
	}
	f.challenge = location.Query().Get("code_challenge")
	callback := httptest.NewRequest(http.MethodGet, "/oauth/callback?"+url.Values{"code": {"test-code"}, "state": {location.Query().Get("state")}}.Encode(), nil)
	for _, cookie := range login.Result().Cookies() {
		callback.AddCookie(cookie)
	}
	result := httptest.NewRecorder()
	h.ServeHTTP(result, callback)
	if result.Code != http.StatusSeeOther {
		t.Fatalf("callback status=%d", result.Code)
	}
	request := httptest.NewRequest(http.MethodGet, "/", nil)
	for _, cookie := range result.Result().Cookies() {
		request.AddCookie(cookie)
	}
	page := httptest.NewRecorder()
	h.ServeHTTP(page, request)
	if page.Code != http.StatusOK {
		t.Fatalf("page status=%d", page.Code)
	}
	if gotSubject != "user-123" || gotEmail != "admin@example.test" || gotName != "Admin User" {
		t.Fatalf("unexpected userinfo: subject=%q email=%q name=%q", gotSubject, gotEmail, gotName)
	}
}

func TestAdminAuthLoginWithoutUserinfoMetadata(t *testing.T) {
	f := newIssuerFixture(t)
	f.omitUserinfo = true
	called := false
	h := f.handler(t, http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		called = true
		if AccessToken(r.Context()) != testAccessToken || Subject(r.Context()) != "" || Email(r.Context()) != "" || Name(r.Context()) != "" {
			t.Error("unexpected session without optional userinfo")
		}
	}))
	location, transaction := beginLogin(t, h, f)
	cookie := completeLogin(t, h, f, location.Query().Get("state"), "", transaction)
	req := httptest.NewRequest(http.MethodGet, "/", nil)
	req.AddCookie(cookie)
	w := httptest.NewRecorder()
	h.ServeHTTP(w, req)
	if w.Code != http.StatusOK || !called {
		t.Fatalf("authenticated handler not reached: status=%d", w.Code)
	}
}
