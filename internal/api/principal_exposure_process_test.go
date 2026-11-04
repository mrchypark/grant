package api

import (
	"bytes"
	"context"
	"crypto/sha256"
	"encoding/base64"
	"encoding/json"
	"fmt"
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

// This harness exercises a synthetic Goauthy protocol fixture, never a cloud
// Goauthy deployment or the issuer owner's human-user acceptance contract.
// Exposure must be a CI-supplied artifact; do not inspect or build its checkout.
func TestPrincipalExposureProcess(t *testing.T) {
	// Leave at least ten seconds for owned-process cleanup before CI's 2m limit.
	ctx, cancel := context.WithTimeout(context.Background(), 80*time.Second)
	defer cancel()
	binary := os.Getenv("GRANT_EXPOSURE_BINARY")
	if binary == "" {
		if os.Getenv("GRANT_EXPOSURE_INTEROP") == "1" {
			t.Fatal("GRANT_EXPOSURE_INTEROP=1 requires GRANT_EXPOSURE_BINARY")
		}
		t.Skip("set GRANT_EXPOSURE_BINARY to a CI-supplied Exposure artifact; no interop executed")
	}
	expectedSHA256 := os.Getenv("GRANT_EXPOSURE_BINARY_SHA256")
	if len(expectedSHA256) != 64 || strings.IndexFunc(expectedSHA256, func(r rune) bool {
		return !(r >= '0' && r <= '9' || r >= 'a' && r <= 'f')
	}) != -1 {
		t.Fatal("GRANT_EXPOSURE_BINARY_SHA256 must contain exactly 64 lowercase hexadecimal characters when GRANT_EXPOSURE_BINARY is supplied")
	}
	var err error
	binary, err = filepath.Abs(binary)
	if err != nil {
		t.Fatal(err)
	}
	info, err := os.Stat(binary)
	if err != nil {
		t.Fatal(err)
	}
	if !info.Mode().IsRegular() || info.Mode().Perm()&0111 == 0 {
		t.Fatal("GRANT_EXPOSURE_BINARY must be an executable regular file (CI must chmod +x)")
	}
	// The CI downloader pins the ZIP separately. Verify its executable here too.
	artifact, err := os.Open(binary)
	if err != nil {
		t.Fatal(err)
	}
	hash := sha256.New()
	_, hashErr := io.Copy(hash, artifact)
	closeErr := artifact.Close()
	if hashErr != nil || closeErr != nil {
		t.Fatalf("read Exposure artifact: hash=%v close=%v", hashErr, closeErr)
	}
	if actual := fmt.Sprintf("%x", hash.Sum(nil)); actual != expectedSHA256 {
		t.Fatalf("Exposure artifact SHA-256=%s want=%s", actual, expectedSHA256)
	}
	t.Run("user", func(t *testing.T) {
		f := newPrincipalExposureSystem(t, ctx)
		digest := f.publish(t, "user")
		var fault atomic.Int32
		relay := f.decisionRelay(t, &fault)
		origin := f.startExposure(t, binary, relay.URL, digest, false)
		request := func(t *testing.T, path, token, org string, want int) {
			t.Helper()
			before := f.upstreamHits.Load()
			decisions := f.decisions.Load()
			status := principalExposureRequest(t, ctx, origin+path, token, org)
			if want != 0 && status != want || want == 0 && status < 400 {
				t.Fatalf("status=%d want=%d (0 means denial)", status, want)
			}
			var delta int64
			if want == 200 {
				delta = 1
			}
			if f.upstreamHits.Load()-before != delta {
				t.Fatalf("upstream delta=%d want=%d", f.upstreamHits.Load()-before, delta)
			}
			if f.decisions.Load()-decisions != 1 {
				t.Fatalf("real Grant decision requests=%d want=1", f.decisions.Load()-decisions)
			}
			f.assertNoApplications(t)
		}
		request(t, "/items/allowed", f.tokens["user"], "fixture-owner", 200)
		request(t, "/items/blocked", f.tokens["user"], "fixture-owner", 403)
		for _, name := range []string{"wrong-client", "wrong-audience", "wrong-sub", "wrong-issuer", "missing-scope"} {
			t.Run(name, func(t *testing.T) {
				before := f.introspections.Load()
				request(t, "/items/allowed", f.tokens[name], "fixture-owner", 0)
				if delta := f.introspections.Load() - before; delta != 1 {
					t.Fatalf("authoritative introspections=%d want=1", delta)
				}
			})
		}
		request(t, "/items/allowed", f.tokens["user"], "other-org", 0)
		for _, test := range []struct {
			name  string
			fault int32
		}{{"request mode injection", 1}, {"stale revision", 2}} {
			t.Run(test.name, func(t *testing.T) {
				fault.Store(test.fault)
				defer fault.Store(0)
				request(t, "/items/allowed", f.tokens["user"], "fixture-owner", 0)
			})
		}
		// Each call creates a fresh HTTP request; no upstream operation is replayed.
		f.revokeResourceRole(t)
		request(t, "/items/allowed", f.tokens["user"], "fixture-owner", 403)
	})
	t.Run("server mode changed after catalog", func(t *testing.T) {
		f := newPrincipalExposureSystem(t, ctx)
		digest := f.publish(t, "user")
		var fault atomic.Int32
		relay := f.decisionRelay(t, &fault)
		origin := f.startExposure(t, binary, relay.URL, digest, false)
		mode := "application"
		if _, err := grant.NewService(f.config, f.store).SetProviderPepClientAdmin(ctx, "fixture-provider", "fixture-pep", grant.UpsertProviderPepClient{Lifecycle: "active", AuthorizationMode: &mode}); err != nil {
			t.Fatal(err)
		}
		if status := principalExposureRequest(t, ctx, origin+"/items/allowed", f.tokens["user"], "fixture-owner"); status < 400 || f.upstreamHits.Load() != 0 {
			t.Fatalf("changed server mode: status=%d upstream=%d", status, f.upstreamHits.Load())
		}
		f.assertNoApplications(t)
	})
	t.Run("legacy application", func(t *testing.T) {
		f := newPrincipalExposureSystem(t, ctx)
		app := grant.Application{ID: "unused-legacy-application", OrganizationID: "fixture-owner", Slug: "legacy", Name: "Legacy", ApplicationType: "confidential", AppType: "server", FirstParty: true, ConsentPolicy: "auto_approve", Active: true, ProvisioningStatus: "ready", CreatedByUserID: "fixture-owner"}
		if err := f.store.InsertApplicationWithRedirectURI(ctx, app, nil); err != nil {
			t.Fatal(err)
		}
		if _, err := f.store.Exec(ctx, "INSERT INTO application_actions(application_id,action_id) VALUES(?,'fixture-provider.item.read')", app.ID); err != nil {
			t.Fatal(err)
		}
		digest := f.publish(t, "application")
		var fault atomic.Int32
		relay := f.decisionRelay(t, &fault)
		origin := f.startExposure(t, binary, relay.URL, digest, true)
		if status := principalExposureRequest(t, ctx, origin+"/items/allowed", f.tokens["user"], "fixture-owner"); status != 200 || f.upstreamHits.Load() != 1 {
			t.Fatalf("legacy status=%d upstream=%d", status, f.upstreamHits.Load())
		}
	})
}

type principalExposureClaims struct {
	subject, client, audience, profileSubject string
	wrongIssuer, missingScope                 bool
}

type principalExposureSystem struct {
	ctx                                     context.Context
	config                                  grant.Config
	mode                                    string
	store                                   *grant.Store
	grantServer, issuer, upstream           *httptest.Server
	tokens                                  map[string]string
	upstreamHits, introspections, userinfos atomic.Int64
	decisions                               atomic.Int64
}

func newPrincipalExposureSystem(t *testing.T, ctx context.Context) *principalExposureSystem {
	t.Helper()
	f := &principalExposureSystem{ctx: ctx, tokens: make(map[string]string)}
	// A compact JWT is an untrusted Exposure parsing hint. Grant authenticates
	// against this server's token lookup, not against those payload claims.
	variants := map[string]principalExposureClaims{
		"user":           {"resource-owner", "fixture-login", "https://grant.example.test/", "resource-owner", false, false},
		"wrong-client":   {"resource-owner", "other-client", "https://grant.example.test/", "resource-owner", false, false},
		"wrong-audience": {"resource-owner", "fixture-login", "https://other.example.test/", "resource-owner", false, false},
		"wrong-sub":      {"resource-owner", "fixture-login", "https://grant.example.test/", "different-user", false, false},
		"wrong-issuer":   {"resource-owner", "fixture-login", "https://grant.example.test/", "resource-owner", true, false},
		"missing-scope":  {"resource-owner", "fixture-login", "https://grant.example.test/", "resource-owner", false, true},
	}
	byToken := make(map[string]principalExposureClaims)
	f.issuer = principalExposureServer(t, ctx, http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		w.Header().Set("Content-Type", "application/json")
		switch r.URL.Path {
		case "/oidc/introspect":
			f.introspections.Add(1)
			id, secret, ok := r.BasicAuth()
			if r.Method != http.MethodPost || !ok || id != "fixture-introspector" || secret != "fixture-secret" {
				http.Error(w, "unauthorized", http.StatusUnauthorized)
				return
			}
			if err := r.ParseForm(); err != nil {
				http.Error(w, "invalid form", http.StatusBadRequest)
				return
			}
			claims, ok := byToken[r.Form.Get("token")]
			if !ok {
				_ = json.NewEncoder(w).Encode(map[string]any{"active": false})
				return
			}
			response := map[string]any{"active": true, "iss": f.issuer.URL, "sub": claims.subject, "client_id": claims.client, "aud": []string{claims.audience}, "scope": "openid grant", "exp": time.Now().Add(time.Hour).Unix(), "token_type": "Bearer"}
			if claims.wrongIssuer {
				response["iss"] = "https://other.example.test"
			}
			if claims.missingScope {
				delete(response, "scope")
			}
			_ = json.NewEncoder(w).Encode(response)
		case "/oidc/userinfo":
			f.userinfos.Add(1)
			claims, ok := byToken[bearerPrincipalExposureToken(r.Header.Get("Authorization"))]
			if r.Method != http.MethodGet || !ok {
				http.Error(w, "unauthorized", http.StatusUnauthorized)
				return
			}
			_ = json.NewEncoder(w).Encode(map[string]any{"sub": claims.profileSubject, "name": "Ordinary User"})
		default:
			http.NotFound(w, r)
		}
	}), true)
	// Seed the immutable lookup before any external request can reach the fixture.
	for name, claims := range variants {
		payload, err := json.Marshal(map[string]any{"iss": f.issuer.URL, "sub": "resource-owner", "aud": []string{"https://grant.example.test/"}, "exp": time.Now().Add(time.Hour).Unix(), "jti": name})
		if err != nil {
			t.Fatal(err)
		}
		token := base64.RawURLEncoding.EncodeToString([]byte(`{"alg":"HS256","typ":"JWT"}`)) + "." + base64.RawURLEncoding.EncodeToString(payload) + ".Zml4dHVyZQ"
		f.tokens[name], byToken[token] = token, claims
	}
	trustLocalIssuerForGrantAPI(t, f.issuer.Certificate())
	cfg := grant.Config{
		Security: grant.SecurityConfig{BootstrapUserID: "fixture-owner", MasterKey: "fixture-master", AppAuthKey: "fixture-app-key", HTTPCapabilityPEPClientID: "fixture-pep"},
		Auth:     grant.AuthConfig{ProviderMode: "goauthy", Goauthy: &grant.GoauthyConfig{Issuer: f.issuer.URL, IntrospectionClientID: "fixture-introspector", IntrospectionClientSecret: "fixture-secret", ExpectedClientID: "fixture-login", Resource: "https://grant.example.test/", ApplicationID: "unused-legacy-application", RequiredScopes: []string{"openid", "grant"}}},
		Database: grant.DatabaseConfig{DataDir: t.TempDir(), DefaultReadConsistency: grant.ReadStrong},
	}
	store, err := grant.OpenStore(ctx, cfg)
	if err != nil {
		t.Fatal(err)
	}
	f.store = store
	f.config = cfg
	t.Cleanup(func() {
		if err := store.Close(); err != nil {
			t.Errorf("close Grant fixture: %v", err)
		}
	})
	if err := store.EnsureBootstrapData(ctx); err != nil {
		t.Fatal(err)
	}
	for _, statement := range []struct {
		sql  string
		args []any
	}{
		{"INSERT INTO users(id,display_name,active,created_at) VALUES('fixture-user','Ordinary User',1,'now')", nil},
		{"INSERT INTO user_identities(id,user_id,provider,issuer,collection_id,external_user_id,last_seen_at,created_at,updated_at) VALUES('fixture-user-identity','fixture-user','goauthy',?,'','resource-owner','now','now','now')", []any{f.issuer.URL}},
		{"INSERT INTO organization_memberships(id,organization_id,user_id,membership_role,status,created_at,updated_at) VALUES('fixture-member','fixture-owner','fixture-user','member','active','now','now')", nil},
		{"INSERT INTO service_providers(id,name,active,created_at,updated_at) VALUES('fixture-provider','Inventory',1,'now','now')", nil},
		{"INSERT INTO actions(id,provider_id,code,name,risk_level,visibility_granting,grantable_to_applications,created_at,updated_at) VALUES('fixture-provider.item.read','fixture-provider','item.read','Read','low',0,1,'now','now')", nil},
		{"INSERT INTO roles(id,organization_id,code,name,created_at,updated_at) VALUES('fixture-reader','fixture-owner','fixture-reader','Reader','now','now')", nil},
		{"INSERT INTO role_actions(role_id,action_id) VALUES('fixture-reader','fixture-provider.item.read')", nil},
		{"INSERT INTO role_bindings(id,subject_type,subject_id,subject_organization_id,provider_id,resource_type,resource_id,role_id,state,created_by_user_id,created_at,updated_at) VALUES('fixture-resource-reader','user','fixture-user','fixture-owner','fixture-provider','item','allowed','fixture-reader','active','fixture-owner','now','now')", nil},
	} {
		if _, err := store.Exec(ctx, statement.sql, statement.args...); err != nil {
			t.Fatal(err)
		}
	}
	f.assertNoApplications(t)
	f.grantServer = principalExposureServer(t, ctx, New(grant.NewService(cfg, store), false), false)
	f.upstream = principalExposureServer(t, ctx, http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		f.upstreamHits.Add(1)
		w.Header().Set("Content-Type", "application/json")
		_ = json.NewEncoder(w).Encode(map[string]string{"path": r.URL.Path})
	}), false)
	return f
}

