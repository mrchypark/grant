// Package adminauth provides the browser session boundary for Grant's admin UI.
package adminauth

import (
	"context"
	"crypto/rand"
	"crypto/sha256"
	"crypto/subtle"
	"encoding/base64"
	"encoding/json"
	"errors"
	"fmt"
	"io"
	"net"
	"net/http"
	"net/url"
	"strings"
	"sync"
	"time"
	"unicode"
)

const (
	maxResponseBytes = 64 << 10
	maxFormBytes     = 1 << 20
	maxSessions      = 5000
	maxLogins        = 1000
	sessionLifetime  = 8 * time.Hour
	loginLifetime    = 10 * time.Minute
	sessionCookie    = "grant_admin_session"
	loginCookie      = "grant_admin_login"
)

type Config struct {
	Issuer       string
	ClientID     string
	ClientSecret string
	// Resource is the optional static OAuth resource indicator sent on the authorization request.
	Resource   string
	PublicURL  string
	Scopes     []string
	HTTPClient *http.Client
}

type endpoints struct {
	authorize string
	token     string
}

type session struct {
	token   string
	csrf    string
	expires time.Time
}

type login struct {
	state      string
	verifier   string
	oldSession string
	expires    time.Time
}

type server struct {
	cfg       Config
	issuer    *url.URL
	public    *url.URL
	endpoints endpoints
	client    *http.Client
	mu        sync.Mutex
	sessions  map[string]session
	logins    map[string]login
}

type contextKey uint8

const (
	tokenKey contextKey = iota
	csrfKey
)

// AccessToken returns the session's OAuth access token, if authenticated.
func AccessToken(ctx context.Context) string {
	token, _ := ctx.Value(tokenKey).(string)
	return token
}

// CSRFToken returns the session's hidden-form token, if authenticated.
func CSRFToken(ctx context.Context) string {
	token, _ := ctx.Value(csrfKey).(string)
	return token
}

func New(cfg Config, next http.Handler) (http.Handler, error) {
	if next == nil || strings.TrimSpace(cfg.ClientID) == "" || strings.TrimSpace(cfg.ClientSecret) == "" {
		return nil, errors.New("admin auth requires a handler and OAuth client credentials")
	}
	issuer, err := parseAbsoluteURL(cfg.Issuer)
	if err != nil || !secureOrLoopback(issuer) || issuer.RawQuery != "" || issuer.ForceQuery || issuer.Fragment != "" {
		return nil, errors.New("invalid OAuth issuer URL")
	}
	public, err := parseAbsoluteURL(cfg.PublicURL)
	if err != nil || !secureOrLoopback(public) || public.RawQuery != "" || public.ForceQuery || public.Fragment != "" || (public.Path != "" && public.Path != "/") {
		return nil, errors.New("invalid public URL")
	}
	if cfg.Resource != "" {
		resource, err := url.ParseRequestURI(cfg.Resource)
		if err != nil || len(cfg.Resource) > 2048 || strings.ContainsFunc(cfg.Resource, unicode.IsSpace) || !resource.IsAbs() || resource.Scheme != "https" || resource.Hostname() == "" || resource.User != nil || resource.RawQuery != "" || resource.ForceQuery || resource.Fragment != "" || strings.Contains(cfg.Resource, "#") {
			return nil, errors.New("invalid OAuth resource URL")
		}
	}
	public.Path, public.RawPath = "", ""
	if len(cfg.Scopes) == 0 {
		cfg.Scopes = []string{"openid", "profile", "email", "goauthy.read"}
	}
	client := safeHTTPClient(cfg.HTTPClient)
	s := &server{cfg: cfg, issuer: issuer, public: public, client: client, sessions: make(map[string]session), logins: make(map[string]login)}
	if err := s.discover(context.Background()); err != nil {
		return nil, fmt.Errorf("OAuth discovery failed: %w", err)
	}
	return http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) { s.serve(next, w, r) }), nil
}

