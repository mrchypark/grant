package grant

import (
	"context"
	"fmt"
	"net/http"
	"net/http/httptest"
	"strings"
	"testing"
	"time"
)

func TestGoauthyIdentityValidationAndNoPositiveCache(t *testing.T) {
	response := fmt.Sprintf(`{"active":true,"sub":"user-1","client_id":"ied-login","exp":%d,"aud":["https://grant.example.test/"],"scope":"openid goauthy.read"}`, time.Now().Add(time.Minute).Unix())
	userinfo := `{"sub":"user-1","email":"USER@example.test","email_verified":true,"name":"User"}`
	introspectionCalls := 0
	server := httptest.NewTLSServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		switch r.URL.Path {
		case "/oidc/introspect":
			introspectionCalls++
			id, secret, ok := r.BasicAuth()
			if !ok || id != "introspector" || secret != "secret" || r.Method != http.MethodPost {
				t.Error("bad introspection client credentials")
			}
			if err := r.ParseForm(); err != nil || r.Form.Get("token") != "opaque+/token==" {
				t.Error("bad introspection request")
			}
			fmt.Fprint(w, response)
		case "/oidc/userinfo":
			if r.Header.Get("Authorization") != "Bearer opaque+/token==" {
				t.Error("bad userinfo bearer")
			}
			fmt.Fprint(w, userinfo)
		default:
			w.WriteHeader(http.StatusNotFound)
		}
	}))
	defer server.Close()
	cfg := Config{Auth: AuthConfig{ProviderMode: "goauthy", Goauthy: &GoauthyConfig{Issuer: server.URL, IntrospectionClientID: "introspector", IntrospectionClientSecret: "secret", ExpectedClientID: "ied-login", Resource: "https://grant.example.test/", ApplicationID: "ied-management-data", RequiredScopes: []string{"openid", "goauthy.read"}}}}
	s := NewService(cfg, nil)
	s.client = server.Client()
	identity, err := s.externalIdentity(context.Background(), "bEaReR opaque+/token==")
	if err != nil || identity.Provider != "goauthy" || identity.Issuer != server.URL || identity.CollectionID != "" || identity.ExternalUserID != "user-1" || identity.ClientID != "ied-management-data" || ptrValue(identity.Email) != "user@example.test" {
		t.Fatalf("identity=%+v err=%v", identity, err)
	}
	userinfo = `{"sub":"user-1"}`
	if _, err = s.externalIdentity(context.Background(), "Bearer opaque+/token=="); err != nil || introspectionCalls != 2 {
		t.Fatalf("Goauthy response was cached or rejected: calls=%d err=%v", introspectionCalls, err)
	}
	response = `{"active":false}`
	if _, err = s.externalIdentity(context.Background(), "Bearer opaque+/token=="); ErrorStatus(err) != 401 || introspectionCalls != 3 {
		t.Fatalf("revocation was cached: calls=%d err=%v", introspectionCalls, err)
	}
}