func bearerPrincipalExposureToken(value string) string {
	const prefix = "Bearer "
	if len(value) < len(prefix) || value[:len(prefix)] != prefix {
		return ""
	}
	return value[len(prefix):]
}

func (f *principalExposureSystem) assertNoApplications(t *testing.T) {
	t.Helper()
	rows, err := f.store.Query(f.ctx, grant.ReadStrong, "SELECT id FROM applications")
	if err != nil || len(rows) != 0 {
		t.Fatalf("zero-application fixture: rows=%v err=%v", rows, err)
	}
}

func (f *principalExposureSystem) revokeResourceRole(t *testing.T) {
	t.Helper()
	result, err := f.store.Exec(f.ctx, "UPDATE role_bindings SET state='revoked' WHERE id='fixture-resource-reader' AND state='active'")
	if err != nil {
		t.Fatal(fmt.Errorf("revoke precise-resource role: %w", err))
	}
	if result != 1 {
		t.Fatalf("revoke precise-resource role affected %d rows, want 1", result)
	}
	f.assertNoApplications(t)
}

func (f *principalExposureSystem) publish(t *testing.T, mode string) string {
	t.Helper()
	f.mode = mode
	svc := grant.NewService(f.config, f.store)
	ctx := f.ctx
	if _, err := svc.SetProviderPepClientAdmin(ctx, "fixture-provider", "fixture-pep", grant.UpsertProviderPepClient{Lifecycle: "active", AuthorizationMode: &mode}); err != nil {
		t.Fatal(err)
	}
	action := "fixture-provider.item.read"
	cap := grant.HttpCapability{CapabilityID: "item-read", Version: 1, Enabled: true, AccessMode: "authorized", Method: "GET", PathTemplate: "/items/{itemId}", ActionID: &action, Resource: &grant.HttpCapabilityResource{ResourceType: "item", ID: grant.HttpResourceExtractor{Kind: "pathSegment", Name: "itemId", Format: "asciiToken"}}, Query: grant.HttpQueryPolicy{Mode: "none"}, Request: grant.HttpRequestPolicy{Body: "none", ForwardHeaders: []string{"x-organization-id"}, Authorization: "forward-user"}, Response: grant.HttpResponsePolicy{MaxBytes: 4096, ForwardHeaders: []string{"content-type"}, Redirects: "deny"}, OrganizationContext: "required"}
	var err error
	cap.ContractDigest, err = grant.HttpCapabilityContractDigest(cap)
	if err != nil {
		t.Fatal(err)
	}
	stage := grant.StageHttpCapabilityGeneration{SchemaVersion: 1, Generation: 1, Capabilities: []grant.HttpCapability{cap}}
	stage.CatalogDigest, err = grant.HttpCapabilityCatalogDigest("fixture-provider", stage)
	if err != nil {
		t.Fatal(err)
	}
	if _, err := svc.StageHttpCapabilityGenerationAdmin(ctx, "fixture-provider", stage); err != nil {
		t.Fatal(err)
	}
	if _, err := svc.ActivateHttpCapabilityGenerationAdmin(ctx, "fixture-provider", grant.ActivateHttpCapabilityGeneration{TargetGeneration: 1, TargetCatalogDigest: stage.CatalogDigest}); err != nil {
		t.Fatal(err)
	}
	return stage.CatalogDigest
}