func parseAbsoluteURL(value string) (*url.URL, error) {
	u, err := url.Parse(strings.TrimSpace(value))
	if err != nil || u == nil || !u.IsAbs() || u.Host == "" || u.User != nil || u.Opaque != "" {
		return nil, errors.New("URL must be absolute and have no credentials")
	}
	return u, nil
}

func secureOrLoopback(u *url.URL) bool {
	if strings.EqualFold(u.Scheme, "https") {
		return true
	}
	if !strings.EqualFold(u.Scheme, "http") {
		return false
	}
	host := u.Hostname()
	return strings.EqualFold(host, "localhost") || net.ParseIP(host) != nil && net.ParseIP(host).IsLoopback()
}

func safeHTTPClient(input *http.Client) *http.Client {
	var client http.Client
	if input != nil {
		client = *input
	}
	client.Timeout = 10 * time.Second
	if client.Transport == nil {
		transport := http.DefaultTransport.(*http.Transport).Clone()
		transport.Proxy = nil
		client.Transport = transport
	} else if transport, ok := client.Transport.(*http.Transport); ok {
		transport = transport.Clone()
		transport.Proxy = nil
		client.Transport = transport
	}
	client.CheckRedirect = func(*http.Request, []*http.Request) error { return http.ErrUseLastResponse }
	return &client
}

func (s *server) discover(ctx context.Context) error {
	u := *s.issuer
	u.Path = strings.TrimRight(u.Path, "/") + "/.well-known/openid-configuration"
	u.RawPath = ""
	req, err := http.NewRequestWithContext(ctx, http.MethodGet, u.String(), nil)
	if err != nil {
		return errors.New("invalid discovery URL")
	}
	req.Header.Set("Accept", "application/json")
	var document struct {
		Issuer                string `json:"issuer"`
		AuthorizationEndpoint string `json:"authorization_endpoint"`
		TokenEndpoint         string `json:"token_endpoint"`
	}
	if err := s.getJSON(req, &document); err != nil {
		return errors.New("issuer metadata unavailable or invalid")
	}
	if document.Issuer != s.issuer.String() {
		return errors.New("issuer metadata issuer mismatch")
	}
	authorize, err := endpointURL(document.AuthorizationEndpoint, s.issuer)
	if err != nil {
		return errors.New("invalid authorization endpoint")
	}
	token, err := endpointURL(document.TokenEndpoint, s.issuer)
	if err != nil {
		return errors.New("invalid token endpoint")
	}
	s.endpoints = endpoints{authorize: authorize.String(), token: token.String()}
	return nil
}

func endpointURL(value string, issuer *url.URL) (*url.URL, error) {
	u, err := parseAbsoluteURL(value)
	if err != nil || u.RawQuery != "" || u.ForceQuery || u.Fragment != "" || !sameOrigin(u, issuer) {
		return nil, errors.New("endpoint must share issuer origin and have no query or fragment")
	}
	return u, nil
}

func sameOrigin(a, b *url.URL) bool {
	return strings.EqualFold(a.Scheme, b.Scheme) && strings.EqualFold(a.Host, b.Host)
}

func (s *server) getJSON(req *http.Request, out any) error {
	resp, err := s.client.Do(req)
	if err != nil {
		return errors.New("request failed")
	}
	defer resp.Body.Close()
	if resp.StatusCode < 200 || resp.StatusCode >= 300 {
		return errors.New("unexpected response status")
	}
	body, err := io.ReadAll(io.LimitReader(resp.Body, maxResponseBytes+1))
	if err != nil || len(body) > maxResponseBytes || json.Unmarshal(body, out) != nil {
		return errors.New("invalid response body")
	}
	return nil
}

func randomValue(size int) (string, error) {
	b := make([]byte, size)
	if _, err := rand.Read(b); err != nil {
		return "", err
	}
	return base64.RawURLEncoding.EncodeToString(b), nil
}

