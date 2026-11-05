package grant

import (
	"context"
	"encoding/json"
	"fmt"
	"net/http"
	"net/http/httptest"
	"strings"
	"sync/atomic"
	"testing"
	"time"
)

type principalHTTPFixture struct {
	svc    *Service
	stage  StageHttpCapabilityGeneration
	caps   map[string]HttpCapability
	issuer string
	calls  atomic.Int64
}

func newPrincipalHTTPFixture(t *testing.T) *principalHTTPFixture {
	t.Helper()
	f := &principalHTTPFixture{}
	server := httptest.NewTLSServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		switch r.URL.Path {
		case "/oidc/introspect":
			f.calls.Add(1)
			id, secret, ok := r.BasicAuth()
			if !ok || id != "introspector" || secret != "secret" || r.Method != "POST" {
				t.Error("invalid introspection authentication")
			}
			if err := r.ParseForm(); err != nil || r.Form.Get("token") != "token" {
				t.Error("invalid introspection token")
			}
			fmt.Fprintf(w, `{"active":true,"sub":"principal-user","client_id":"login","exp":%d,"aud":["https://grant.example.test/"],"scope":"openid grant"}`, time.Now().Add(time.Minute).Unix())
		case "/oidc/userinfo":
			if r.Header.Get("Authorization") != "Bearer token" {
				t.Error("invalid UserInfo authentication")
			}
			fmt.Fprint(w, `{"sub":"principal-user"}`)
		default:
			w.WriteHeader(404)
		}
	}))
	t.Cleanup(server.Close)
	f.issuer = server.URL
	cfg := Config{Database: DatabaseConfig{DataDir: t.TempDir()}, Security: SecurityConfig{HTTPCapabilityPEPClientID: "batch-pep"}, Auth: AuthConfig{ProviderMode: "goauthy", Goauthy: &GoauthyConfig{Issuer: server.URL, IntrospectionClientID: "introspector", IntrospectionClientSecret: "secret", ExpectedClientID: "login", Resource: "https://grant.example.test/", ApplicationID: "absent-app", RequiredScopes: []string{"openid", "grant"}}}}
	ctx := context.Background()
	store, err := OpenStore(ctx, cfg)
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() { store.Close() })
	if err = store.EnsureBootstrapData(ctx); err != nil {
		t.Fatal(err)
	}
	f.svc = NewService(cfg, store)
	f.svc.client = server.Client()
	f.exec(t, "UPDATE users SET email='principal@example.test' WHERE id='conalog-grant'")
	if err = f.svc.LinkGoauthyIdentityAdmin(ctx, GoauthyIdentityLinkInput{ExistingUserID: "conalog-grant", Issuer: server.URL, Subject: "principal-user", ExpectedEmail: "principal@example.test"}); err != nil {
		t.Fatal(err)
	}
	if _, err = f.svc.UpsertServiceProviderAdmin(ctx, ServiceProvider{ID: "patch-data", Name: "Patch Data", Active: true}); err != nil {
		t.Fatal(err)
	}
	stage, actions, err := PatchDataCatalog(1, nil)
	if err != nil {
		t.Fatal(err)
	}
	f.stage, f.caps = stage, map[string]HttpCapability{}
	for _, c := range stage.Capabilities {
		f.caps[c.CapabilityID] = c
	}
	if _, err = f.svc.ReplaceProviderActionsAdmin(ctx, "patch-data", actions); err != nil {
		t.Fatal(err)
	}
	mode := "user"
	if _, err = store.UpsertProviderPepClient(ctx, "patch-data", "batch-pep", "conalog-grant", UpsertProviderPepClient{Lifecycle: "active", AuthorizationMode: &mode}); err != nil {
		t.Fatal(err)
	}
	if _, err = store.StageHttpCapabilityGeneration(ctx, "patch-data", stage); err != nil {
		t.Fatal(err)
	}
	if _, err = store.ActivateHttpCapabilityGeneration(ctx, "patch-data", "conalog-grant", ActivateHttpCapabilityGeneration{TargetGeneration: 1, TargetCatalogDigest: stage.CatalogDigest}); err != nil {
		t.Fatal(err)
	}
	f.exec(t, "INSERT INTO role_actions(role_id,action_id) SELECT 'role-system-bootstrap-owner',id FROM actions WHERE provider_id='patch-data'")
	rows, err := store.Query(ctx, ReadStrong, "SELECT id FROM applications")
	if err != nil || len(rows) != 0 {
		t.Fatalf("fixture requires zero applications: %v %v", rows, err)
	}
	return f
}

