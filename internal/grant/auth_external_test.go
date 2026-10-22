package grant

import (
	"context"
	"fmt"
	"net/http"
	"net/http/httptest"
	"strings"
	"sync/atomic"
	"testing"
	"time"
)

func TestPocketBaseOpaqueRefreshBoundary(t *testing.T) {
	var calls atomic.Int32
	payload := `{"token":"refreshed","record":{"id":"external","collectionId":"_pb_users_auth_","collectionName":"users","email":"USER@EXAMPLE.TEST","verified":true,"name":"Name","organizations":{"untrusted":"opaque"}}}`
	upstream := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		calls.Add(1)
		if r.URL.Path != "/api/collections/users/auth-refresh" || r.Method != "POST" || r.Header.Get("Authorization") != "opaque token" {
			t.Errorf("upstream mismatch %s %s", r.Method, r.URL)
		}
		w.Header().Set("Content-Type", "application/json")
		fmt.Fprint(w, payload)
	}))
	defer upstream.Close()
	s := NewService(Config{Auth: AuthConfig{ProviderMode: "pocketbase", PocketBase: &PocketBaseConfig{BaseURL: upstream.URL, ApplicationID: "app"}}}, nil)
	for i := 0; i < 2; i++ {
		identity, err := s.externalIdentity(context.Background(), "opaque token")
		if err != nil || identity.ExternalUserID != "external" || ptrValue(identity.Email) != "user@example.test" {
			t.Fatalf("identity=%+v err=%v", identity, err)
		}
	}
	if calls.Load() != 2 {
		t.Fatal("PocketBase token was cached")
	}
	if _, err := s.externalIdentity(context.Background(), "opaque token"); err != nil {
		t.Fatal("authentication depended on domain organization claims", err)
	}
	payload = `{"token":"ok","record":{"id":"a","id":"b","email":"x@y","verified":true}}`
	if _, err := s.externalIdentity(context.Background(), "opaque token"); err == nil || ErrorStatus(err) != 503 {
		t.Fatal("duplicate identity accepted", err)
	}
	payload = strings.Repeat("x", 65537)
	if _, err := s.externalIdentity(context.Background(), "opaque token"); err == nil || ErrorStatus(err) != 503 {
		t.Fatal("oversized response accepted", err)
	}
}
func TestRauthyIntrospectionAndUserinfoBinding(t *testing.T) {
	var calls atomic.Int32
	subject := "user"
	upstream := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		calls.Add(1)
		w.Header().Set("Content-Type", "application/json")
		switch r.URL.Path {
		case "/introspect":
			u, p, ok := r.BasicAuth()
			if !ok || u != "client" || p != "secret" {
				t.Error("missing introspection client auth")
			}
			_ = r.ParseForm()
			if r.Form.Get("token") != "raw" {
				t.Error("bad token")
			}
			fmt.Fprintf(w, `{"active":true,"sub":"user","client_id":"app","exp":%d}`, time.Now().Add(time.Minute).Unix())
		case "/userinfo":
			if r.Header.Get("Authorization") != "Bearer raw" {
				t.Error("bad userinfo bearer")
			}
			fmt.Fprintf(w, `{"sub":%q,"email":"user@example.test","email_verified":false}`, subject)
		}
	}))
	defer upstream.Close()
	cfg := Config{Auth: AuthConfig{ProviderMode: "rauthy", Rauthy: RauthyConfig{BaseURL: upstream.URL, IntrospectPath: "/introspect", UserinfoPath: "/userinfo", ClientID: "client", ClientSecret: "secret", CacheTTLSeconds: 10}}}
	s := NewService(cfg, nil)
	i, err := s.externalIdentity(context.Background(), "Bearer raw")
	if err != nil || i.Email != nil || i.ClientID != "app" {
		t.Fatal(i, err)
	}
	if _, err = s.externalIdentity(context.Background(), "bearer raw"); err != nil || calls.Load() != 2 {
		t.Fatal("bounded cache failed", err, calls.Load())
	}
	subject = "other"
	s = NewService(cfg, nil)
	if _, err = s.externalIdentity(context.Background(), "Bearer raw"); err == nil || ErrorStatus(err) != 401 {
		t.Fatal("mismatched userinfo accepted", err)
	}
	if _, err = s.externalIdentity(context.Background(), "raw"); err == nil {
		t.Fatal("Rauthy accepted missing bearer scheme")
	}
}