func (s *server) serve(next http.Handler, w http.ResponseWriter, r *http.Request) {
	w.Header().Set("Cache-Control", "no-store")
	// Preserve Origin on same-origin form POSTs without sending cross-origin referrers.
	w.Header().Set("Referrer-Policy", "same-origin")
	w.Header().Set("Content-Security-Policy", "frame-ancestors 'none'")
	switch {
	case r.URL.Path == "/logged-out":
		if r.Method != http.MethodGet {
			http.Error(w, "method not allowed", http.StatusMethodNotAllowed)
			return
		}
		if id, ok := s.sessionCookie(r); ok {
			s.mu.Lock()
			current, exists := s.sessions[id]
			s.mu.Unlock()
			if exists && time.Now().Before(current.expires) {
				http.Redirect(w, r, "/", http.StatusSeeOther)
				return
			}
		}
		w.Header().Set("Content-Type", "text/html; charset=utf-8")
		_, _ = w.Write([]byte(`<!doctype html><html lang="ko"><head><meta charset="utf-8"><meta name="viewport" content="width=device-width, initial-scale=1"><title>로그아웃 · Grant 관리</title><link rel="stylesheet" href="/assets/grant-admin.css"></head><body><main class="workspace"><h1>Grant에서 로그아웃했습니다</h1><p>Grant 관리 세션이 종료되었습니다. Goauthy 로그인 상태는 변경하지 않았습니다.</p><p><a href="/login">다시 로그인</a></p></main></body></html>`))
		return
	case r.URL.Path == "/login":
		if r.Method != http.MethodGet {
			http.Error(w, "method not allowed", http.StatusMethodNotAllowed)
			return
		}
		s.login(w, r)
		return
	case r.URL.Path == "/oauth/callback":
		if r.Method != http.MethodGet {
			http.Error(w, "method not allowed", http.StatusMethodNotAllowed)
			return
		}
		s.callback(w, r)
		return
	case r.URL.Path == "/logout":
		if r.Method != http.MethodPost {
			http.Error(w, "method not allowed", http.StatusMethodNotAllowed)
			return
		}
	case (r.Method == http.MethodGet || r.Method == http.MethodHead) && strings.HasPrefix(r.URL.Path, "/assets/"):
		next.ServeHTTP(w, r)
		return
	case r.URL.Path == "/healthz" && r.Method == http.MethodGet:
		next.ServeHTTP(w, r)
		return
	}
	s.cleanup(time.Now())
	sessionID, ok := s.sessionCookie(r)
	if !ok {
		if r.URL.Path == "/logout" {
			s.finishLogout(w, r)
			return
		}
		redirectToLogin(w, r)
		return
	}
	s.mu.Lock()
	current, ok := s.sessions[sessionID]
	s.mu.Unlock()
	if !ok || !time.Now().Before(current.expires) {
		if r.URL.Path == "/logout" {
			s.finishLogout(w, r)
			return
		}
		redirectToLogin(w, r)
		return
	}
	if r.Method != http.MethodGet && r.Method != http.MethodHead && r.Method != http.MethodOptions {
		if r.Method != http.MethodPost {
			http.Error(w, "method not allowed", http.StatusMethodNotAllowed)
			return
		}
		if !s.validOrigin(r) || !validFormCSRF(w, r, current.csrf) {
			http.Error(w, "forbidden", http.StatusForbidden)
			return
		}
	}
	ctx := context.WithValue(r.Context(), tokenKey, current.token)
	ctx = context.WithValue(ctx, csrfKey, current.csrf)
	r = r.WithContext(ctx)
	if r.URL.Path == "/logout" {
		s.mu.Lock()
		delete(s.sessions, sessionID)
		s.mu.Unlock()
		s.finishLogout(w, r)
		return
	}
	next.ServeHTTP(w, r)
}

func (s *server) finishLogout(w http.ResponseWriter, r *http.Request) {
	s.clearCookie(w, sessionCookie)
	http.Redirect(w, r, "/logged-out", http.StatusSeeOther)
}

func redirectToLogin(w http.ResponseWriter, r *http.Request) {
	if strings.EqualFold(strings.TrimSpace(r.Header.Get("HX-Request")), "true") {
		w.Header().Set("HX-Redirect", "/login")
		w.WriteHeader(http.StatusOK)
		return
	}
	http.Redirect(w, r, "/login", http.StatusSeeOther)
}

