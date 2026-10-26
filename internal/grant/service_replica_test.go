package grant

import (
	"context"
	"errors"
	"github.com/mrchypark/rhiza"
	"net"
	"net/http"
	"net/http/httptest"
	"net/url"
	"strings"
	"testing"
	"time"
)

func TestReplicaFenceRevocationAndReadOnly(t *testing.T) {
	ctx := context.Background()
	udp, e := net.ListenPacket("udp", "127.0.0.1:0")
	if e != nil {
		t.Fatal(e)
	}
	addr := udp.LocalAddr().String()
	udp.Close()
	member := rhiza.Member{ID: "voter", PeerURL: "quic://" + addr, PublicKey: rhiza.PeerPublicKey("fence", "voter", "voter-only")}
	cfg := Config{Database: DatabaseConfig{DataDir: t.TempDir(), ReadFenceKey: strings.Repeat("f", 40), RemoteAPISecret: strings.Repeat("w", 40), Rhiza: rhiza.Config{ClusterID: "fence", NodeID: "voter", PeerAddr: addr, PeerToken: "voter-only", Members: []rhiza.Member{member}, AdminToken: "sync-only", ObjStoreProvider: "filesystem", ObjStoreDir: t.TempDir(), ObjStoreDurability: rhiza.ObjectStoreDurabilityBeforeAck}}}
	voter, e := OpenStore(ctx, cfg)
	if e != nil {
		t.Fatal(e)
	}
	defer voter.Close()
	if e = voter.EnsureBootstrapData(ctx); e != nil {
		t.Fatal(e)
	}
	server := httptest.NewServer(voter.RuntimeHandler())
	defer server.Close()
	public, e := rhiza.NewReplicaMember("fence", member)
	if e != nil {
		t.Fatal(e)
	}
	var services []*Service
	for range 2 {
		lc := cfg
		lc.Database.DataDir = t.TempDir()
		lc.Database.LocalNodeRole = "learner"
		lc.Database.RemoteAPISecret = ""
		lc.Database.RemoteNodes = []string{server.URL}
		lc.Database.Replica = rhiza.ReplicaConfig{ClusterID: "fence", ReplicaID: requestID(), AdminToken: "sync-only", Members: []rhiza.ReplicaMember{public}, SyncInterval: time.Hour}
		ls, e := OpenStore(ctx, lc)
		if e != nil {
			t.Fatal(e)
		}
		defer ls.Close()
		services = append(services, NewService(lc, ls))
	}
	check := func(ctx context.Context, s *Service) (bool, error) {
		return localReplicaRead(ctx, s, func(c context.Context) (bool, error) {
			d, e := s.store.EvaluateUserActions(c, []UserActionCheck{{UserID: "conalog-grant", OrganizationID: "conalog-grant", ProviderID: "grant", ActionID: "organization.group.manage", ResourceType: "organization", ResourceID: "conalog-grant", Consistency: ReadStrong}})
			if e != nil {
				return false, e
			}
			return d[0].Allowed, nil
		})
	}
	for _, s := range services {
		if ok, e := check(ctx, s); e != nil || !ok {
			t.Fatalf("before revoke=%v %v", ok, e)
		}
		if _, e = s.store.Exec(ctx, "DELETE FROM users"); e == nil {
			t.Fatal("learner write accepted")
		}
		if e = s.store.EnsureBootstrapData(ctx); e == nil {
			t.Fatal("learner bootstrap accepted")
		}
	}
	now := nowString()
	if e = voter.Transaction(ctx, []Statement{
		{SQL: "INSERT INTO users(id,display_name,active,created_at) VALUES('fence-backup-owner','Backup owner',1,?)", Args: []any{now}},
		{SQL: "INSERT INTO organization_memberships(id,organization_id,user_id,membership_role,status,created_at,updated_at) VALUES('fence-backup-membership','conalog-grant','fence-backup-owner','owner','active',?,?)", Args: []any{now, now}},
		{SQL: "UPDATE organization_memberships SET membership_role='member',updated_at=? WHERE organization_id='conalog-grant' AND user_id='conalog-grant' AND membership_role='owner'", Args: []any{now}},
	}); e != nil {
		t.Fatal(e)
	}
	fence, e := voter.db.Query(ctx, rhiza.QueryRequest{SQL: "SELECT 1", Consistency: rhiza.ConsistencyLinearizable})
	if e != nil {
		t.Fatal(e)
	}
	if ok, e := check(ctx, NewService(cfg, voter)); e != nil || ok {
		t.Fatalf("voter after revoke=%v %v", ok, e)
	}
	for _, s := range services {
		if applied := s.store.replica.Status().AppliedSlot; applied >= fence.AppliedSlot {
			t.Fatalf("learner already caught up: applied=%d committed=%d", applied, fence.AppliedSlot)
		}
		// Bypass the request fence only here to expose the stale authorization.
		scope := &replicaRead{}
		staleCtx := context.WithValue(ctx, replicaReadKey{}, scope)
		if ok, err := check(staleCtx, s); err != nil || !ok {
			t.Fatalf("scoped stale read=%v %v", ok, err)
		}
		if !scope.pinned || scope.changed || scope.slot >= fence.AppliedSlot {
			t.Fatalf("stale snapshot scope=%+v committed=%d", scope, fence.AppliedSlot)
		}
		if ok, e := check(ctx, s); e != nil || ok {
			t.Fatalf("after revoke=%v %v", ok, e)
		}
	}
	req := httptest.NewRequest(http.MethodPost, "/execute", strings.NewReader(`{"sql":"DELETE FROM users"}`))
	req.Header.Set("Authorization", "Bearer "+cfg.Database.ReadFenceKey)
	response := httptest.NewRecorder()
	voter.RuntimeHandler().ServeHTTP(response, req)
	if response.Code != 401 {
		t.Fatalf("fence key admitted write: %d", response.Code)
	}
	server.Close()
	for _, s := range services {
		if _, e := check(ctx, s); ErrorStatus(e) != 503 {
			t.Fatalf("partition did not fail closed: %v", e)
		}
	}
}
func TestPocketBaseApplicationIsReadyWithoutTrust(t *testing.T) {
	for _, mode := range []string{"pocketbase", "rauthy"} {
		t.Run(mode, func(t *testing.T) {
			ctx := context.Background()
			cfg := Config{Auth: AuthConfig{ProviderMode: mode}, Database: DatabaseConfig{DataDir: t.TempDir()}}
			store, e := OpenStore(ctx, cfg)
			if e != nil {
				t.Fatal(e)
			}
			defer store.Close()
			if e = store.EnsureBootstrapData(ctx); e != nil {
				t.Fatal(e)
			}
			a, e := NewService(cfg, store).CreateApplication(ctx, "conalog-grant", "conalog-grant", CreateApplicationInput{ID: "app", Slug: "app", Name: "App", ApplicationType: "confidential", AppType: "server", Active: true})
			if e != nil {
				t.Fatal(e)
			}
			if a.Application.FirstParty || a.Application.ConsentPolicy != "explicit" {
				t.Fatal("trust broadened")
			}
			if mode == "pocketbase" {
				if a.Application.ProvisioningStatus != "ready" || a.Application.RauthyClientID != nil {
					t.Fatal(a)
				}
			} else if a.Application.ProvisioningStatus != "unprovisioned" || a.Application.RauthyClientID != nil {
				t.Fatal(a)
			}
		})
	}
}

