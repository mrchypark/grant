package api

import (
	"context"
	"encoding/json"
	"net"
	"net/http"
	"net/http/httptest"
	"strings"
	"testing"
	"time"

	"github.com/mrchypark/grant/internal/grant"
	"github.com/mrchypark/rhiza"
)

func TestLearnerApplicationAuthorizationHTTPRealReplica(t *testing.T) {
	ctx := context.Background()
	idp := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		if r.Method != "POST" || r.URL.Path != "/api/collections/users/auth-refresh" || r.Header.Get("Authorization") != "Bearer fixture-token" {
			w.WriteHeader(401)
			return
		}
		w.Header().Set("Content-Type", "application/json")
		_, _ = w.Write([]byte(`{"token":"refreshed","record":{"id":"fixtureuser0001","collectionId":"users","collectionName":"users","name":"Fixture"}}`))
	}))
	defer idp.Close()
	udp, err := net.ListenPacket("udp", "127.0.0.1:0")
	if err != nil {
		t.Fatal(err)
	}
	addr := udp.LocalAddr().String()
	udp.Close()
	member := rhiza.Member{ID: "voter", PeerURL: "quic://" + addr, PublicKey: rhiza.PeerPublicKey("http-application", "voter", "voter-only")}
	cfg := grant.Config{Security: grant.SecurityConfig{AppAuthKey: "fixture-app-key"}, Auth: grant.AuthConfig{ProviderMode: "pocketbase", PocketBase: &grant.PocketBaseConfig{BaseURL: idp.URL, Collection: "users", ApplicationID: "fixture-app"}}, Database: grant.DatabaseConfig{DataDir: t.TempDir(), ReadFenceKey: strings.Repeat("f", 40), RemoteAPISecret: strings.Repeat("w", 40), Rhiza: rhiza.Config{ClusterID: "http-application", NodeID: "voter", PeerAddr: addr, PeerToken: "voter-only", Members: []rhiza.Member{member}, AdminToken: "sync-only", ObjStoreProvider: "filesystem", ObjStoreDir: t.TempDir(), ObjStoreDurability: rhiza.ObjectStoreDurabilityBeforeAck}}}
	voter, err := grant.OpenStore(ctx, cfg)
	if err != nil {
		t.Fatal(err)
	}
	defer voter.Close()
	if err = voter.EnsureBootstrapData(ctx); err != nil {
		t.Fatal(err)
	}
	exec := func(sql string, args ...any) {
		t.Helper()
		if _, err := voter.Exec(ctx, sql, args...); err != nil {
			t.Fatal(err)
		}
	}
	exec("INSERT INTO user_identities(id,user_id,provider,issuer,collection_id,external_user_id,last_seen_at,created_at,updated_at) VALUES('http-identity','conalog-grant','pocketbase',?,'users','fixtureuser0001','now','now','now')", idp.URL)
	exec("INSERT INTO applications(id,organization_id,slug,name,application_type,app_type,first_party,consent_policy,active,provisioning_status,created_by_user_id,created_at,updated_at) VALUES('fixture-app','conalog-grant','fixture-app','Fixture','confidential','server',1,'auto_approve',1,'ready','conalog-grant','now','now')")
	exec("INSERT INTO application_actions(application_id,action_id) VALUES('fixture-app','organization.group.manage')")
	runtime := httptest.NewServer(voter.RuntimeHandler())
	defer runtime.Close()
	public, err := rhiza.NewReplicaMember("http-application", member)
	if err != nil {
		t.Fatal(err)
	}
	lc := cfg
	lc.Database.DataDir = t.TempDir()
	lc.Database.LocalNodeRole = "learner"
	lc.Database.RemoteAPISecret = ""
	lc.Database.RemoteNodes = []string{runtime.URL}
	lc.Database.Replica = rhiza.ReplicaConfig{ClusterID: "http-application", ReplicaID: "http-learner", AdminToken: "sync-only", Members: []rhiza.ReplicaMember{public}, SyncInterval: time.Hour}
	replica, err := grant.OpenStore(ctx, lc)
	if err != nil {
		t.Fatal(err)
	}
	defer replica.Close()
	server := httptest.NewServer(New(grant.NewService(lc, replica), true))
	defer server.Close()
	check := `{"action":"organization.group.manage","resource":{"provider":"grant","type":"organization","id":"conalog-grant"}}`
	filter := `{"action":"organization.group.manage","resources":[{"provider":"grant","type":"organization","id":"conalog-grant"}]}`
	request := func(path, body, key string, want int) map[string]any {
		t.Helper()
		req, err := http.NewRequest("POST", server.URL+path, strings.NewReader(body))
		if err != nil {
			t.Fatal(err)
		}
		req.Header.Set("Content-Type", "application/json")
		req.Header.Set("Authorization", "Bearer fixture-token")
		req.Header.Set("X-Grant-App-Auth-Key", key)
		req.Header.Set("X-Organization-ID", "conalog-grant")
		res, err := server.Client().Do(req)
		if err != nil {
			t.Fatal(err)
		}
		defer res.Body.Close()
		var value map[string]any
		if err = json.NewDecoder(res.Body).Decode(&value); err != nil {
			t.Fatal(err)
		}
		if res.StatusCode != want {
			t.Fatalf("%s status=%d body=%v", path, res.StatusCode, value)
		}
		return value
	}
	// The first successful HTTP request must catch up without manual replica sync.
	out := request("/api/v1/application-authz/check", check, "fixture-app-key", 200)
	if out["allowed"] != true {
		t.Fatalf("check=%v", out)
	}
	out = request("/api/v1/application-authz/batch-check", `{"checks":[`+check+`,`+check+`]}`, "fixture-app-key", 200)
	if results, ok := out["results"].([]any); !ok || len(results) != 2 {
		t.Fatalf("batch=%v", out)
	}
	out = request("/api/v1/application-authz/filter", filter, "fixture-app-key", 200)
	if ids, ok := out["resource_ids"].([]any); !ok || len(ids) != 1 || ids[0] != "conalog-grant" {
		t.Fatalf("filter=%v", out)
	}
	request("/api/v1/application-authz/filter", filter, "wrong-key", 401)
	request("/api/v1/authz/check", check, "fixture-app-key", 503)
	exec("DELETE FROM application_actions WHERE application_id='fixture-app'")
	out = request("/api/v1/application-authz/filter", filter, "fixture-app-key", 200)
	if ids, ok := out["resource_ids"].([]any); !ok || len(ids) != 0 {
		t.Fatalf("revoked filter=%v", out)
	}
	runtime.Close()
	request("/api/v1/application-authz/filter", filter, "fixture-app-key", 503)
}
