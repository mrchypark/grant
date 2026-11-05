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

// These are protocol fixtures, not the issuer owner's human-user acceptance artifacts.
func principalGoauthyFixture(t *testing.T, s *Service, claims, profile string) *int {
	t.Helper()
	calls := new(int)
	server := httptest.NewTLSServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		switch r.URL.Path {
		case "/oidc/introspect":
			*calls++
			id, secret, ok := r.BasicAuth()
			if !ok || id != "introspector" || secret != "secret" || r.Method != http.MethodPost {
				t.Error("invalid introspection request")
			}
			if err := r.ParseForm(); err != nil || r.Form.Get("token") != "opaque-token" {
				t.Error("invalid introspection token")
			}
			fmt.Fprint(w, claims)
		case "/oidc/userinfo":
			if r.Header.Get("Authorization") != "Bearer opaque-token" {
				t.Error("invalid UserInfo credential")
			}
			fmt.Fprint(w, profile)
		default:
			http.NotFound(w, r)
		}
	}))
	t.Cleanup(server.Close)
	s.cfg.Auth = AuthConfig{ProviderMode: "goauthy", Goauthy: &GoauthyConfig{Issuer: server.URL, IntrospectionClientID: "introspector", IntrospectionClientSecret: "secret", ExpectedClientID: "login", Resource: "https://grant.example.test/", ApplicationID: "legacy-app", RequiredScopes: []string{"openid", "grant"}}}
	s.client = server.Client()
	return calls
}

const principalClaims = `{"active":true,"sub":"user-sub","client_id":"login","exp":4102444800,"aud":["https://grant.example.test/"],"scope":"openid grant"}`

func TestPrincipalGoauthySourceMetadata(t *testing.T) {
	s := NewService(Config{}, nil)
	principalGoauthyFixture(t, s, principalClaims, `{"sub":"user-sub"}`)
	identity, err := s.externalIdentity(context.Background(), "Bearer opaque-token")
	if err != nil {
		t.Fatal(err)
	}
	if identity.ClientID != "legacy-app" || identity.TokenClientID != "login" || identity.TokenAudience != "https://grant.example.test/" || !identity.UserInfoVerified {
		t.Fatalf("identity=%+v", identity)
	}
	for _, test := range []struct{ name, claims, profile string }{
		{"extra audience", strings.Replace(principalClaims, `["https://grant.example.test/"]`, `["https://grant.example.test/","login"]`, 1), `{"sub":"user-sub"}`},
		{"wrong client", strings.Replace(principalClaims, `"client_id":"login"`, `"client_id":"other"`, 1), `{"sub":"user-sub"}`},
		{"missing scope", strings.Replace(principalClaims, "openid grant", "openid", 1), `{"sub":"user-sub"}`},
		{"different UserInfo subject", principalClaims, `{"sub":"other"}`},
	} {
		t.Run(test.name, func(t *testing.T) {
			s := NewService(Config{}, nil)
			principalGoauthyFixture(t, s, test.claims, test.profile)
			identity, err := s.externalIdentity(context.Background(), "Bearer opaque-token")
			if ErrorStatus(err) != 401 || identity.TokenClientID != "" || identity.TokenAudience != "" || identity.UserInfoVerified {
				t.Fatalf("identity=%+v err=%v", identity, err)
			}
		})
	}
}

func principalLocalFixture(t *testing.T) (context.Context, *Service, ExternalIdentity, *int) {
	t.Helper()
	ctx, s := serviceFixture(t)
	calls := principalGoauthyFixture(t, s, principalClaims, `{"sub":"user-sub"}`)
	identity, err := s.externalIdentity(ctx, "Bearer opaque-token")
	if err != nil {
		t.Fatal(err)
	}
	now := nowString()
	lifecycleExec(t, ctx, s, "INSERT INTO user_identities(id,user_id,provider,issuer,collection_id,external_user_id,last_seen_at,created_at,updated_at) VALUES('principal-id','owner','goauthy',?,'','user-sub',?,?,?)", identity.Issuer, now, now, now)
	return ctx, s, identity, calls
}