func TestReadFenceRejectsMalformedResponses(t *testing.T) {
	for _, body := range []string{`{}`, `{"appliedSlot":null}`, `{"appliedSlot":1,"appliedSlot":0}`, `{"appliedSlot":1} {}`, `{"appliedSlot":-1}`} {
		server := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) { w.Write([]byte(body)) }))
		store := Store{cfg: Config{Database: DatabaseConfig{ReadFenceKey: strings.Repeat("f", 40), RemoteNodes: []string{server.URL}}}}
		if _, e := store.readFence(context.Background()); e == nil {
			t.Errorf("accepted %s", body)
		}
		server.Close()
	}
}

func TestReadFenceNeverUsesEnvironmentProxy(t *testing.T) {
	original := http.DefaultTransport
	defer func() { http.DefaultTransport = original }()
	called := false
	transport := original.(*http.Transport).Clone()
	transport.Proxy = func(*http.Request) (*url.URL, error) { called = true; return nil, errors.New("unexpected proxy") }
	http.DefaultTransport = transport
	server := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) { w.Write([]byte(`{"appliedSlot":3}`)) }))
	defer server.Close()
	store := Store{cfg: Config{Database: DatabaseConfig{ReadFenceKey: strings.Repeat("f", 40), RemoteNodes: []string{server.URL}}}}
	slot, e := store.readFence(context.Background())
	if e != nil || slot != 3 || called {
		t.Fatalf("slot=%d proxy=%v err=%v", slot, called, e)
	}
}
