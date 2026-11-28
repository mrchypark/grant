package api

import (
	"context"
	"encoding/json"
	"fmt"
	"net"
	"net/http"
	"net/http/httptest"
	"strings"
	"testing"
	"time"

	pdppb "github.com/mrchypark/grant/api/pdp/v1"
	"github.com/mrchypark/grant/internal/grant"
	"github.com/mrchypark/rhiza"
	"google.golang.org/grpc/codes"
	"google.golang.org/grpc/metadata"
	"google.golang.org/grpc/status"
)

func TestProviderUserAuthorizationHTTPBoundary(t *testing.T) {
	const root = "/api/v2/providers/operable/authz/"
	const check = `{"action":"operable.row.read","resource":{"provider":"operable","type":"row","id":"allowed"}}`
	for _, learner := range []bool{false, true} {
		t.Run(fmt.Sprintf("learner=%t", learner), func(t *testing.T) {
			s := testServer()
			s.learner = learner
			for _, tc := range []struct {
				name, method, path, body string
				want                     int
			}{
				{"batch reaches private auth", "POST", root + "batch-check", `{"checks":[` + check + `]}`, 401},
				{"filter reaches private auth", "POST", root + "filter", `{"action":"operable.row.read","resources":[]}`, 401},
				{"empty batch reaches private auth", "POST", root + "batch-check", `{"checks":[]}`, 401},
				{"subject override", "POST", root + "batch-check", `{"checks":[],"subject":{"id":"owner"}}`, 422},
				{"application override", "POST", root + "batch-check", `{"checks":[],"applicationId":"app"}`, 422},
				{"mode override", "POST", root + "batch-check", `{"checks":[],"authorizationMode":"application"}`, 422},
				{"policy override", "POST", root + "batch-check", `{"checks":[],"pepPolicyRevision":1}`, 422},
				{"nested override", "POST", root + "batch-check", `{"checks":[{"action":"read","subject":{},"resource":{"provider":"operable","type":"row","id":"a"}}]}`, 422},
				{"null checks", "POST", root + "batch-check", `{"checks":null}`, 422},
				{"missing checks", "POST", root + "batch-check", `{}`, 422},
				{"missing resources", "POST", root + "filter", `{"action":"read"}`, 422},
				{"blank action", "POST", root + "filter", `{"action":" ","resources":[]}`, 422},
				{"blank resource", "POST", root + "batch-check", `{"checks":[{"action":"read","resource":{"provider":"operable","type":"row","id":" "}}]}`, 422},
				{"foreign resource", "POST", root + "batch-check", `{"checks":[{"action":"read","resource":{"provider":"other","type":"row","id":"a"}}]}`, 400},
				{"duplicate field", "POST", root + "batch-check", `{"checks":[],"checks":[]}`, 422},
				{"view unsupported", "POST", root + "filter", `{"action":"read","resources":[{"provider":"operable","type":"row","id":"a","view":null}]}`, 400},
				{"invalid provider", "POST", "/api/v2/providers/bad%20provider/authz/batch-check", `{"checks":[]}`, 422},
				{"decoded slash", "POST", "/api/v2/providers/bad%2Fprovider/authz/batch-check", `{"checks":[]}`, 422},
				{"no single check", "POST", root + "check", check, 404},
			} {
				t.Run(tc.name, func(t *testing.T) {
					r := httptest.NewRequest(tc.method, tc.path, strings.NewReader(tc.body))
					r.Header.Set("Content-Type", "application/json")
					w := httptest.NewRecorder()
					s.ServeHTTP(w, r)
					if w.Code != tc.want {
						t.Fatalf("status=%d want=%d body=%s", w.Code, tc.want, w.Body.String())
					}
				})
			}
			for _, name := range []string{"Authorization", "X-Grant-App-Auth-Key", "X-Organization-ID"} {
				r := httptest.NewRequest("POST", root+"batch-check", strings.NewReader(`{"checks":[]}`))
				r.Header.Set("Content-Type", "application/json")
				r.Header.Set("Authorization", "Bearer token")
				r.Header.Set("X-Grant-App-Auth-Key", "app-secret")
				r.Header.Set("X-Organization-ID", "org")
				r.Header.Add(name, "ambiguous")
				w := httptest.NewRecorder()
				s.ServeHTTP(w, r)
				if w.Code != 400 {
					t.Fatalf("duplicate %s accepted: %d", name, w.Code)
				}
			}
			for _, n := range []int{1000, 1001} {
				payload := `{"checks":[` + strings.Repeat(check+",", n-1) + check + `]}`
				r := httptest.NewRequest("POST", root+"batch-check", strings.NewReader(payload))
				r.Header.Set("Content-Type", "application/json")
				w := httptest.NewRecorder()
				s.ServeHTTP(w, r)
				want := 401
				if n > 1000 {
					want = 400
				}
				if w.Code != want {
					t.Fatalf("n=%d status=%d want=%d", n, w.Code, want)
				}
			}
			r := httptest.NewRequest("POST", root+"batch-check", strings.NewReader(strings.Repeat(" ", 2*1024*1024+1)))
			r.Header.Set("Content-Type", "application/json")
			w := httptest.NewRecorder()
			s.ServeHTTP(w, r)
			if w.Code != 413 {
				t.Fatalf("oversized body status=%d", w.Code)
			}
		})
	}
}