func (f *principalHTTPFixture) exec(t *testing.T, sql string, args ...any) {
	t.Helper()
	if _, err := f.svc.store.Exec(context.Background(), sql, args...); err != nil {
		t.Fatal(err)
	}
}
func (f *principalHTTPFixture) bind(t *testing.T, id, kind, resource string) {
	t.Helper()
	f.exec(t, "INSERT INTO role_bindings(id,subject_type,subject_id,subject_organization_id,provider_id,resource_type,resource_id,role_id,state,created_by_user_id,created_at,updated_at) VALUES(?,'user','conalog-grant','conalog-grant','patch-data',?,?,'role-system-bootstrap-owner','active','conalog-grant','now','now')", id, kind, resource)
}
func (f *principalHTTPFixture) wire(in HttpCapabilityDecisionInput) HttpCapabilityDecisionInput {
	in.WireVersion, in.PEPPolicyRevision = 2, 1
	in.ProviderCredentialDigest = CredentialDigest("Bearer token")
	in.ProviderCredential = HttpCapabilityCredentialEvidence{Subject: goauthySubject(f.issuer, "principal-user"), Audience: "https://grant.example.test/"}
	return in
}
func (f *principalHTTPFixture) input(t *testing.T, id, resource string) HttpCapabilityDecisionInput {
	t.Helper()
	cap := f.caps[id]
	return f.wire(batchChildInput(batchEvidenceInput(t, f.stage, cap, nil), batchEvidenceChild(cap, 0, resource)))
}
func (f *principalHTTPFixture) decide(t *testing.T, in HttpCapabilityDecisionInput, want bool) HttpCapabilityDecision {
	t.Helper()
	before := f.calls.Load()
	out, err := f.svc.DecideHttpCapability(context.Background(), "Bearer token", "conalog-grant", in)
	if err != nil || out.Allowed != want {
		t.Fatalf("allowed=%v decision=%+v err=%v", want, out, err)
	}
	if f.calls.Load() != before+1 {
		t.Fatal("external authentication was repeated or cached")
	}
	if out.WireVersion != 2 || out.PEPPolicyRevision != in.PEPPolicyRevision || out.ProviderAudience != in.ProviderCredential.Audience || out.PrincipalDigest != ExposurePrincipalDigest("conalog-grant", "conalog-grant", "") {
		t.Fatalf("incorrect user binding: %+v", out)
	}
	return out
}

func TestPrincipalHTTPCapabilityWithoutApplicationsAndRevocation(t *testing.T) {
	f := newPrincipalHTTPFixture(t)
	in := f.input(t, "files.update.0", "aaaaaaaaaaaaaaa")
	f.decide(t, in, false)
	f.bind(t, "file", "files", in.ResourceID)
	f.decide(t, in, true)
	f.exec(t, "UPDATE role_bindings SET state='revoked' WHERE id='file'")
	f.decide(t, in, false)
	f.exec(t, "UPDATE role_bindings SET state='active' WHERE id='file'")
	f.exec(t, "UPDATE organization_memberships SET status='disabled' WHERE user_id='conalog-grant'")
	if _, err := f.svc.DecideHttpCapability(context.Background(), "Bearer token", "conalog-grant", in); ErrorStatus(err) != 403 {
		t.Fatal("revoked membership allowed", err)
	}
}

