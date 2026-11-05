package grant

import (
	"context"
	"fmt"
	"net"
	"net/http"
	"net/http/httptest"
	"strings"
	"sync/atomic"
	"testing"
	"time"

	"github.com/mrchypark/rhiza"
)

// Real one-voter/passive-replica transport, following the existing application
// fence fixture. No Local mode, HTTP capability catalog, or application rows.
type userAuthzFixture struct {
	voter, learner           *Service
	server                   *httptest.Server
	introspections, profiles atomic.Int64
	fences                   atomic.Int64
	issuerState              atomic.Int32
}

func newUserAuthzFixture(t *testing.T) *userAuthzFixture {
	t.Helper()
	f := &userAuthzFixture{}
	issuer := httptest.NewTLSServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		switch r.URL.Path {
		case "/oidc/introspect":
			f.introspections.Add(1)
			id, secret, ok := r.BasicAuth()
			if err := r.ParseForm(); err != nil || r.Method != http.MethodPost || !ok || id != "introspector" || secret != "secret" || r.Form.Get("token") != "opaque-token" {
				t.Error("invalid original introspection credential")
			}
			if f.issuerState.Load() == 1 {
				fmt.Fprint(w, `{"active":false}`)
				return
			}
			claims := principalClaims
			switch f.issuerState.Load() {
			case 3:
				claims = strings.Replace(claims, `"client_id":"login"`, `"client_id":"other"`, 1)
			case 4:
				claims = strings.Replace(claims, "https://grant.example.test/", "https://other.example.test/", 1)
			case 5:
				claims = strings.Replace(claims, "openid grant", "openid", 1)
			}
			fmt.Fprint(w, claims)
		case "/oidc/userinfo":
			f.profiles.Add(1)
			if r.Header.Get("Authorization") != "Bearer opaque-token" {
				t.Error("invalid original UserInfo credential")
			}
			if f.issuerState.Load() == 2 {
				fmt.Fprint(w, `{"sub":"other-user"}`)
				return
			}
			fmt.Fprint(w, `{"sub":"user-sub"}`)
		default:
			http.NotFound(w, r)
		}
	}))
	t.Cleanup(issuer.Close)
	udp, err := net.ListenPacket("udp", "127.0.0.1:0")
	if err != nil {
		t.Fatal(err)
	}
	addr := udp.LocalAddr().String()
	if err := udp.Close(); err != nil {
		t.Fatal(err)
	}
	const cluster = "provider-user-authz"
	member := rhiza.Member{ID: "voter", PeerURL: "quic://" + addr, PublicKey: rhiza.PeerPublicKey(cluster, "voter", "voter-only")}
	cfg := Config{
		Security: SecurityConfig{HTTPCapabilityPEPClientID: "user-pep"},
		Auth:     AuthConfig{ProviderMode: "goauthy", Goauthy: &GoauthyConfig{Issuer: issuer.URL, IntrospectionClientID: "introspector", IntrospectionClientSecret: "secret", ExpectedClientID: "login", Resource: "https://grant.example.test/", RequiredScopes: []string{"openid", "grant"}}},
		Database: DatabaseConfig{DataDir: t.TempDir(), ReadFenceKey: strings.Repeat("f", 40), RemoteAPISecret: strings.Repeat("w", 40), Rhiza: rhiza.Config{ClusterID: cluster, NodeID: "voter", PeerAddr: addr, PeerToken: "voter-only", Members: []rhiza.Member{member}, AdminToken: "sync-only", ObjStoreProvider: "filesystem", ObjStoreDir: t.TempDir(), ObjStoreDurability: rhiza.ObjectStoreDurabilityBeforeAck}},
	}
	ctx := context.Background()
	voter, err := OpenStore(ctx, cfg)
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() {
		if err := voter.Close(); err != nil {
			t.Error(err)
		}
	})
	if err := voter.EnsureBootstrapData(ctx); err != nil {
		t.Fatal(err)
	}
	f.voter = NewService(cfg, voter)
	f.voter.client = issuer.Client()
	for _, sql := range []string{
		"INSERT INTO users(id,display_name,active,created_at) VALUES('user','User',1,'now')",
		"INSERT INTO organization_memberships(id,organization_id,user_id,membership_role,status,created_at,updated_at) VALUES('user-member','conalog-grant','user','member','active','now','now')",
		"INSERT INTO service_providers(id,name,active,created_at,updated_at) VALUES('compos','Compos',1,'now','now'),('other','Other',1,'now','now')",
		"INSERT INTO actions(id,provider_id,code,name,risk_level,visibility_granting,grantable_to_applications,created_at,updated_at) VALUES('compos.read','compos','read','Read','low',0,0,'now','now'),('other.read','other','read','Read','low',0,0,'now','now')",
		"INSERT INTO roles(id,organization_id,code,name,created_at,updated_at) VALUES('reader','conalog-grant','reader','Reader','now','now')",
		"INSERT INTO role_actions(role_id,action_id) VALUES('reader','compos.read')",
		"INSERT INTO role_bindings(id,subject_type,subject_id,subject_organization_id,provider_id,resource_type,resource_id,role_id,state,created_by_user_id,created_at,updated_at) VALUES('read-collection','user','user','conalog-grant','compos','object_type','collection-1','reader','active','conalog-grant','now','now')",
	} {
		f.exec(t, sql)
	}
	if _, err := voter.Exec(ctx, "INSERT INTO user_identities(id,user_id,provider,issuer,collection_id,external_user_id,last_seen_at,created_at,updated_at) VALUES('user-identity','user','goauthy',?,'','user-sub','now','now','now')", issuer.URL); err != nil {
		t.Fatal(err)
	}
	mode := "user"
	if _, err := f.voter.SetProviderPepClientAdmin(ctx, "compos", "user-pep", UpsertProviderPepClient{Lifecycle: "active", AuthorizationMode: &mode}); err != nil {
		t.Fatal(err)
	}
	f.server = httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		if r.URL.Path == "/read-fence" {
			f.fences.Add(1)
		}
		voter.RuntimeHandler().ServeHTTP(w, r)
	}))
	t.Cleanup(f.server.Close)
	public, err := rhiza.NewReplicaMember(cluster, member)
	if err != nil {
		t.Fatal(err)
	}
	lc := cfg
	lc.Database.DataDir = t.TempDir()
	lc.Database.LocalNodeRole = "learner"
	lc.Database.RemoteAPISecret = ""
	lc.Database.RemoteNodes = []string{f.server.URL}
	lc.Database.Replica = rhiza.ReplicaConfig{ClusterID: cluster, ReplicaID: requestID(), AdminToken: "sync-only", Members: []rhiza.ReplicaMember{public}, SyncInterval: time.Hour}
	replica, err := OpenStore(ctx, lc)
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() {
		if err := replica.Close(); err != nil {
			t.Error(err)
		}
	})
	f.learner = NewService(lc, replica)
	f.learner.client = issuer.Client()
	return f
}