func (s *server) login(w http.ResponseWriter, r *http.Request) {
	state, err := randomValue(32)
	if err != nil {
		http.Error(w, "authentication unavailable", http.StatusServiceUnavailable)
		return
	}
	verifier, err := randomValue(32)
	if err != nil {
		http.Error(w, "authentication unavailable", http.StatusServiceUnavailable)
		return
	}
	key, err := randomValue(32)
	if err != nil {
		http.Error(w, "authentication unavailable", http.StatusServiceUnavailable)
		return
	}
	oldSession, _ := s.sessionCookie(r)
	s.mu.Lock()
	s.cleanupLocked(time.Now())
	if len(s.logins) >= maxLogins {
		s.mu.Unlock()
		http.Error(w, "authentication busy", http.StatusServiceUnavailable)
		return
	}
	s.logins[key] = login{state: state, verifier: verifier, oldSession: oldSession, expires: time.Now().Add(loginLifetime)}
	s.mu.Unlock()
	s.setCookie(w, loginCookie, key, loginLifetime)
	hash := sha256.Sum256([]byte(verifier))
	challenge := base64.RawURLEncoding.EncodeToString(hash[:])
	redirect, _ := url.Parse(s.endpoints.authorize)
	q := redirect.Query()
	q.Set("response_type", "code")
	q.Set("client_id", s.cfg.ClientID)
	q.Set("redirect_uri", s.callbackURL())
	q.Set("scope", strings.Join(s.cfg.Scopes, " "))
	q.Set("state", state)
	q.Set("code_challenge", challenge)
	q.Set("code_challenge_method", "S256")
	if s.cfg.Resource != "" {
		q.Set("resource", s.cfg.Resource)
	}
	redirect.RawQuery = q.Encode()
	http.Redirect(w, r, redirect.String(), http.StatusFound)
}

func (s *server) callbackURL() string {
	return strings.TrimRight(s.public.String(), "/") + "/oauth/callback"
}

func (s *server) callback(w http.ResponseWriter, r *http.Request) {
	query := r.URL.Query()
	values, stateOK := query["state"]
	codes, codeOK := query["code"]
	key, cookieErr := r.Cookie(loginCookie)
	if !stateOK || len(values) != 1 || !codeOK || len(codes) != 1 || codes[0] == "" || cookieErr != nil {
		s.failCallback(w)
		return
	}
	s.mu.Lock()
	transaction, found := s.logins[key.Value]
	delete(s.logins, key.Value)
	s.mu.Unlock()
	s.clearCookie(w, loginCookie)
	if !found || time.Now().After(transaction.expires) || subtle.ConstantTimeCompare([]byte(values[0]), []byte(transaction.state)) != 1 {
		s.failCallback(w)
		return
	}
	if issuers, ok := query["iss"]; ok && (len(issuers) != 1 || issuers[0] != s.issuer.String()) {
		s.failCallback(w)
		return
	}
	if query.Get("error") != "" {
		s.failCallback(w)
		return
	}
	token, expires, err := s.exchange(r.Context(), codes[0], transaction.verifier)
	if err != nil {
		s.failCallback(w)
		return
	}
	sid, err := randomValue(32)
	if err != nil {
		s.failCallback(w)
		return
	}
	csrf, err := randomValue(32)
	if err != nil {
		s.failCallback(w)
		return
	}
	now := time.Now()
	if expires.IsZero() || expires.After(now.Add(sessionLifetime)) {
		expires = now.Add(sessionLifetime)
	}
	if !expires.After(now) {
		s.failCallback(w)
		return
	}
	s.mu.Lock()
	s.cleanupLocked(now)
	if len(s.sessions) >= maxSessions {
		s.mu.Unlock()
		s.failCallback(w)
		return
	}
	delete(s.sessions, transaction.oldSession)
	s.sessions[sid] = session{token: token, csrf: csrf, expires: expires}
	s.mu.Unlock()
	s.setCookie(w, sessionCookie, sid, time.Until(expires))
	http.Redirect(w, r, "/", http.StatusSeeOther)
}