func TestPrincipalHTTPRejectsHintsRevisionsAndLegacy(t *testing.T) {
	f := newPrincipalHTTPFixture(t)
	in := f.input(t, "files.update.0", "aaaaaaaaaaaaaaa")
	for name, change := range map[string]func(*HttpCapabilityDecisionInput){
		"subject":          func(v *HttpCapabilityDecisionInput) { v.ProviderCredential.Subject = "forged" },
		"audience":         func(v *HttpCapabilityDecisionInput) { v.ProviderCredential.Audience = "absent-app" },
		"revision":         func(v *HttpCapabilityDecisionInput) { v.PEPPolicyRevision++ },
		"missing-revision": func(v *HttpCapabilityDecisionInput) { v.PEPPolicyRevision = 0 },
		"legacy":           func(v *HttpCapabilityDecisionInput) { v.WireVersion, v.PEPPolicyRevision = 0, 0 },
		"version":          func(v *HttpCapabilityDecisionInput) { v.WireVersion = 3 },
		"catalog":          func(v *HttpCapabilityDecisionInput) { v.CatalogDigest = sha256Text("stale") },
	} {
		t.Run(name, func(t *testing.T) {
			v := in
			change(&v)
			if _, err := f.svc.DecideHttpCapability(context.Background(), "Bearer token", "conalog-grant", v); err == nil {
				t.Fatal("invalid evidence accepted")
			}
		})
	}
	if _, err := f.svc.DecideHttpCapability(context.Background(), "Bearer token", "other-org", in); ErrorStatus(err) != 403 {
		t.Fatal("wrong organization accepted", err)
	}
	var injected HttpCapabilityDecisionInput
	raw, _ := json.Marshal(in)
	raw = append(raw[:len(raw)-1], []byte(`,"authorizationMode":"application"}`)...)
	decoder := json.NewDecoder(strings.NewReader(string(raw)))
	decoder.DisallowUnknownFields()
	if err := decoder.Decode(&injected); err == nil {
		t.Fatal("mode injection accepted")
	}
	catalog, err := f.svc.ActiveHttpCapabilityCatalog(context.Background(), "patch-data")
	if err != nil || catalog.AuthorizationMode != "user" || catalog.PEPPolicyRevision != 1 {
		t.Fatal(catalog, err)
	}
	mode := "application"
	if _, err = f.svc.store.UpsertProviderPepClient(context.Background(), "patch-data", "batch-pep", "conalog-grant", UpsertProviderPepClient{Lifecycle: "active", AuthorizationMode: &mode}); err != nil {
		t.Fatal(err)
	}
	if _, err = f.svc.DecideHttpCapability(context.Background(), "Bearer token", "conalog-grant", in); ErrorStatus(err) != 403 {
		t.Fatal("stale policy accepted after mode switch", err)
	}
	catalog, err = f.svc.ActiveHttpCapabilityCatalog(context.Background(), "patch-data")
	raw, _ = json.Marshal(catalog)
	if err != nil || strings.Contains(string(raw), "authorizationMode") || strings.Contains(string(raw), "pepPolicyRevision") {
		t.Fatal("legacy catalog changed", string(raw), err)
	}
	in.WireVersion, in.PEPPolicyRevision = 0, 0
	if _, err = f.svc.DecideHttpCapability(context.Background(), "Bearer token", "conalog-grant", in); ErrorStatus(err) != 403 {
		t.Fatal("legacy skipped application admission", err)
	}
}

