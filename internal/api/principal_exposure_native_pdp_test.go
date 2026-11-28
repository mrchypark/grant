package api

import (
	"context"
	"debug/macho"
	"encoding/json"
	"net"
	"net/http"
	"os"
	"runtime"
	"strings"
	"sync/atomic"
	"testing"
	"time"

	pdppb "github.com/mrchypark/grant/api/pdp/v1"
	"github.com/mrchypark/grant/internal/grant"
	"github.com/mrchypark/rhiza"
	"google.golang.org/grpc"
	"google.golang.org/grpc/codes"
	"google.golang.org/grpc/status"
)

func TestPrincipalExposureNativePDP(t *testing.T) {
	if os.Getenv("GRANT_EXPOSURE_NATIVE_PDP_INTEROP") != "1" {
		t.Skip("set GRANT_EXPOSURE_NATIVE_PDP_INTEROP=1 with the pinned native Exposure CI artifact")
	}
	if runtime.GOOS != "darwin" || runtime.GOARCH != "arm64" {
		t.Fatal("native Exposure fixture requires darwin/arm64")
	}
	binary := os.Getenv("GRANT_EXPOSURE_BINARY")
	if binary == "" {
		t.Fatal("GRANT_EXPOSURE_BINARY is required when native interop is enabled")
	}
	binary = verifyPrincipalExposureBinary(t, binary)
	artifact, err := macho.Open(binary)
	if err != nil {
		t.Fatal(err)
	}
	cpu := artifact.Cpu
	if err := artifact.Close(); err != nil {
		t.Fatal(err)
	}
	if cpu != macho.CpuArm64 {
		t.Fatalf("Exposure artifact architecture is %v, want arm64", cpu)
	}
	ctx, cancel := context.WithTimeout(context.Background(), 80*time.Second)
	t.Cleanup(cancel)
	for _, mode := range []string{"user", "application"} {
		t.Run(mode, func(t *testing.T) { exercisePrincipalExposureNativePDP(t, ctx, binary, mode) })
	}
}