func (f *userAuthzFixture) exec(t *testing.T, sql string) {
	t.Helper()
	if _, err := f.voter.store.Exec(context.Background(), sql); err != nil {
		t.Fatal(err)
	}
}

func userAuthzChecks() []ApplicationAuthorizationCheck {
	return []ApplicationAuthorizationCheck{
		{Action: "compos.read", Resource: AuthorizationResource{ProviderID: "compos", ResourceType: "object_type", ID: "collection-1"}},
		{Action: "compos.read", Resource: AuthorizationResource{ProviderID: "compos", ResourceType: "object_type", ID: "collection-2"}},
		{Action: "compos.read", Resource: AuthorizationResource{ProviderID: "compos", ResourceType: "object", ID: "collection-1"}},
	}
}

func TestUserAuthorizationRealVoterAndReplica(t *testing.T) {
	f := newUserAuthzFixture(t)
	ctx := context.Background()
	for _, s := range []*Service{f.voter, f.learner} {
		empty, err := s.UserBatchCheck(ctx, "Bearer opaque-token", "conalog-grant", "compos", []ApplicationAuthorizationCheck{})
		if err != nil || empty.Results == nil || len(empty.Results) != 0 || empty.PEPPolicyRevision <= 0 {
			t.Fatalf("valid empty batch: %+v %v", empty, err)
		}
		emptyFilter, err := s.UserFilter(ctx, "Bearer opaque-token", "conalog-grant", "compos", "compos.read", []AuthorizationResource{})
		if err != nil || emptyFilter.ResourceIDs == nil || len(emptyFilter.ResourceIDs) != 0 || emptyFilter.PEPPolicyRevision <= 0 {
			t.Fatalf("valid empty filter: %+v %v", emptyFilter, err)
		}
		before, profiles := f.introspections.Load(), f.profiles.Load()
		out, err := s.UserBatchCheck(ctx, "Bearer opaque-token", "conalog-grant", "compos", userAuthzChecks())
		if err != nil || out.PEPPolicyRevision <= 0 || len(out.Results) != 3 || !out.Results[0].Allowed || out.Results[1].Allowed || out.Results[2].Allowed {
			t.Fatalf("ordered exact-resource decisions: %+v %v", out, err)
		}
		if f.introspections.Load()-before != 1 || f.profiles.Load()-profiles != 1 {
			t.Fatal("identity must be verified exactly once")
		}
		resources := []AuthorizationResource{userAuthzChecks()[1].Resource, userAuthzChecks()[0].Resource, userAuthzChecks()[0].Resource}
		filtered, err := s.UserFilter(ctx, "Bearer opaque-token", "conalog-grant", "compos", "compos.read", resources)
		if err != nil || filtered.PEPPolicyRevision != out.PEPPolicyRevision || strings.Join(filtered.ResourceIDs, ",") != "collection-1,collection-1" {
			t.Fatalf("filter=%+v err=%v", filtered, err)
		}
		for _, table := range []string{"applications", "provider_http_capability_sets"} {
			rows, err := f.voter.store.Query(ctx, ReadStrong, "SELECT id FROM applications")
			if table != "applications" {
				rows, err = f.voter.store.Query(ctx, ReadStrong, "SELECT provider_id FROM provider_http_capability_sets")
			}
			if err != nil || len(rows) != 0 {
				t.Fatalf("unexpected %s rows=%v err=%v", table, rows, err)
			}
		}
	}
	if f.fences.Load() != 4 {
		t.Fatalf("fresh learner fences=%d want=4", f.fences.Load())
	}
	// The passive replica must catch up a new revocation on the next request.
	f.exec(t, "UPDATE role_bindings SET state='disabled' WHERE id='read-collection'")
	for _, s := range []*Service{f.voter, f.learner} {
		out, err := s.UserBatchCheck(ctx, "Bearer opaque-token", "conalog-grant", "compos", userAuthzChecks())
		if err != nil || len(out.Results) != 3 || out.Results[0].Allowed {
			t.Fatalf("revocation=%+v err=%v", out, err)
		}
		filtered, err := s.UserFilter(ctx, "Bearer opaque-token", "conalog-grant", "compos", "compos.read", []AuthorizationResource{userAuthzChecks()[0].Resource})
		if err != nil || len(filtered.ResourceIDs) != 0 {
			t.Fatalf("revoked filter=%+v err=%v", filtered, err)
		}
	}
	f.server.Close()
	for _, filter := range []bool{false, true} {
		var err error
		if filter {
			_, err = f.learner.UserFilter(ctx, "Bearer opaque-token", "conalog-grant", "compos", "compos.read", nil)
		} else {
			_, err = f.learner.UserBatchCheck(ctx, "Bearer opaque-token", "conalog-grant", "compos", nil)
		}
		if err == nil || ErrorStatus(err) != 503 {
			t.Fatalf("disconnected replica must fail closed: %v", err)
		}
	}
}