// Faults alter only requests to the actual Grant API. Responses always come from
// the real service, so this relay cannot fabricate an authorization decision.
func (f *principalExposureSystem) decisionRelay(t *testing.T, fault *atomic.Int32) *httptest.Server {
	t.Helper()
	client := &http.Client{Timeout: 5 * time.Second}
	t.Cleanup(client.CloseIdleConnections)
	relay := principalExposureServer(t, f.ctx, http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		body, err := io.ReadAll(io.LimitReader(r.Body, 1<<20))
		if err != nil {
			http.Error(w, "read relay request", 502)
			return
		}
		decide := r.URL.Path == "/api/v2/http-capabilities/decide"
		if decide {
			var wire map[string]json.RawMessage
			if err := json.Unmarshal(body, &wire); err != nil {
				t.Errorf("invalid Exposure decision JSON: %v", err)
			}
			if f.mode == "application" {
				if _, ok := wire["wireVersion"]; ok {
					t.Error("legacy Exposure request unexpectedly includes wireVersion")
				}
				if _, ok := wire["pepPolicyRevision"]; ok {
					t.Error("legacy Exposure request unexpectedly includes pepPolicyRevision")
				}
			} else if string(wire["wireVersion"]) != "2" {
				t.Error("user Exposure request must use wireVersion 2")
			}
			switch fault.Load() {
			case 1:
				wire["authorizationMode"] = json.RawMessage(`"application"`)
			case 2:
				wire["pepPolicyRevision"] = json.RawMessage(`9223372036854775807`)
			}
			body, err = json.Marshal(wire)
			if err != nil {
				http.Error(w, "encode relay request", 502)
				return
			}
		}
		req, err := http.NewRequestWithContext(r.Context(), r.Method, f.grantServer.URL+r.URL.RequestURI(), bytes.NewReader(body))
		if err != nil {
			http.Error(w, "create relay request", 502)
			return
		}
		req.Header = r.Header.Clone()
		req.Header.Del("Content-Length")
		res, err := client.Do(req)
		if err != nil {
			http.Error(w, "real Grant unavailable", 502)
			return
		}
		defer res.Body.Close()
		if decide {
			f.decisions.Add(1)
		}
		response, err := io.ReadAll(io.LimitReader(res.Body, 1<<20))
		if err != nil {
			http.Error(w, "read real Grant response", 502)
			return
		}
		if decide && fault.Load() != 0 && res.StatusCode < 400 {
			t.Error("real Grant did not reject injected policy evidence")
		}
		if decide && fault.Load() == 2 && res.StatusCode != http.StatusForbidden {
			t.Errorf("real Grant stale revision status=%d want=403", res.StatusCode)
		}
		if res.StatusCode == 200 && (decide || strings.Contains(r.URL.Path, "/http-capabilities/catalog/")) {
			var wire map[string]json.RawMessage
			if err := json.Unmarshal(response, &wire); err != nil {
				t.Errorf("invalid Grant wire JSON: %v", err)
			}
			if f.mode == "application" {
				for _, key := range []string{"wireVersion", "pepPolicyRevision", "authorizationMode"} {
					if _, ok := wire[key]; ok {
						t.Errorf("legacy wire unexpectedly includes %s", key)
					}
				}
			} else if decide {
				if string(wire["wireVersion"]) != "2" || len(wire["pepPolicyRevision"]) == 0 {
					t.Error("real user decision lacks version/revision binding")
				}
				var audience string
				if err := json.Unmarshal(wire["providerAudience"], &audience); err != nil || audience != "https://grant.example.test/" {
					t.Errorf("real user decision audience=%q err=%v", audience, err)
				}
			}
		}
		for key, values := range res.Header {
			if !strings.EqualFold(key, "Content-Length") {
				w.Header()[key] = values
			}
		}
		w.WriteHeader(res.StatusCode)
		_, _ = w.Write(response)
	}), false)
	return relay
}