func TestGoauthyIdentityRejectsIntrospectionClaims(t *testing.T) {
	tests := map[string]string{
		"wrong issuer":         `{"active":true,"sub":"u","client_id":"login","exp":4102444800,"aud":["https://grant.example.test/"],"scope":"openid grant","iss":"https://other.example.test"}`,
		"wrong resource":       `{"active":true,"sub":"u","client_id":"login","exp":4102444800,"aud":["https://other.example.test/"],"scope":"openid grant"}`,
		"missing scope":        `{"active":true,"sub":"u","client_id":"login","exp":4102444800,"aud":["https://grant.example.test/"],"scope":"openid"}`,
		"wrong client":         `{"active":true,"sub":"u","client_id":"other","exp":4102444800,"aud":["https://grant.example.test/"],"scope":"openid grant"}`,
		"expired":              `{"active":true,"sub":"u","client_id":"login","exp":1,"aud":["https://grant.example.test/"],"scope":"openid grant"}`,
		"missing expiry":       `{"active":true,"sub":"u","client_id":"login","aud":["https://grant.example.test/"],"scope":"openid grant"}`,
		"machine subject":      `{"active":true,"sub":"","client_id":"login","exp":4102444800,"aud":["https://grant.example.test/"],"scope":"openid grant"}`,
		"delegated":            `{"active":true,"sub":"u","client_id":"login","exp":4102444800,"aud":["https://grant.example.test/"],"scope":"openid grant","act":{"sub":"actor"}}`,
		"sender constrained":   `{"active":true,"sub":"u","client_id":"login","exp":4102444800,"aud":["https://grant.example.test/"],"scope":"openid grant","cnf":{"jkt":"x"}}`,
		"extra resource":       `{"active":true,"sub":"u","client_id":"login","exp":4102444800,"aud":["https://grant.example.test/","https://other.example.test/"],"scope":"openid grant"}`,
		"client plus resource": `{"active":true,"sub":"u","client_id":"login","exp":4102444800,"aud":["login","https://grant.example.test/"],"scope":"openid grant"}`,
		"missing resource":     `{"active":true,"sub":"u","client_id":"login","exp":4102444800,"aud":["login"],"scope":"openid grant"}`,
		"duplicate resource":   `{"active":true,"sub":"u","client_id":"login","exp":4102444800,"aud":["https://grant.example.test/","https://grant.example.test/"],"scope":"openid grant"}`,
		"id token type":        `{"active":true,"sub":"u","client_id":"login","exp":4102444800,"aud":["https://grant.example.test/"],"scope":"openid grant","token_type":"id_token"}`,
		"dpop token type":      `{"active":true,"sub":"u","client_id":"login","exp":4102444800,"aud":["https://grant.example.test/"],"scope":"openid grant","token_type":"DPoP"}`,
		"duplicate":            `{"active":true,"active":true,"sub":"u","client_id":"login","exp":4102444800,"aud":["https://grant.example.test"],"scope":"openid grant"}`,
		"malformed":            `{`,
		"oversized":            `{"active":true,"sub":"` + strings.Repeat("x", 65537) + `"}`,
	}
	for name, response := range tests {
		t.Run(name, func(t *testing.T) {
			server := httptest.NewTLSServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
				if r.URL.Path == "/oidc/introspect" {
					fmt.Fprint(w, response)
					return
				}
				t.Fatal("userinfo called for rejected introspection")
			}))
			defer server.Close()
			s := NewService(Config{Auth: AuthConfig{ProviderMode: "goauthy", Goauthy: &GoauthyConfig{Issuer: server.URL, IntrospectionClientID: "introspect", IntrospectionClientSecret: "secret", ExpectedClientID: "login", Resource: "https://grant.example.test/", ApplicationID: "ied", RequiredScopes: []string{"openid", "grant"}}}}, nil)
			s.client = server.Client()
			if _, err := s.externalIdentity(context.Background(), "Bearer opaque-token"); err == nil || ErrorStatus(err) != 401 && ErrorStatus(err) != 503 {
				t.Fatalf("rejected response accepted: %v", err)
			}
		})
	}
}

func TestGoauthyIdentityRejectsUserinfoUnauthorized(t *testing.T) {
	server := httptest.NewTLSServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		if r.URL.Path == "/oidc/introspect" {
			fmt.Fprint(w, `{"active":true,"sub":"one","client_id":"login","exp":4102444800,"aud":["https://grant.example.test/"],"scope":"openid grant"}`)
			return
		}
		w.WriteHeader(http.StatusUnauthorized)
	}))
	defer server.Close()
	s := NewService(Config{Auth: AuthConfig{ProviderMode: "goauthy", Goauthy: &GoauthyConfig{Issuer: server.URL, IntrospectionClientID: "introspect", IntrospectionClientSecret: "secret", ExpectedClientID: "login", Resource: "https://grant.example.test/", ApplicationID: "ied", RequiredScopes: []string{"openid", "grant"}}}}, nil)
	s.client = server.Client()
	if _, err := s.externalIdentity(context.Background(), "Bearer opaque-token"); ErrorStatus(err) != 401 {
		t.Fatalf("userinfo unauthorized accepted: %v", err)
	}
}

func TestGoauthySubject(t *testing.T) {
	if got := goauthySubject("https://gw-auth.dev.conalog.com", "subject"); got != "goauthy:https://gw-auth.dev.conalog.com/subject" {
		t.Fatalf("qualified subject=%q", got)
	}
}

func TestGoauthyIdentityRejectsUserinfoSubjectMismatch(t *testing.T) {
	server := httptest.NewTLSServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		if r.URL.Path == "/oidc/introspect" {
			fmt.Fprint(w, `{"active":true,"sub":"one","client_id":"login","exp":4102444800,"aud":["https://grant.example.test/"],"scope":"openid grant"}`)
			return
		}
		fmt.Fprint(w, `{"sub":"two"}`)
	}))
	defer server.Close()
	s := NewService(Config{Auth: AuthConfig{ProviderMode: "goauthy", Goauthy: &GoauthyConfig{Issuer: server.URL, IntrospectionClientID: "introspect", IntrospectionClientSecret: "secret", ExpectedClientID: "login", Resource: "https://grant.example.test/", ApplicationID: "ied", RequiredScopes: []string{"openid", "grant"}}}}, nil)
	s.client = server.Client()
	if _, err := s.externalIdentity(context.Background(), "Bearer opaque-token"); ErrorStatus(err) != 401 {
		t.Fatalf("userinfo mismatch accepted: %v", err)
	}
}