func TestPrincipalHTTPBatchAndResponseRemainUserScoped(t *testing.T) {
	f := newPrincipalHTTPFixture(t)
	root := f.caps["patch-data.batch"]
	in := f.wire(batchEvidenceInput(t, f.stage, root, []HttpBatchChild{batchEvidenceChild(f.caps["files.update.0"], 0, "aaaaaaaaaaaaaaa")}))
	f.bind(t, "batch", "batch", "patch-data")
	f.decide(t, in, false)
	f.bind(t, "file", "files", "aaaaaaaaaaaaaaa")
	f.decide(t, in, true)
	f.exec(t, "UPDATE role_bindings SET state='revoked' WHERE id='file'")
	f.decide(t, in, false)
	_, _, response := responseFixture(t)
	response = f.wire(response)
	f.bind(t, "response-root", "permissions", "ccccccccccccccc")
	f.decide(t, response, false)
	f.bind(t, "related", "plant", "ddddddddddddddd")
	f.decide(t, response, true)
	f.exec(t, "UPDATE role_bindings SET state='revoked' WHERE id='related'")
	f.decide(t, response, false)
	list := f.input(t, "permissions.list.0", "")
	out := f.decide(t, list, true)
	if out.RecordScope == nil || len(out.RecordScope.IDs) != 1 || out.RecordScope.IDs[0] != "ccccccccccccccc" {
		t.Fatal("list lost exact RBAC scope", out)
	}
	f.exec(t, "UPDATE role_bindings SET state='revoked' WHERE id='response-root'")
	out = f.decide(t, list, true)
	if out.RecordScope == nil || len(out.RecordScope.IDs) != 0 {
		t.Fatal("list retained revoked scope", out)
	}
	identity, err := f.svc.externalIdentity(context.Background(), "Bearer token")
	if err != nil {
		t.Fatal(err)
	}
	ctx := context.WithValue(context.Background(), replicaIdentityKey{}, identity)
	principal, err := f.svc.resolveLocalUserAuthorizationContext(ctx, identity, "conalog-grant")
	if err != nil {
		t.Fatal(err)
	}
	if _, err = f.svc.store.UpsertProviderPepClient(ctx, "patch-data", "batch-pep", "conalog-grant", UpsertProviderPepClient{Lifecycle: "active"}); err != nil {
		t.Fatal(err)
	}
	if _, _, err = f.svc.batchDecision(ctx, "Bearer token", "conalog-grant", in, principal); ErrorStatus(err) != 403 {
		t.Fatal("stale batch follow-up accepted", err)
	}
	if _, _, err = f.svc.responseDecision(ctx, "Bearer token", "conalog-grant", response, principal); ErrorStatus(err) != 403 {
		t.Fatal("stale response follow-up accepted", err)
	}
	f.exec(t, "UPDATE organization_memberships SET status='disabled' WHERE user_id='conalog-grant'")
	empty := in
	empty.PEPPolicyRevision = 2
	empty.BatchChildren = nil
	empty.BatchDigest, _ = HttpBatchChildrenDigest(nil)
	if _, err := f.svc.DecideHttpCapability(context.Background(), "Bearer token", "conalog-grant", empty); ErrorStatus(err) != 403 {
		t.Fatal("empty batch bypassed membership", err)
	}
}