func TestPrincipalLocalUserContextWithoutApplications(t *testing.T) {
	for _, test := range []struct {
		name, mutation, org string
		status              int
	}{
		{"active", "", "owner", 0},
		{"missing organization", "", "", 400},
		{"wrong organization", "", "other", 403},
		{"inactive user", "UPDATE users SET active=0 WHERE id='owner'", "owner", 403},
		{"inactive organization", "UPDATE organizations SET inactivated_at='now' WHERE id='owner'", "owner", 403},
		{"disabled membership", "UPDATE organization_memberships SET status='disabled' WHERE user_id='owner'", "owner", 403},
		{"missing membership", "DELETE FROM organization_memberships WHERE user_id='owner'", "owner", 403},
		{"unlinked identity", "DELETE FROM user_identities WHERE id='principal-id'", "owner", 403},
	} {
		t.Run(test.name, func(t *testing.T) {
			ctx, s, identity, _ := principalLocalFixture(t)
			if test.mutation != "" {
				lifecycleExec(t, ctx, s, test.mutation)
			}
			before := lifecycleCount(t, ctx, s, "SELECT id FROM users")
			applicationQueries := 0
			s.store.afterReplicaQuery = func(_ context.Context, sql string) {
				if strings.Contains(strings.ToLower(sql), "applications") {
					applicationQueries++
				}
			}
			result, err := capabilityAuthorizationRead(ctx, s, func(ctx context.Context) (applicationAuthorizationContext, error) {
				return s.resolveLocalUserAuthorizationContext(ctx, identity, test.org)
			})
			s.store.afterReplicaQuery = nil
			if test.status == 0 {
				if err != nil || result.UserID != "owner" || result.OrganizationID != "owner" || result.ApplicationID != "" || result.ProviderAudience != identity.TokenAudience || result.ExternalSubject != goauthySubject(identity.Issuer, identity.ExternalUserID) {
					t.Fatalf("context=%+v err=%v", result, err)
				}
			} else if ErrorStatus(err) != test.status {
				t.Fatalf("err=%v want=%d", err, test.status)
			}
			if applicationQueries != 0 || lifecycleCount(t, ctx, s, "SELECT id FROM applications") != 0 || lifecycleCount(t, ctx, s, "SELECT id FROM users") != before {
				t.Fatal("application lookup or automatic user materialization")
			}
		})
	}
}

func TestCapabilityAuthorizationReadRetriesMutationWithoutRevalidation(t *testing.T) {
	for _, mutateAlways := range []bool{false, true} {
		t.Run(fmt.Sprint(mutateAlways), func(t *testing.T) {
			ctx, s, identity, calls := principalLocalFixture(t)
			ctx = context.WithValue(ctx, replicaIdentityKey{}, identity)
			queries, attempts := 0, 0
			s.store.afterReplicaQuery = func(_ context.Context, _ string) {
				queries++
				if mutateAlways || queries == 1 {
					lifecycleExec(t, context.Background(), s, "UPDATE users SET display_name=? WHERE id='owner'", fmt.Sprint(queries))
				}
			}
			defer func() { s.store.afterReplicaQuery = nil }()
			_, err := capabilityAuthorizationRead(ctx, s, func(ctx context.Context) (applicationAuthorizationContext, error) {
				attempts++
				if deadline, ok := ctx.Deadline(); !ok || time.Until(deadline) > 3*time.Second {
					t.Fatal("missing local budget")
				}
				got, err := s.resolveExternalIdentity(ctx, "unused", ReadStrong)
				if err != nil || got.TokenClientID != identity.TokenClientID || got.TokenAudience != identity.TokenAudience || got.UserInfoVerified != identity.UserInfoVerified {
					t.Fatalf("lost identity=%+v err=%v", got, err)
				}
				return s.resolveLocalUserAuthorizationContext(ctx, got, "owner")
			})
			if mutateAlways {
				if ErrorStatus(err) != 503 || attempts != 3 {
					t.Fatalf("attempts=%d err=%v", attempts, err)
				}
			} else if err != nil || attempts != 2 {
				t.Fatalf("attempts=%d err=%v", attempts, err)
			}
			if *calls != 1 {
				t.Fatalf("external validation calls=%d", *calls)
			}
		})
	}
}

func TestPrincipalLocalUserContextRejectsUnverifiedEvidence(t *testing.T) {
	for _, change := range []struct {
		name  string
		apply func(*ExternalIdentity)
	}{
		{"other provider", func(i *ExternalIdentity) { i.Provider = "rauthy" }},
		{"inactive external identity", func(i *ExternalIdentity) { i.Active = false }},
		{"no UserInfo proof", func(i *ExternalIdentity) { i.UserInfoVerified = false }},
		{"wrong issuer", func(i *ExternalIdentity) { i.Issuer = "https://other.example.test" }},
		{"no client", func(i *ExternalIdentity) { i.TokenClientID = "" }},
		{"no audience", func(i *ExternalIdentity) { i.TokenAudience = "" }},
	} {
		t.Run(change.name, func(t *testing.T) {
			s := NewService(Config{Auth: AuthConfig{ProviderMode: "goauthy", Goauthy: &GoauthyConfig{Issuer: testGoauthyIssuer}}}, nil)
			identity := ExternalIdentity{Provider: "goauthy", Issuer: testGoauthyIssuer, ExternalUserID: "user-sub", Active: true, UserInfoVerified: true, TokenClientID: "login", TokenAudience: "https://grant.example.test/"}
			change.apply(&identity)
			if _, err := s.resolveLocalUserAuthorizationContext(context.Background(), identity, "org"); ErrorStatus(err) != 403 {
				t.Fatalf("err=%v", err)
			}
		})
	}
}

func TestCapabilityAuthorizationReadFenceFailureDoesNotRead(t *testing.T) {
	for _, role := range []string{"voter", "learner"} {
		t.Run(role, func(t *testing.T) {
			cfg := Config{Database: DatabaseConfig{LocalNodeRole: role}}
			s := NewService(cfg, &Store{cfg: cfg})
			called := false
			result, err := capabilityAuthorizationRead(context.Background(), s, func(context.Context) (bool, error) { called = true; return true, nil })
			if ErrorStatus(err) != 503 || called || result {
				t.Fatalf("result=%v called=%v err=%v", result, called, err)
			}
		})
	}
}