func (s *server) exchange(ctx context.Context, code, verifier string) (string, time.Time, error) {
	form := url.Values{"grant_type": {"authorization_code"}, "code": {code}, "redirect_uri": {s.callbackURL()}, "code_verifier": {verifier}}
	req, err := http.NewRequestWithContext(ctx, http.MethodPost, s.endpoints.token, strings.NewReader(form.Encode()))
	if err != nil {
		return "", time.Time{}, errors.New("token request failed")
	}
	req.SetBasicAuth(s.cfg.ClientID, s.cfg.ClientSecret)
	req.Header.Set("Content-Type", "application/x-www-form-urlencoded")
	req.Header.Set("Accept", "application/json")
	var response struct {
		AccessToken string `json:"access_token"`
		TokenType   string `json:"token_type"`
		ExpiresIn   *int64 `json:"expires_in"`
	}
	if err := s.getJSON(req, &response); err != nil || strings.TrimSpace(response.AccessToken) == "" || !strings.EqualFold(response.TokenType, "bearer") {
		return "", time.Time{}, errors.New("token exchange failed")
	}
	if response.ExpiresIn != nil && (*response.ExpiresIn < 1 || *response.ExpiresIn > int64((time.Duration(1<<63-1))/time.Second)) {
		return "", time.Time{}, errors.New("invalid token expiry")
	}
	var expires time.Time
	if response.ExpiresIn != nil {
		expires = time.Now().Add(time.Duration(*response.ExpiresIn) * time.Second)
	}
	return response.AccessToken, expires, nil
}

func (s *server) sessionCookie(r *http.Request) (string, bool) {
	cookie, err := r.Cookie(sessionCookie)
	if err != nil || cookie.Value == "" {
		return "", false
	}
	s.mu.Lock()
	defer s.mu.Unlock()
	session, ok := s.sessions[cookie.Value]
	return cookie.Value, ok && time.Now().Before(session.expires)
}

func validFormCSRF(w http.ResponseWriter, r *http.Request, expected string) bool {
	r.Body = http.MaxBytesReader(w, r.Body, maxFormBytes)
	if err := r.ParseForm(); err != nil {
		return false
	}
	values := r.PostForm["csrf_token"]
	return len(values) == 1 && len(values[0]) == len(expected) && subtle.ConstantTimeCompare([]byte(values[0]), []byte(expected)) == 1
}

func (s *server) validOrigin(r *http.Request) bool {
	values := r.Header.Values("Origin")
	if len(values) == 0 {
		return true
	}
	if len(values) != 1 {
		return false
	}
	origin, err := parseAbsoluteURL(values[0])
	return err == nil && origin.Path == "" && origin.RawQuery == "" && !origin.ForceQuery && origin.Fragment == "" && sameOrigin(origin, s.public)
}

func (s *server) setCookie(w http.ResponseWriter, name, value string, maxAge time.Duration) {
	http.SetCookie(w, &http.Cookie{Name: name, Value: value, Path: "/", HttpOnly: true, Secure: strings.EqualFold(s.public.Scheme, "https"), SameSite: http.SameSiteLaxMode, MaxAge: int(maxAge.Seconds()), Expires: time.Now().Add(maxAge)})
}

func (s *server) clearCookie(w http.ResponseWriter, name string) {
	http.SetCookie(w, &http.Cookie{Name: name, Value: "", Path: "/", HttpOnly: true, Secure: strings.EqualFold(s.public.Scheme, "https"), SameSite: http.SameSiteLaxMode, MaxAge: -1, Expires: time.Unix(1, 0)})
}

func (s *server) failCallback(w http.ResponseWriter) {
	http.Error(w, "authentication failed", http.StatusUnauthorized)
}

func (s *server) cleanup(now time.Time) {
	s.mu.Lock()
	s.cleanupLocked(now)
	s.mu.Unlock()
}

func (s *server) cleanupLocked(now time.Time) {
	for key, value := range s.sessions {
		if !now.Before(value.expires) {
			delete(s.sessions, key)
		}
	}
	for key, value := range s.logins {
		if !now.Before(value.expires) {
			delete(s.logins, key)
		}
	}
}