func TestUserAuthorizationScopeAndPolicy(t *testing.T) {
	f := newUserAuthzFixture(t)
	ctx := context.Background()
	for _, s := range []*Service{f.voter, f.learner} {
		for _, test := range []struct {
			name, mutate, restore string
			status                int
		}{
			{"application policy", "UPDATE provider_pep_clients SET authorization_mode='application',policy_revision=policy_revision+1", "UPDATE provider_pep_clients SET authorization_mode='user',policy_revision=policy_revision+1", 403},
			{"inactive policy", "UPDATE provider_pep_clients SET lifecycle='disabled'", "UPDATE provider_pep_clients SET lifecycle='active'", 403},
			{"expired policy", "UPDATE provider_pep_clients SET valid_until='2000-01-01T00:00:00Z'", "UPDATE provider_pep_clients SET valid_until=NULL", 403},
			{"inactive user", "UPDATE users SET active=0 WHERE id='user'", "UPDATE users SET active=1 WHERE id='user'", 403},
			{"inactive membership", "UPDATE organization_memberships SET status='disabled' WHERE id='user-member'", "UPDATE organization_memberships SET status='active' WHERE id='user-member'", 403},
			{"inactive org", "UPDATE organizations SET inactivated_at='now' WHERE id='conalog-grant'", "UPDATE organizations SET inactivated_at=NULL WHERE id='conalog-grant'", 403},
		} {
			t.Run(test.name+"/"+s.cfg.Database.LocalNodeRole, func(t *testing.T) {
				f.exec(t, test.mutate)
				defer f.exec(t, test.restore)
				_, err := s.UserBatchCheck(ctx, "Bearer opaque-token", "conalog-grant", "compos", nil)
				if err == nil || ErrorStatus(err) != test.status {
					t.Fatalf("empty batch escaped scope: %v", err)
				}
				_, err = s.UserFilter(ctx, "Bearer opaque-token", "conalog-grant", "compos", "compos.read", nil)
				if err == nil || ErrorStatus(err) != test.status {
					t.Fatalf("empty filter escaped scope: %v", err)
				}
			})
		}
		for _, state := range []int32{1, 2, 3, 4, 5} {
			f.issuerState.Store(state)
			_, err := s.UserFilter(ctx, "Bearer opaque-token", "conalog-grant", "compos", "compos.read", nil)
			f.issuerState.Store(0)
			if err == nil || ErrorStatus(err) != 401 {
				t.Fatalf("unverified empty filter: %v", err)
			}
		}
		checks := userAuthzChecks()
		checks[0].Resource.ProviderID = "other"
		if _, err := s.UserBatchCheck(ctx, "Bearer opaque-token", "conalog-grant", "compos", checks); err == nil || ErrorStatus(err) != 400 {
			t.Fatalf("cross-provider resource: %v", err)
		}
		if _, err := s.UserFilter(ctx, "Bearer opaque-token", "conalog-grant", "compos", "compos.read", []AuthorizationResource{checks[0].Resource}); err == nil || ErrorStatus(err) != 400 {
			t.Fatalf("cross-provider filter candidate: %v", err)
		}
		checks = userAuthzChecks()
		checks[0].Action = "other.read"
		if _, err := s.UserBatchCheck(ctx, "Bearer opaque-token", "conalog-grant", "compos", checks); err == nil || ErrorStatus(err) != 400 {
			t.Fatalf("cross-provider action: %v", err)
		}
		if _, err := s.UserFilter(ctx, "Bearer opaque-token", "conalog-grant", "compos", "other.read", nil); err == nil || ErrorStatus(err) != 400 {
			t.Fatalf("empty cross-provider filter: %v", err)
		}
		if _, err := s.UserFilter(ctx, "Bearer opaque-token", "conalog-grant", "compos", "compos.read", []AuthorizationResource{checks[0].Resource, checks[2].Resource}); err == nil || ErrorStatus(err) != 400 {
			t.Fatalf("mixed namespace filter: %v", err)
		}
	}
}