func TestProviderUserAuthorizationHTTPRealReplica(t *testing.T) {
	ctx := context.Background()
	idp := httptest.NewTLSServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		switch r.URL.Path {
		case "/oidc/introspect":
			id, secret, ok := r.BasicAuth()
			if !ok || id != "introspector" || secret != "secret" || r.Method != "POST" {
				t.Error("invalid introspection authentication")
			}
			if err := r.ParseForm(); err != nil || r.Form.Get("token") != "fixture-token" {
				t.Error("invalid introspection token")
			}
			fmt.Fprint(w, `{"active":true,"sub":"fixture-user","client_id":"login","exp":4102444800,"aud":["https://grant.example.test/"],"scope":"openid grant"}`)
		case "/oidc/userinfo":
			if r.Header.Get("Authorization") != "Bearer fixture-token" {
				t.Error("invalid UserInfo credential")
			}
			fmt.Fprint(w, `{"sub":"fixture-user"}`)
		default:
			w.WriteHeader(404)
		}
	}))
	t.Cleanup(idp.Close)
	trustLocalIssuerForGrantAPI(t, idp.Certificate())
	udp, err := net.ListenPacket("udp", "127.0.0.1:0")
	if err != nil {
		t.Fatal(err)
	}
	addr := udp.LocalAddr().String()
	if err = udp.Close(); err != nil {
		t.Fatal(err)
	}
	member := rhiza.Member{ID: "voter", PeerURL: "quic://" + addr, PublicKey: rhiza.PeerPublicKey("http-user-authz", "voter", "voter-only")}
	cfg := grant.Config{
		Security: grant.SecurityConfig{AppAuthKey: "fixture-app-key", HTTPCapabilityPEPClientID: "provider-pep"},
		Auth:     grant.AuthConfig{ProviderMode: "goauthy", Goauthy: &grant.GoauthyConfig{Issuer: idp.URL, IntrospectionClientID: "introspector", IntrospectionClientSecret: "secret", ExpectedClientID: "login", Resource: "https://grant.example.test/", ApplicationID: "absent-app", RequiredScopes: []string{"openid", "grant"}}},
		Database: grant.DatabaseConfig{DataDir: t.TempDir(), ReadFenceKey: strings.Repeat("f", 40), RemoteAPISecret: strings.Repeat("w", 40), Rhiza: rhiza.Config{ClusterID: "http-user-authz", NodeID: "voter", PeerAddr: addr, PeerToken: "voter-only", Members: []rhiza.Member{member}, AdminToken: "sync-only", ObjStoreProvider: "filesystem", ObjStoreDir: t.TempDir(), ObjStoreDurability: rhiza.ObjectStoreDurabilityBeforeAck}},
	}
	voter, err := grant.OpenStore(ctx, cfg)
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() {
		if err := voter.Close(); err != nil {
			t.Error(err)
		}
	})
	if err = voter.EnsureBootstrapData(ctx); err != nil {
		t.Fatal(err)
	}
	exec := func(sql string, args ...any) {
		t.Helper()
		if _, err := voter.Exec(ctx, sql, args...); err != nil {
			t.Fatal(err)
		}
	}
	exec("INSERT INTO user_identities(id,user_id,provider,issuer,collection_id,external_user_id,last_seen_at,created_at,updated_at) VALUES('http-user-identity','conalog-grant','goauthy',?,'','fixture-user','now','now','now')", idp.URL)
	svc := grant.NewService(cfg, voter)
	if _, err = svc.UpsertServiceProviderAdmin(ctx, grant.ServiceProvider{ID: "operable", Name: "Operable", Active: true}); err != nil {
		t.Fatal(err)
	}
	if _, err = svc.CreateProviderAction(ctx, "operable", grant.Action{ID: "operable.row.read", ProviderID: "operable", Code: "row.read", Name: "Read row", RiskLevel: "low"}); err != nil {
		t.Fatal(err)
	}
	mode := "user"
	if _, err = svc.SetProviderPepClientAdmin(ctx, "operable", "provider-pep", grant.UpsertProviderPepClient{Lifecycle: "active", AuthorizationMode: &mode}); err != nil {
		t.Fatal(err)
	}
	exec("INSERT INTO roles(id,organization_id,code,name,created_at,updated_at) VALUES('provider-role','conalog-grant','provider-role','Provider role','now','now')")
	exec("INSERT INTO role_actions(role_id,action_id) VALUES('provider-role','operable.row.read')")
	exec("INSERT INTO role_bindings(id,subject_type,subject_id,subject_organization_id,provider_id,resource_type,resource_id,role_id,state,created_by_user_id,created_at,updated_at) VALUES('provider-binding','user','conalog-grant','conalog-grant','operable','row','allowed','provider-role','active','conalog-grant','now','now')")
	runtime := httptest.NewServer(voter.RuntimeHandler())
	t.Cleanup(runtime.Close)
	public, err := rhiza.NewReplicaMember("http-user-authz", member)
	if err != nil {
		t.Fatal(err)
	}
	lc := cfg
	lc.Database.DataDir = t.TempDir()
	lc.Database.LocalNodeRole = "learner"
	lc.Database.RemoteAPISecret = ""
	lc.Database.RemoteNodes = []string{runtime.URL}
	lc.Database.Replica = rhiza.ReplicaConfig{ClusterID: "http-user-authz", ReplicaID: "http-user-learner", AdminToken: "sync-only", Members: []rhiza.ReplicaMember{public}, SyncInterval: time.Hour}
	replica, err := grant.OpenStore(ctx, lc)
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() {
		if err := replica.Close(); err != nil {
			t.Error(err)
		}
	})
	learner := grant.NewService(lc, replica)
	server := httptest.NewServer(New(learner, true))
	t.Cleanup(server.Close)
	request := func(path, payload, key, token, org string, want int) map[string]json.RawMessage {
		t.Helper()
		r, err := http.NewRequest("POST", server.URL+path, strings.NewReader(payload))
		if err != nil {
			t.Fatal(err)
		}
		r.Header.Set("Content-Type", "application/json")
		if key != "" {
			r.Header.Set("X-Grant-App-Auth-Key", key)
		}
		if token != "" {
			r.Header.Set("Authorization", token)
		}
		if org != "" {
			r.Header.Set("X-Organization-ID", org)
		}
		response, err := server.Client().Do(r)
		if err != nil {
			t.Fatal(err)
		}
		defer response.Body.Close()
		var out map[string]json.RawMessage
		if err := json.NewDecoder(response.Body).Decode(&out); err != nil {
			t.Fatal(err)
		}
		if response.StatusCode != want {
			t.Fatalf("%s status=%d want=%d error=%s", path, response.StatusCode, want, out["error"])
		}
		return out
	}
	const root = "/api/v2/providers/operable/authz/"
	const allowed = `{"action":"operable.row.read","resource":{"provider":"operable","type":"row","id":"allowed"}}`
	const denied = `{"action":"operable.row.read","resource":{"provider":"operable","type":"row","id":"denied"}}`
	batch := `{"checks":[` + denied + `,` + allowed + `,` + allowed + `]}`
	pdp := newPDPTestClient(t, learner)
	pdpBatch := func(org, provider, body string, want codes.Code) grant.UserBatchCheckResult {
		t.Helper()
		call, cancel := context.WithTimeout(ctx, 5*time.Second)
		defer cancel()
		call = metadata.AppendToOutgoingContext(call, "x-grant-app-auth-key", "fixture-app-key", "authorization", "Bearer fixture-token", "x-organization-id", org)
		reply, err := pdp.BatchCheckProviderUser(call, &pdppb.ProviderUserBatchRequest{ProviderId: provider, ChecksJson: []byte(body)})
		var result grant.UserBatchCheckResult
		if status.Code(err) != want {
			t.Fatalf("native USER batch: status=%v want %s", err, want)
		}
		if want != codes.OK {
			if reply != nil {
				t.Fatalf("failed native USER batch returned a response: %v", reply)
			}
		} else if err := json.Unmarshal(reply.GetDecisionJson(), &result); err != nil {
			t.Fatalf("decode native USER batch: %v", err)
		}
		return result
	}
	out := request(root+"batch-check", batch, "fixture-app-key", "Bearer fixture-token", "conalog-grant", 200)
	var result grant.UserBatchCheckResult
	raw, _ := json.Marshal(out)
	if err := json.Unmarshal(raw, &result); err != nil {
		t.Fatal(err)
	}
	if result.PEPPolicyRevision != 1 || len(result.Results) != 3 || result.Results[0].Allowed || !result.Results[1].Allowed || !result.Results[2].Allowed {
		t.Fatalf("ordered decisions lost: %+v", result)
	}
	if got := pdpBatch("conalog-grant", "operable", batch, codes.OK); got.PEPPolicyRevision != result.PEPPolicyRevision ||
		len(got.Results) != 3 || got.Results[0] != result.Results[0] || got.Results[1] != result.Results[1] || got.Results[2] != result.Results[2] {
		t.Fatalf("native batch differs from HTTP: native=%+v HTTP=%+v", got, result)
	}
	pdpBatch("wrong-org", "operable", batch, codes.PermissionDenied)
	pdpBatch("conalog-grant", "grant", `{"checks":[]}`, codes.PermissionDenied)
	filter := `{"action":"operable.row.read","resources":[{"provider":"operable","type":"row","id":"denied"},{"provider":"operable","type":"row","id":"allowed"},{"provider":"operable","type":"row","id":"allowed"}]}`
	out = request(root+"filter", filter, "fixture-app-key", "Bearer fixture-token", "conalog-grant", 200)
	if string(out["resource_ids"]) != `["allowed","allowed"]` || string(out["pepPolicyRevision"]) != "1" {
		t.Fatalf("filter ordering/revision lost: %s %s", out["resource_ids"], out["pepPolicyRevision"])
	}
	out = request(root+"batch-check", `{"checks":[]}`, "fixture-app-key", "Bearer fixture-token", "conalog-grant", 200)
	if string(out["results"]) != "[]" || string(out["pepPolicyRevision"]) != "1" {
		t.Fatal("empty batch lost fenced policy result")
	}
	if got := pdpBatch("conalog-grant", "operable", `{"checks":[]}`, codes.OK); got.Results == nil || len(got.Results) != 0 || got.PEPPolicyRevision <= 0 {
		t.Fatalf("native empty batch lost fenced policy result: %+v", got)
	}
	out = request(root+"filter", `{"action":"operable.row.read","resources":[]}`, "fixture-app-key", "Bearer fixture-token", "conalog-grant", 200)
	if string(out["resource_ids"]) != "[]" || string(out["pepPolicyRevision"]) != "1" {
		t.Fatal("empty filter lost fenced policy result")
	}
	request(root+"batch-check", batch, "wrong-key", "Bearer fixture-token", "conalog-grant", 401)
	request(root+"batch-check", batch, "fixture-app-key", "", "conalog-grant", 401)
	request(root+"batch-check", batch, "fixture-app-key", "Bearer fixture-token", "", 400)
	request("/api/v1/application-authz/batch-check", batch, "fixture-app-key", "Bearer fixture-token", "conalog-grant", 403)
	exec("UPDATE role_bindings SET state='revoked' WHERE id='provider-binding'")
	if got := pdpBatch("conalog-grant", "operable", `{"checks":[`+allowed+`]}`, codes.OK); got.PEPPolicyRevision != 1 ||
		len(got.Results) != 1 || got.Results[0].Allowed {
		t.Fatalf("native batch retained revoked authority: %+v", got)
	}
	out = request(root+"filter", filter, "fixture-app-key", "Bearer fixture-token", "conalog-grant", 200)
	if string(out["resource_ids"]) != "[]" {
		t.Fatal("revoked binding retained")
	}
	if _, err = svc.SetProviderPepClientAdmin(ctx, "operable", "provider-pep", grant.UpsertProviderPepClient{Lifecycle: "disabled"}); err != nil {
		t.Fatal(err)
	}
	pdpBatch("conalog-grant", "operable", `{"checks":[]}`, codes.PermissionDenied)
	request(root+"batch-check", `{"checks":[]}`, "fixture-app-key", "Bearer fixture-token", "conalog-grant", 403)
	rows, err := voter.Query(ctx, grant.ReadStrong, "SELECT id FROM applications")
	if err != nil || len(rows) != 0 {
		t.Fatalf("zero-application contract changed: rows=%d err=%v", len(rows), err)
	}
	runtime.Close()
	pdpBatch("conalog-grant", "operable", batch, codes.Unavailable)
	request(root+"batch-check", batch, "fixture-app-key", "Bearer fixture-token", "conalog-grant", 503)
	request(root+"filter", `{"action":"operable.row.read","resources":[]}`, "fixture-app-key", "Bearer fixture-token", "conalog-grant", 503)
}