func (f *principalExposureSystem) startExposure(t *testing.T, binary, grantURL, digest string, legacy bool) string {
	t.Helper()
	address := func() string {
		t.Helper()
		ln, err := net.Listen("tcp", "127.0.0.1:0")
		if err != nil {
			t.Fatal(err)
		}
		addr := ln.Addr().String()
		if err := ln.Close(); err != nil {
			t.Fatal(err)
		}
		return addr
	}
	data, admin := address(), address()
	for data == admin {
		admin = address()
	}
	evidence := os.Getenv("GRANT_EXPOSURE_EVIDENCE")
	if evidence == "" {
		evidence = t.TempDir()
	} else if err := os.MkdirAll(evidence, 0700); err != nil {
		t.Fatal(err)
	}
	log, err := os.CreateTemp(evidence, "grant-principal-exposure-*.log")
	if err != nil {
		t.Fatal(err)
	}
	command := exec.CommandContext(f.ctx, binary)
	command.WaitDelay = 2 * time.Second
	// Isolate from existing Exposure settings; never pass sibling source paths.
	for _, entry := range os.Environ() {
		if !strings.HasPrefix(entry, "EXPOSURE_") {
			command.Env = append(command.Env, entry)
		}
	}
	settings := map[string]string{
		"PROXY_ENABLED": "true", "DATA_ADDR": data, "ADMIN_ADDR": admin,
		"GRANT_LEARNER_URL": grantURL, "PROVIDER_ORIGIN": f.upstream.URL,
		"PROVIDER_ID": "fixture-provider", "PEP_CLIENT_ID": "fixture-pep", "EXPECTED_CATALOG_DIGEST": digest,
		"GRANT_APP_AUTH_KEY": "fixture-app-key", "AUTHN_LIMITER_KEY": strings.Repeat("f", 40),
		"PROVIDER_AUTHORIZATION_MODE": "forward-user", "CREDENTIAL_MODE": "goauthy", "GOAUTHY_ISSUER": f.issuer.URL,
	}
	if legacy {
		settings["GOAUTHY_APPLICATION_ID"] = "unused-legacy-application"
	}
	for key, value := range settings {
		command.Env = append(command.Env, "EXPOSURE_"+key+"="+value)
	}
	command.Stdout, command.Stderr = log, log
	if err := command.Start(); err != nil {
		_ = log.Close()
		t.Logf("Exposure startup log retained: %s", log.Name())
		t.Fatal(err)
	}
	t.Logf("owned Exposure pid=%d log=%s", command.Process.Pid, log.Name())
	done := make(chan struct{})
	var waitErr error
	go func() { waitErr = command.Wait(); close(done) }()
	t.Cleanup(func() {
		select {
		case <-done:
		default:
			_ = command.Process.Signal(syscall.SIGTERM)
		}
		select {
		case <-done:
		case <-time.After(5 * time.Second):
			_ = command.Process.Kill()
			select {
			case <-done:
			case <-time.After(5 * time.Second):
				t.Errorf("owned Exposure pid=%d did not exit; log retained at %s", command.Process.Pid, log.Name())
			}
		}
		if err := log.Close(); err != nil {
			t.Errorf("close Exposure log: %v", err)
		}
		if t.Failed() {
			t.Logf("Exposure failure log retained: %s", log.Name())
		} else if err := os.Remove(log.Name()); err != nil {
			t.Errorf("remove Exposure log: %v", err)
		}
	})
	client := &http.Client{Timeout: time.Second}
	defer client.CloseIdleConnections()
	deadline := time.Now().Add(15 * time.Second)
	for time.Now().Before(deadline) {
		select {
		case <-f.ctx.Done():
			t.Fatalf("Exposure readiness canceled: %v; log=%s", f.ctx.Err(), log.Name())
		case <-done:
			t.Fatalf("Exposure exited before readiness: %v; log=%s", waitErr, log.Name())
		default:
		}
		req, err := http.NewRequestWithContext(f.ctx, http.MethodGet, "http://"+admin+"/readyz", nil)
		if err != nil {
			t.Fatal(err)
		}
		res, err := client.Do(req)
		if err == nil {
			_, _ = io.Copy(io.Discard, res.Body)
			_ = res.Body.Close()
			if res.StatusCode == 200 {
				return "http://" + data
			}
		}
		time.Sleep(30 * time.Millisecond)
	}
	t.Fatalf("Exposure readiness timeout; log=%s", log.Name())
	return ""
}