func TestUserAuthorizationSnapshotRetriesAndCutoff(t *testing.T) {
	f := newUserAuthzFixture(t)
	ctx := context.Background()
	// Remote SQL access is configured, but is never a substitute for a local
	// voter or a passive replica with a fresh fence.
	remoteCfg := f.voter.cfg
	remoteCfg.Database.RemoteNodes = []string{f.server.URL}
	remote := NewService(remoteCfg, &Store{cfg: remoteCfg})
	remote.client = f.voter.client
	if out, err := remote.UserBatchCheck(ctx, "Bearer opaque-token", "conalog-grant", "compos", userAuthzChecks()); err == nil || ErrorStatus(err) != 503 || out.Results != nil {
		t.Fatalf("remote SQL fallback: %+v %v", out, err)
	}
	for _, s := range []*Service{f.voter, f.learner} {
		for _, test := range []struct {
			name, sql, restore string
			status             int
			allowed            bool
		}{
			{"revision", "UPDATE provider_pep_clients SET policy_revision=policy_revision+1", "", 0, true},
			{"mode", "UPDATE provider_pep_clients SET authorization_mode='application',policy_revision=policy_revision+1", "UPDATE provider_pep_clients SET authorization_mode='user',policy_revision=policy_revision+1", 403, false},
			{"rbac", "UPDATE role_bindings SET state='disabled' WHERE id='read-collection'", "UPDATE role_bindings SET state='active' WHERE id='read-collection'", 0, false},
		} {
			t.Run(test.name+"/"+s.cfg.Database.LocalNodeRole, func(t *testing.T) {
				before, profiles := f.introspections.Load(), f.profiles.Load()
				reads := 0
				s.store.afterReplicaQuery = func(c context.Context, sql string) {
					deadline, ok := c.Deadline()
					if !ok || time.Until(deadline) > 3*time.Second || readScope(c) == nil {
						t.Error("missing bounded read proof")
					}
					if !strings.Contains(sql, "provider_pep_clients") {
						return
					}
					reads++
					if reads != 1 {
						return
					}
					f.exec(t, test.sql)
					if s.store.replica != nil {
						if err := s.store.replica.Sync(ctx); err != nil {
							t.Fatal(err)
						}
					}
				}
				defer func() {
					s.store.afterReplicaQuery = nil
					if test.restore != "" {
						f.exec(t, test.restore)
					}
				}()
				out, err := s.UserBatchCheck(ctx, "Bearer opaque-token", "conalog-grant", "compos", userAuthzChecks())
				if test.status != 0 {
					if err == nil || ErrorStatus(err) != test.status || out.Results != nil {
						t.Fatalf("mode race: %+v %v", out, err)
					}
				} else if err != nil || len(out.Results) != 3 || out.Results[0].Allowed != test.allowed {
					t.Fatalf("snapshot retry: %+v %v", out, err)
				}
				if reads < 2 || f.introspections.Load()-before != 1 || f.profiles.Load()-profiles != 1 {
					t.Fatalf("reads=%d introspections=%d profiles=%d", reads, f.introspections.Load()-before, f.profiles.Load()-profiles)
				}
				if err == nil {
					current, err := f.voter.GetProviderPepClientAdmin(ctx, "compos", "user-pep")
					if err != nil || out.PEPPolicyRevision != current.PolicyRevision {
						t.Fatalf("stale revision: %+v %+v %v", out, current, err)
					}
				}
			})
		}
		t.Run("unstable/"+s.cfg.Database.LocalNodeRole, func(t *testing.T) {
			before := f.introspections.Load()
			s.store.afterReplicaQuery = func(_ context.Context, _ string) {
				f.exec(t, "UPDATE provider_pep_clients SET policy_revision=policy_revision+1")
				if s.store.replica != nil {
					if err := s.store.replica.Sync(ctx); err != nil {
						t.Fatal(err)
					}
				}
			}
			defer func() { s.store.afterReplicaQuery = nil }()
			out, err := s.UserFilter(ctx, "Bearer opaque-token", "conalog-grant", "compos", "compos.read", nil)
			if err == nil || ErrorStatus(err) != 503 || out.ResourceIDs != nil || f.introspections.Load()-before != 1 {
				t.Fatalf("unstable snapshot: %+v %v", out, err)
			}
		})
		t.Run("cutoff/"+s.cfg.Database.LocalNodeRole, func(t *testing.T) {
			requestCtx, cancel := context.WithCancel(ctx)
			defer cancel()
			proof := false
			s.store.afterReplicaQuery = func(c context.Context, sql string) {
				if sql == "SELECT 1" {
					scope := readScope(c)
					proof = scope != nil && scope.pinned && !scope.changed && scope.slot >= scope.fence
					cancel()
				}
			}
			defer func() { s.store.afterReplicaQuery = nil }()
			out, err := s.UserBatchCheck(requestCtx, "Bearer opaque-token", "conalog-grant", "compos", nil)
			if !proof || err == nil || ErrorStatus(err) != 503 || out.Results != nil {
				t.Fatalf("expired read proof: proof=%v out=%+v err=%v", proof, out, err)
			}
		})
	}
}

func TestUserAuthorizationBoundsAndNoFallback(t *testing.T) {
	s := NewService(Config{}, &Store{})
	ctx := context.Background()
	if _, err := s.UserBatchCheck(ctx, "Bearer opaque-token", "org", "compos", make([]ApplicationAuthorizationCheck, 1001)); err == nil || ErrorStatus(err) != 400 {
		t.Fatalf("batch bound: %v", err)
	}
	if _, err := s.UserFilter(ctx, "Bearer opaque-token", "org", "compos", "compos.read", make([]AuthorizationResource, 1001)); err == nil || ErrorStatus(err) != 400 {
		t.Fatalf("filter bound: %v", err)
	}
	for _, mode := range []string{"rauthy", "pocketbase", ""} {
		s.cfg.Auth.ProviderMode = mode
		if _, err := s.UserBatchCheck(ctx, "Bearer opaque-token", "org", "compos", nil); err == nil || ErrorStatus(err) != 403 {
			t.Fatalf("unsupported issuer %s: %v", mode, err)
		}
	}
}