func TestPrincipalHTTPRemoteLegacyApplicationCompatibility(t *testing.T) {
	for _, mode := range []string{"remote", "split"} {
		t.Run(mode, func(t *testing.T) {
			f := newPrincipalHTTPFixture(t)
			ctx := context.Background()
			secret := strings.Repeat("r", 40)
			f.svc.store.cfg.Database.RemoteAPISecret = secret
			server := httptest.NewServer(f.svc.store.RuntimeHandler())
			t.Cleanup(server.Close)
			cfg := f.svc.cfg
			cfg.Database.RuntimeMode = mode
			cfg.Database.LocalNodeRole = "voter"
			cfg.Database.RemoteNodes = []string{server.URL}
			cfg.Database.RemoteAPISecret = secret
			remote := NewService(cfg, &Store{cfg: cfg})
			remote.client = f.svc.client
			in := f.input(t, "files.update.0", "aaaaaaaaaaaaaaa")
			if catalog, err := remote.ActiveHttpCapabilityCatalog(ctx, "patch-data"); ErrorStatus(err) != 503 || catalog.AuthorizationMode != "" || len(catalog.Capabilities) != 0 {
				t.Fatalf("remote user catalog delivered: %+v %v", catalog, err)
			}
			if decision, err := remote.DecideHttpCapability(ctx, "Bearer token", "conalog-grant", in); ErrorStatus(err) != 503 || decision.Allowed {
				t.Fatalf("remote user decision accepted: %+v %v", decision, err)
			}
			application := "application"
			if _, err := f.svc.store.UpsertProviderPepClient(ctx, "patch-data", "batch-pep", "conalog-grant", UpsertProviderPepClient{Lifecycle: "active", AuthorizationMode: &application}); err != nil {
				t.Fatal(err)
			}
			app := Application{ID: "absent-app", OrganizationID: "conalog-grant", Slug: "remote-app", Name: "Remote", ApplicationType: "confidential", AppType: "server", FirstParty: true, ConsentPolicy: "auto_approve", Active: true, ProvisioningStatus: "ready", CreatedByUserID: "conalog-grant"}
			if err := f.svc.store.InsertApplicationWithRedirectURI(ctx, app, nil); err != nil {
				t.Fatal(err)
			}
			f.exec(t, "INSERT INTO application_actions(application_id,action_id) VALUES(?,?)", app.ID, *f.caps[in.CapabilityID].ActionID)
			f.bind(t, "remote-file", "files", in.ResourceID)
			catalog, err := remote.ActiveHttpCapabilityCatalog(ctx, "patch-data")
			if err != nil || catalog.Generation != f.stage.Generation || catalog.CatalogDigest != f.stage.CatalogDigest || len(catalog.Capabilities) == 0 || catalog.AuthorizationMode != "" || catalog.PEPPolicyRevision != 0 {
				t.Fatalf("legacy remote catalog failed: %+v %v", catalog, err)
			}
			in.PEPPolicyRevision = 2
			if decision, err := remote.DecideHttpCapability(ctx, "Bearer token", "conalog-grant", in); ErrorStatus(err) != 503 || decision.Allowed {
				t.Fatalf("remote versioned decision accepted: %+v %v", decision, err)
			}
			in.WireVersion, in.PEPPolicyRevision = 0, 0
			in.ProviderCredential.Audience = app.ID
			decision, err := remote.DecideHttpCapability(ctx, "Bearer token", "conalog-grant", in)
			if err != nil || !decision.Allowed || decision.WireVersion != 0 || decision.PEPPolicyRevision != 0 || decision.PrincipalDigest != ExposurePrincipalDigest("conalog-grant", "conalog-grant", app.ID) {
				t.Fatalf("legacy remote application decision failed: %+v %v", decision, err)
			}
			f.exec(t, "DELETE FROM application_actions WHERE application_id=?", app.ID)
			decision, err = remote.DecideHttpCapability(ctx, "Bearer token", "conalog-grant", in)
			if err != nil || decision.Allowed {
				t.Fatalf("legacy remote admission bypassed: %+v %v", decision, err)
			}
			user := "user"
			if _, err = f.svc.store.UpsertProviderPepClient(ctx, "patch-data", "batch-pep", "conalog-grant", UpsertProviderPepClient{Lifecycle: "active", AuthorizationMode: &user}); err != nil {
				t.Fatal(err)
			}
			if _, err = remote.ActiveHttpCapabilityCatalog(ctx, "patch-data"); ErrorStatus(err) != 503 {
				t.Fatal("remote catalog accepted mode switch", err)
			}
			if decision, err = remote.DecideHttpCapability(ctx, "Bearer token", "conalog-grant", in); ErrorStatus(err) != 503 || decision.Allowed {
				t.Fatalf("legacy request accepted user mode: %+v %v", decision, err)
			}
		})
	}
}