func principalExposureServer(t *testing.T, ctx context.Context, handler http.Handler, tls bool) *httptest.Server {
	t.Helper()
	server := httptest.NewUnstartedServer(handler)
	server.Config.BaseContext = func(net.Listener) context.Context { return ctx }
	server.Config.ReadHeaderTimeout = time.Second
	server.Config.ReadTimeout = 5 * time.Second
	server.Config.WriteTimeout = 5 * time.Second
	if tls {
		server.StartTLS()
	} else {
		server.Start()
	}
	t.Cleanup(func() {
		server.CloseClientConnections()
		server.Close()
	})
	return server
}

func principalExposureRequest(t *testing.T, ctx context.Context, target, token, org string) int {
	t.Helper()
	req, err := http.NewRequestWithContext(ctx, http.MethodGet, target, nil)
	if err != nil {
		t.Fatal(err)
	}
	req.Header.Set("Authorization", "Bearer "+token)
	req.Header.Set("X-Organization-ID", org)
	client := &http.Client{Timeout: 5 * time.Second, CheckRedirect: func(*http.Request, []*http.Request) error { return http.ErrUseLastResponse }}
	defer client.CloseIdleConnections()
	res, err := client.Do(req)
	if err != nil {
		t.Fatal(err)
	}
	defer res.Body.Close()
	_, err = io.Copy(io.Discard, io.LimitReader(res.Body, 8192))
	if err != nil {
		t.Fatal(err)
	}
	t.Logf("GET %s status=%d", target, res.StatusCode)
	return res.StatusCode
}
