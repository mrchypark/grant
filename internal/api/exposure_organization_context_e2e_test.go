package api

import (
	"context"
	"encoding/base64"
	"encoding/json"
	"io"
	"net"
	"net/http"
	"net/http/httptest"
	"os"
	"os/exec"
	"path/filepath"
	"strings"
	"sync/atomic"
	"syscall"
	"testing"
	"time"

	"github.com/mrchypark/grant/internal/grant"
)

// Opt in with EXPOSURE_SOURCE_DIR pointing at the sibling Exposure checkout.
func TestExposureOrganizationContextE2E(t *testing.T) {
	source := os.Getenv("EXPOSURE_SOURCE_DIR")
	if source == "" {
		t.Skip("set EXPOSURE_SOURCE_DIR to run the real Exposure binary")
	}
	ctx := context.Background()
	payload, _ := json.Marshal(map[string]any{"id": "fixtureuser0001", "collectionId": "users", "type": "auth", "exp": time.Now().Add(time.Hour).Unix()})
	token := base64.RawURLEncoding.EncodeToString([]byte(`{"alg":"HS256","typ":"JWT"}`)) + "." + base64.RawURLEncoding.EncodeToString(payload) + ".Zml4dHVyZQ"
	idp := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		if r.Method != "POST" || r.URL.Path != "/api/collections/users/auth-refresh" || r.Header.Get("Authorization") != "Bearer "+token {
			http.Error(w, "unknown token", 401)
			return
		}
		_ = json.NewEncoder(w).Encode(map[string]any{"token": token, "record": map[string]any{"id": "fixtureuser0001", "collectionId": "users", "collectionName": "users", "name": "Fixture"}})
	}))
	defer idp.Close()
	cfg := grant.Config{Security: grant.SecurityConfig{BootstrapUserID: "fixture-user", MasterKey: "fixture-master", AppAuthKey: "fixture-app-key", HTTPCapabilityPEPClientID: "fixture-pep"}, Auth: grant.AuthConfig{ProviderMode: "pocketbase", PocketBase: &grant.PocketBaseConfig{BaseURL: idp.URL, Collection: "users", ApplicationID: "fixture-app"}}, Database: grant.DatabaseConfig{DataDir: t.TempDir(), DefaultReadConsistency: grant.ReadStrong}}
	store, err := grant.OpenStore(ctx, cfg)
	if err != nil {
		t.Fatal(err)
	}
	defer store.Close()
	if err = store.EnsureBootstrapData(ctx); err != nil {
		t.Fatal(err)
	}
	sqlExec := func(sql string, args ...any) {
		t.Helper()
		if _, e := store.Exec(ctx, sql, args...); e != nil {
			t.Fatal(e)
		}
	}
	sqlExec("INSERT INTO user_identities(id,user_id,provider,issuer,collection_id,external_user_id,last_seen_at,created_at,updated_at) VALUES('fixture-identity','fixture-user','pocketbase',?,'users','fixtureuser0001','now','now','now')", idp.URL)
	sqlExec("INSERT INTO service_providers(id,name,active,created_at,updated_at) VALUES('inventory','Inventory',1,'now','now')")
	for _, code := range []string{"item.read", "app.only"} {
		sqlExec("INSERT INTO actions(id,provider_id,code,name,risk_level,visibility_granting,grantable_to_applications,created_at,updated_at) VALUES(?,'inventory',?,?,'low',0,1,'now','now')", "inventory."+code, code, code)
	}
	app := grant.Application{ID: "fixture-app", OrganizationID: "fixture-user", Slug: "fixture-app", Name: "Fixture", ApplicationType: "confidential", AppType: "server", FirstParty: true, ConsentPolicy: "auto_approve", Active: true, ProvisioningStatus: "ready", CreatedByUserID: "fixture-user"}
	if err = store.InsertApplicationWithRedirectURI(ctx, app, nil); err != nil {
		t.Fatal(err)
	}
	sqlExec("INSERT INTO application_actions(application_id,action_id) SELECT 'fixture-app',id FROM actions WHERE provider_id='inventory'")
	sqlExec("INSERT INTO roles(id,organization_id,code,name,created_at,updated_at) VALUES('fixture-role',NULL,'fixture-role','Fixture','now','now')")
	sqlExec("INSERT INTO role_actions(role_id,action_id) VALUES('fixture-role','inventory.item.read')")
	sqlExec("INSERT INTO role_bindings(id,subject_type,subject_id,subject_organization_id,provider_id,resource_type,resource_id,role_id,state,created_by_user_id,created_at,updated_at) VALUES('fixture-binding-org','user','fixture-user','fixture-user','inventory','item','allowed','fixture-role','active','fixture-user','now','now')")
	svc := grant.NewService(cfg, store)
	server := httptest.NewServer(New(svc, false))
	defer server.Close()
	call := func(method, path string, value any, want int) []byte {
		t.Helper()
		raw, e := json.Marshal(value)
		if e != nil {
			t.Fatal(e)
		}
		req, e := http.NewRequest(method, server.URL+path, strings.NewReader(string(raw)))
		if e != nil {
			t.Fatal(e)
		}
		req.Header.Set("X-Grant-Master-Key", "fixture-master")
		req.Header.Set("Content-Type", "application/json")
		resp, e := http.DefaultClient.Do(req)
		if e != nil {
			t.Fatal(e)
		}
		defer resp.Body.Close()
		b, e := io.ReadAll(resp.Body)
		if e != nil {
			t.Fatal(e)
		}
		if resp.StatusCode != want {
			t.Fatalf("%s %s: %d %s", method, path, resp.StatusCode, b)
		}
		return b
	}
	call("PUT", "/ops/orgless-bindings/fixture-global-binding", grant.OrglessUserBinding{SubjectID: "fixture-user", ProviderID: "inventory", ResourceType: "item", ResourceID: "allowed", ActionID: "inventory.item.read", State: "active"}, 200)
	call("PUT", "/ops/providers/inventory/enforcers/fixture-pep", grant.UpsertProviderPepClient{Lifecycle: "active"}, 200)
	stage := grant.StageHttpCapabilityGeneration{SchemaVersion: 1, Generation: 1, Capabilities: []grant.HttpCapability{}}
	for _, v := range []struct{ id, policy, action string }{{"optional", "optional", "inventory.item.read"}, {"required", "required", "inventory.item.read"}, {"legacy", "", "inventory.item.read"}, {"app-only", "optional", "inventory.app.only"}} {
		action := v.action
		c := grant.HttpCapability{CapabilityID: v.id, Version: 1, Enabled: true, AccessMode: "authorized", Method: "GET", PathTemplate: "/" + v.id + "/{itemId}", ActionID: &action, Resource: &grant.HttpCapabilityResource{ResourceType: "item", ID: grant.HttpResourceExtractor{Kind: "pathSegment", Name: "itemId", Format: "asciiToken"}}, Query: grant.HttpQueryPolicy{Mode: "none"}, Request: grant.HttpRequestPolicy{Body: "none", ForwardHeaders: []string{"x-organization-id"}, Authorization: "forward-user"}, Response: grant.HttpResponsePolicy{MaxBytes: 4096, ForwardHeaders: []string{"content-type"}, Redirects: "deny"}, OrganizationContext: v.policy}
		c.ContractDigest, err = grant.HttpCapabilityContractDigest(c)
		if err != nil {
			t.Fatal(err)
		}
		stage.Capabilities = append(stage.Capabilities, c)
	}
	stage.CatalogDigest, err = grant.HttpCapabilityCatalogDigest("inventory", stage)
	if err != nil {
		t.Fatal(err)
	}
	call("POST", "/api/v1/admin/providers/inventory/http-capability-generations", stage, 201)
	call("POST", "/api/v1/admin/providers/inventory/http-capability-active", grant.ActivateHttpCapabilityGeneration{TargetGeneration: 1, TargetCatalogDigest: stage.CatalogDigest}, 200)
	binary := filepath.Join(t.TempDir(), "exposure")
	buildCtx, cancelBuild := context.WithTimeout(ctx, 2*time.Minute)
	defer cancelBuild()
	build := exec.CommandContext(buildCtx, "go", "build", "-o", binary, "./cmd/exposure")
	build.Dir = source
	if out, e := build.CombinedOutput(); e != nil {
		t.Fatalf("build Exposure: %v\n%s", e, out)
	}
	var hits atomic.Int64
	upstream := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		hits.Add(1)
		w.Header().Set("Content-Type", "application/json")
		_ = json.NewEncoder(w).Encode(map[string]string{"path": r.URL.Path, "organization": r.Header.Get("X-Organization-ID"), "authorization": r.Header.Get("Authorization")})
	}))
	defer upstream.Close()
	// This relay always executes the real publisher/PDP first, then corrupts its
	// response for trust-boundary checks. It never invents an authorization result.
	var fault atomic.Int32
	var decisions atomic.Int64
	grantClient := &http.Client{Timeout: 5 * time.Second}
	relay := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		forward, e := http.NewRequestWithContext(r.Context(), r.Method, server.URL+r.URL.RequestURI(), r.Body)
		if e != nil {
			http.Error(w, e.Error(), 502)
			return
		}
		forward.Header = r.Header.Clone()
		response, e := grantClient.Do(forward)
		if e != nil {
			http.Error(w, e.Error(), 502)
			return
		}
		defer response.Body.Close()
		b, e := io.ReadAll(response.Body)
		if e != nil {
			http.Error(w, e.Error(), 502)
			return
		}
		if r.URL.Path == "/api/v2/http-capabilities/decide" {
			decisions.Add(1)
		}
		if response.StatusCode == 200 {
			var value map[string]any
			if json.Unmarshal(b, &value) == nil {
				if fault.Load() == 1 && r.URL.Path == "/api/v2/http-capabilities/decide" {
					value["requestBinding"] = "tampered"
				}
				if strings.HasPrefix(r.URL.Path, "/api/v2/http-capabilities/catalog/") {
					if fault.Load() == 2 {
						value["catalogDigest"] = "sha256:" + strings.Repeat("0", 64)
					}
					if fault.Load() == 3 {
						if caps, ok := value["capabilities"].([]any); ok {
							for _, raw := range caps {
								if c, ok := raw.(map[string]any); ok {
									if c["capabilityId"] == "legacy" {
										c["organizationContext"] = "optional"
									}
								}
							}
						}
					}
				}
				encoded, encodeErr := json.Marshal(value)
				if encodeErr != nil {
					http.Error(w, encodeErr.Error(), 500)
					return
				}
				b = encoded
			}
		}
		for k, vs := range response.Header {
			if strings.EqualFold(k, "Content-Length") {
				continue
			}
			for _, v := range vs {
				w.Header().Add(k, v)
			}
		}
		w.WriteHeader(response.StatusCode)
		_, _ = w.Write(b)
	}))
	defer relay.Close()
	address := func() string {
		t.Helper()
		ln, e := net.Listen("tcp", "127.0.0.1:0")
		if e != nil {
			t.Fatal(e)
		}
		addr := ln.Addr().String()
		if e = ln.Close(); e != nil {
			t.Fatal(e)
		}
		return addr
	}
	client := &http.Client{Timeout: 2 * time.Second}
	start := func(t *testing.T, ready bool) string {
		t.Helper()
		dataAddr, adminAddr := address(), address()
		for adminAddr == dataAddr {
			adminAddr = address()
		}
		logPath := filepath.Join(t.TempDir(), "exposure.log")
		logFile, e := os.Create(logPath)
		if e != nil {
			t.Fatal(e)
		}
		command := exec.Command(binary)
		for _, v := range os.Environ() {
			if !strings.HasPrefix(v, "EXPOSURE_") {
				command.Env = append(command.Env, v)
			}
		}
		settings := map[string]string{
			"PROXY_ENABLED": "true", "GRANT_LEARNER_URL": relay.URL, "PROVIDER_ORIGIN": upstream.URL,
			"DATA_ADDR": dataAddr, "ADMIN_ADDR": adminAddr, "PROVIDER_ID": "inventory", "PEP_CLIENT_ID": "fixture-pep",
			"EXPECTED_CATALOG_DIGEST": stage.CatalogDigest, "GRANT_APP_AUTH_KEY": "fixture-app-key",
			"AUTHN_LIMITER_KEY": strings.Repeat("f", 32), "PROVIDER_AUTHORIZATION_MODE": "forward-user", "CREDENTIAL_MODE": "pocketbase",
			"POCKETBASE_COLLECTION_ID": "users", "POCKETBASE_ISSUER": idp.URL, "POCKETBASE_APPLICATION_ID": "fixture-app",
		}
		for k, v := range settings {
			command.Env = append(command.Env, "EXPOSURE_"+k+"="+v)
		}
		command.Stdout = logFile
		command.Stderr = logFile
		if e = command.Start(); e != nil {
			logFile.Close()
			t.Fatal(e)
		}
		done := make(chan error, 1)
		go func() { done <- command.Wait() }()
		t.Cleanup(func() {
			_ = command.Process.Signal(syscall.SIGTERM)
			select {
			case <-done:
			case <-time.After(5 * time.Second):
				_ = command.Process.Kill()
				<-done
			}
			_ = logFile.Close()
			if t.Failed() {
				b, _ := os.ReadFile(logPath)
				t.Logf("Exposure log:\n%s", b)
			}
		})
		deadline := time.Now().Add(15 * time.Second)
		for time.Now().Before(deadline) {
			resp, e := client.Get("http://" + adminAddr + "/readyz")
			if e == nil {
				io.Copy(io.Discard, resp.Body)
				resp.Body.Close()
				if ready && resp.StatusCode == 200 || !ready && resp.StatusCode == 503 {
					return "http://" + dataAddr
				}
			}
			time.Sleep(30 * time.Millisecond)
		}
		t.Fatal("Exposure readiness deadline exceeded")
		return ""
	}
	t.Run("authorization", func(t *testing.T) {
		origin := start(t, true)
		for _, tc := range []struct {
			name, path, org, bearer string
			status                  int
			decisionDelta           int64
		}{
			{"optional global allow", "/optional/allowed", "", token, 200, 1},
			{"optional global deny", "/optional/blocked", "", token, 403, 1},
			{"optional org allow", "/optional/allowed", "fixture-user", token, 200, 1},
			{"optional org deny", "/optional/blocked", "fixture-user", token, 403, 1},
			{"required absent", "/required/allowed", "", token, 401, 0},
			{"required present", "/required/allowed", "fixture-user", token, 200, 1},
			{"legacy absent", "/legacy/allowed", "", token, 401, 0},
			{"legacy present", "/legacy/allowed", "fixture-user", token, 200, 1},
			{"application admission is not user permission", "/app-only/allowed", "", token, 403, 1},
			{"missing bearer", "/optional/allowed", "", "", 401, 0},
			{"malformed bearer", "/optional/allowed", "", "not-a-jwt", 401, 0},
			{"unknown JWT bearer", "/optional/allowed", "", strings.TrimSuffix(token, "Zml4dHVyZQ") + "dW5rbm93bg", 401, 1},
		} {
			t.Run(tc.name, func(t *testing.T) {
				before, n := hits.Load(), decisions.Load()
				req, e := http.NewRequest("GET", origin+tc.path, nil)
				if e != nil {
					t.Fatal(e)
				}
				if tc.bearer != "" {
					req.Header.Set("Authorization", "Bearer "+tc.bearer)
				}
				if tc.org != "" {
					req.Header.Set("X-Organization-ID", tc.org)
				}
				resp, e := client.Do(req)
				if e != nil {
					t.Fatal(e)
				}
				b, e := io.ReadAll(resp.Body)
				resp.Body.Close()
				if e != nil {
					t.Fatal(e)
				}
				wantHits := int64(0)
				if tc.status == 200 {
					wantHits = 1
				}
				if resp.StatusCode != tc.status || decisions.Load()-n != tc.decisionDelta || hits.Load()-before != wantHits {
					t.Fatalf("status=%d want=%d decisions=%d want=%d hits=%d want=%d body=%s", resp.StatusCode, tc.status, decisions.Load()-n, tc.decisionDelta, hits.Load()-before, wantHits, b)
				}
				if tc.status == 200 {
					var got map[string]string
					if e = json.Unmarshal(b, &got); e != nil {
						t.Fatal(e)
					}
					if got["path"] != tc.path || got["organization"] != tc.org || got["authorization"] != "Bearer "+token {
						t.Fatalf("forwarded request: %s", b)
					}
				}
			})
		}
		t.Run("tampered decision", func(t *testing.T) {
			fault.Store(1)
			defer fault.Store(0)
			before, n := hits.Load(), decisions.Load()
			req, _ := http.NewRequest("GET", origin+"/optional/allowed", nil)
			req.Header.Set("Authorization", "Bearer "+token)
			resp, e := client.Do(req)
			if e != nil {
				t.Fatal(e)
			}
			defer resp.Body.Close()
			if resp.StatusCode != 502 || hits.Load() != before || decisions.Load() != n+1 {
				t.Fatalf("tampered real decision: status=%d want=502 hits=%d want=0 decisions=%d want=1", resp.StatusCode, hits.Load()-before, decisions.Load()-n)
			}
		})
	})
	for _, tc := range []struct {
		name string
		mode int32
	}{{"tampered catalog digest", 2}, {"old omitted policy digest cannot authorize optional", 3}} {
		t.Run(tc.name, func(t *testing.T) {
			fault.Store(tc.mode)
			defer fault.Store(0)
			before, n := hits.Load(), decisions.Load()
			origin := start(t, false)
			req, _ := http.NewRequest("GET", origin+"/legacy/allowed", nil)
			req.Header.Set("Authorization", "Bearer "+token)
			resp, e := client.Do(req)
			if e != nil {
				t.Fatal(e)
			}
			defer resp.Body.Close()
			if resp.StatusCode != 503 || hits.Load() != before || decisions.Load() != n {
				t.Fatalf("invalid catalog: status=%d want=503 hits=%d want=0 decisions=%d want=0", resp.StatusCode, hits.Load()-before, decisions.Load()-n)
			}
			var failure struct {
				ErrorCode string `json:"errorCode"`
			}
			if e = json.NewDecoder(resp.Body).Decode(&failure); e != nil {
				t.Fatal(e)
			}
			if failure.ErrorCode != "not_ready" {
				t.Fatalf("invalid catalog: errorCode=%q want=not_ready", failure.ErrorCode)
			}
		})
	}
}