func exercisePrincipalExposureNativePDP(t *testing.T, ctx context.Context, binary, mode string) {
	t.Helper()
	const cluster = "native-pdp-interop"
	udp, err := net.ListenPacket("udp", "127.0.0.1:0")
	if err != nil {
		t.Fatal(err)
	}
	peerAddr := udp.LocalAddr().String()
	if err := udp.Close(); err != nil {
		t.Fatal(err)
	}
	member := rhiza.Member{ID: "voter", PeerURL: "quic://" + peerAddr, PublicKey: rhiza.PeerPublicKey(cluster, "voter", "voter-only")}
	f := newPrincipalExposureSystemWithDB(t, ctx, func(db *grant.DatabaseConfig) {
		db.ReadFenceKey = strings.Repeat("f", 40)
		db.RemoteAPISecret = strings.Repeat("w", 40)
		db.Rhiza = rhiza.Config{
			ClusterID: cluster, NodeID: "voter", PeerAddr: peerAddr, PeerToken: "voter-only",
			Members: []rhiza.Member{member}, AdminToken: "sync-only",
			ObjStoreProvider: "filesystem", ObjStoreDir: t.TempDir(), ObjStoreDurability: rhiza.ObjectStoreDurabilityBeforeAck,
		}
	}, func(r *http.Request, token string) {
		for name, want := range map[string]string{"Authorization": "Bearer " + token, "X-Organization-ID": "fixture-owner"} {
			if values := r.Header.Values(name); len(values) != 1 || values[0] != want {
				t.Errorf("upstream did not receive the original %s", name)
			}
		}
		for name := range r.Header {
			switch key := strings.ToLower(name); {
			case key == "x-grant-app-auth-key", key == "x-grant-master-key", strings.Contains(key, "fence"):
				t.Errorf("upstream received private header %s", name)
			}
		}
	})
	if mode == "application" {
		app := grant.Application{
			ID: f.config.Auth.Goauthy.ApplicationID, OrganizationID: "fixture-owner", Slug: "legacy", Name: "Legacy",
			ApplicationType: "confidential", AppType: "server", FirstParty: true, ConsentPolicy: "auto_approve",
			Active: true, ProvisioningStatus: "ready", CreatedByUserID: "fixture-owner",
		}
		if err := f.store.InsertApplicationWithRedirectURI(ctx, app, nil); err != nil {
			t.Fatal(err)
		}
		if _, err := f.store.Exec(ctx, "INSERT INTO application_actions(application_id,action_id) VALUES(?,'fixture-provider.item.read')", app.ID); err != nil {
			t.Fatal(err)
		}
	}
	digest := f.publish(t, mode)

	var fenceDown atomic.Bool
	var blockedFences atomic.Int64
	runtimeHandler := f.store.RuntimeHandler()
	fence := principalExposureServer(t, ctx, http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		if r.URL.Path == "/read-fence" && fenceDown.Load() {
			blockedFences.Add(1)
			w.WriteHeader(http.StatusServiceUnavailable)
			return
		}
		runtimeHandler.ServeHTTP(w, r)
	}), false)
	public, err := rhiza.NewReplicaMember(cluster, member)
	if err != nil {
		t.Fatal(err)
	}
	lc := f.config
	lc.Database.DataDir = t.TempDir()
	lc.Database.LocalNodeRole = "learner"
	lc.Database.RemoteAPISecret = ""
	lc.Database.RemoteNodes = []string{fence.URL}
	lc.Database.Replica = rhiza.ReplicaConfig{
		ClusterID: cluster, ReplicaID: "native-pdp-learner", AdminToken: "sync-only",
		Members: []rhiza.ReplicaMember{public}, SyncInterval: time.Hour,
	}
	replica, err := grant.OpenStore(ctx, lc)
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() {
		if err := replica.Close(); err != nil {
			t.Errorf("close passive learner: %v", err)
		}
	})

	var catalogs, allows, denials, unavailable atomic.Int64
	listener, err := net.Listen("tcp", "127.0.0.1:0")
	if err != nil {
		t.Fatal(err)
	}
	server := grpc.NewServer(
		grpc.MaxRecvMsgSize(PDPGRPCMaxWireBytes), grpc.MaxSendMsgSize(PDPGRPCMaxWireBytes),
		grpc.UnaryInterceptor(func(ctx context.Context, in any, info *grpc.UnaryServerInfo, next grpc.UnaryHandler) (any, error) {
			out, err := next(ctx, in)
			switch info.FullMethod {
			case pdppb.PolicyDecisionPoint_GetHttpCatalog_FullMethodName:
				if err == nil {
					catalogs.Add(1)
				}
			case pdppb.PolicyDecisionPoint_DecideHttp_FullMethodName:
				if status.Code(err) == codes.Unavailable {
					unavailable.Add(1)
				}
				if reply, ok := out.(*pdppb.DecisionResponse); err == nil && ok {
					var decision grant.HttpCapabilityDecision
					if err := json.Unmarshal(reply.GetDecisionJson(), &decision); err != nil {
						t.Errorf("decode real native PDP decision: %v", err)
					} else {
						if mode == "user" && (decision.WireVersion != 2 || decision.PEPPolicyRevision <= 0) {
							t.Error("USER receipt did not carry wire v2 and a positive policy revision")
						}
						if decision.Allowed {
							allows.Add(1)
						} else {
							denials.Add(1)
						}
					}
				}
			}
			return out, err
		}),
	)
	RegisterPDPGRPC(server, grant.NewService(lc, replica))
	serveDone := make(chan error, 1)
	go func() { serveDone <- server.Serve(listener) }()
	t.Cleanup(func() {
		server.Stop()
		_ = listener.Close()
		if err := <-serveDone; err != nil && err != grpc.ErrServerStopped {
			t.Errorf("native PDP listener: %v", err)
		}
	})

	var httpPDPCalls atomic.Int64
	trap := principalExposureServer(t, ctx, http.HandlerFunc(func(w http.ResponseWriter, _ *http.Request) {
		httpPDPCalls.Add(1)
		w.WriteHeader(http.StatusServiceUnavailable)
	}), false)
	origin := f.startExposureWithPDP(t, binary, digest, mode == "application", map[string]string{
		"GRANT_GRPC_TARGET": listener.Addr().String(), "GRANT_LEARNER_URL": trap.URL,
	})
	if catalogs.Load() == 0 {
		t.Fatal("Exposure did not load its HTTP capability catalog over native gRPC")
	}
	request := func(wantHTTP int, observed *atomic.Int64, wantUpstream int64) {
		t.Helper()
		before, upstream := observed.Load(), f.upstreamHits.Load()
		got := principalExposureRequest(t, ctx, origin+"/items/allowed", f.tokens["user"], "fixture-owner")
		if (wantHTTP == 0 && got < 400) || (wantHTTP != 0 && got != wantHTTP) ||
			observed.Load()-before != 1 || f.upstreamHits.Load()-upstream != wantUpstream || httpPDPCalls.Load() != 0 {
			t.Fatalf("HTTP=%d want=%d native decisions=%d upstream=%d want=%d HTTP PDP attempts=%d",
				got, wantHTTP, observed.Load()-before, f.upstreamHits.Load()-upstream, wantUpstream, httpPDPCalls.Load())
		}
	}
	request(http.StatusOK, &allows, 1)
	if n, err := f.store.Exec(ctx, "UPDATE role_bindings SET state='revoked' WHERE id='fixture-resource-reader' AND state='active'"); err != nil || n != 1 {
		t.Fatalf("revoke resource role: affected=%d err=%v", n, err)
	}
	request(http.StatusForbidden, &denials, 0)
	if n, err := f.store.Exec(ctx, "UPDATE role_bindings SET state='active' WHERE id='fixture-resource-reader' AND state='revoked'"); err != nil || n != 1 {
		t.Fatalf("restore resource role: affected=%d err=%v", n, err)
	}
	request(http.StatusOK, &allows, 1)
	fenceDown.Store(true)
	request(0, &unavailable, 0)
	if blockedFences.Load() == 0 {
		t.Fatal("learner never attempted the unavailable voter fence")
	}
	if mode == "user" {
		f.assertNoApplications(t)
	}
	t.Log("configured HTTP PDP trap received zero calls")
}
