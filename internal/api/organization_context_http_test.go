package api

import (
	"context"
	"crypto/sha256"
	"encoding/base64"
	"encoding/hex"
	"encoding/json"
	"io"
	"net/http"
	"net/http/httptest"
	"strings"
	"testing"
	"time"

	"github.com/mrchypark/grant/internal/grant"
)

// Exercise publication and PDP policy through Grant's real HTTP boundary.
func TestOrganizationContextHTTPPolicy(t *testing.T) {
	ctx := context.Background()
	hash := func(s string) string { sum := sha256.Sum256([]byte(s)); return "sha256:" + hex.EncodeToString(sum[:]) }
	payload, _ := json.Marshal(map[string]any{"id": "fixtureuser0001", "collectionId": "users", "type": "auth", "exp": time.Now().Add(time.Hour).Unix()})
	token := base64.RawURLEncoding.EncodeToString([]byte(`{"alg":"HS256","typ":"JWT"}`)) + "." + base64.RawURLEncoding.EncodeToString(payload) + ".Zml4dHVyZQ"
	idp := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		if r.Method != "POST" || r.URL.Path != "/api/collections/users/auth-refresh" || r.Header.Get("Authorization") != "Bearer "+token {
			http.Error(w, "invalid fixture token", 401)
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
	exec := func(sql string, args ...any) {
		t.Helper()
		if _, e := store.Exec(ctx, sql, args...); e != nil {
			t.Fatal(e)
		}
	}
	exec("INSERT INTO user_identities(id,user_id,provider,issuer,collection_id,external_user_id,last_seen_at,created_at,updated_at) VALUES('fixture-identity','fixture-user','pocketbase',?,'users','fixtureuser0001','now','now','now')", idp.URL)
	exec("INSERT INTO service_providers(id,name,active,created_at,updated_at) VALUES('inventory','Inventory',1,'now','now')")
	for _, code := range []string{"item.read", "app.only"} {
		exec("INSERT INTO actions(id,provider_id,code,name,risk_level,visibility_granting,grantable_to_applications,created_at,updated_at) VALUES(?,'inventory',?,?,'low',0,1,'now','now')", "inventory."+code, code, code)
	}
	app := grant.Application{ID: "fixture-app", OrganizationID: "fixture-user", Slug: "fixture-app", Name: "Fixture", ApplicationType: "confidential", AppType: "server", FirstParty: true, ConsentPolicy: "auto_approve", Active: true, ProvisioningStatus: "ready", CreatedByUserID: "fixture-user"}
	if err = store.InsertApplicationWithRedirectURI(ctx, app, nil); err != nil {
		t.Fatal(err)
	}
	exec("INSERT INTO application_actions(application_id,action_id) SELECT 'fixture-app',id FROM actions WHERE provider_id='inventory'")
	exec("INSERT INTO roles(id,organization_id,code,name,created_at,updated_at) VALUES('fixture-role',NULL,'fixture-role','Fixture','now','now')")
	exec("INSERT INTO role_actions(role_id,action_id) VALUES('fixture-role','inventory.item.read')")
	for _, org := range []string{"fixture-user"} {
		exec("INSERT INTO role_bindings(id,subject_type,subject_id,subject_organization_id,provider_id,resource_type,resource_id,role_id,state,created_by_user_id,created_at,updated_at) VALUES(?,'user','fixture-user',?,'inventory','item','allowed','fixture-role','active','fixture-user','now','now')", "fixture-binding-"+org, org)
	}
	svc := grant.NewService(cfg, store)
	server := httptest.NewServer(New(svc, false))
	defer server.Close()
	call := func(method, path string, value any, key string, want int) []byte {
		t.Helper()
		var raw []byte
		if value != nil {
			raw, err = json.Marshal(value)
			if err != nil {
				t.Fatal(err)
			}
		}
		req, e := http.NewRequest(method, server.URL+path, strings.NewReader(string(raw)))
		if e != nil {
			t.Fatal(e)
		}
		if key != "" {
			req.Header.Set("X-Grant-Master-Key", key)
		}
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
	call("PUT", "/ops/providers/inventory/enforcers/fixture-pep", grant.UpsertProviderPepClient{Lifecycle: "active"}, "fixture-master", 200)
	binding := grant.OrglessUserBinding{SubjectID: "fixture-user", ProviderID: "inventory", ResourceType: "item", ResourceID: "allowed", ActionID: "inventory.item.read", State: "active"}
	call("PUT", "/ops/orgless-bindings/fixture-orgless", binding, "", 401)
	call("PUT", "/ops/orgless-bindings/fixture-orgless", binding, "fixture-master", 200)
	stage := grant.StageHttpCapabilityGeneration{SchemaVersion: 1, Generation: 1, Capabilities: []grant.HttpCapability{}}
	for _, variant := range []struct{ id, policy, action string }{{"optional", "optional", "inventory.item.read"}, {"required", "required", "inventory.item.read"}, {"legacy", "", "inventory.item.read"}, {"app-only", "optional", "inventory.app.only"}} {
		action := variant.action
		c := grant.HttpCapability{CapabilityID: variant.id, Version: 1, Enabled: true, AccessMode: "authorized", Method: "GET", PathTemplate: "/" + variant.id + "/{itemId}", ActionID: &action, Resource: &grant.HttpCapabilityResource{ResourceType: "item", ID: grant.HttpResourceExtractor{Kind: "pathSegment", Name: "itemId", Format: "asciiToken"}}, Query: grant.HttpQueryPolicy{Mode: "none"}, Request: grant.HttpRequestPolicy{Body: "none", ForwardHeaders: []string{"x-organization-id"}, Authorization: "forward-user"}, Response: grant.HttpResponsePolicy{MaxBytes: 4096, ForwardHeaders: []string{"content-type"}, Redirects: "deny"}, OrganizationContext: variant.policy}
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
	call("POST", "/api/v1/admin/providers/inventory/http-capability-generations", stage, "fixture-master", 201)
	call("POST", "/api/v1/admin/providers/inventory/http-capability-active", grant.ActivateHttpCapabilityGeneration{TargetGeneration: 1, TargetCatalogDigest: stage.CatalogDigest}, "fixture-master", 200)
	published := call("GET", "/api/v1/admin/providers/inventory/http-capability-active", nil, "fixture-master", 200)
	var catalog grant.AdminHttpCapabilityCatalog
	if err = json.Unmarshal(published, &catalog); err != nil {
		t.Fatal(err)
	}
	if catalog.CatalogDigest != stage.CatalogDigest || catalog.Capabilities[0].OrganizationContext != "optional" || catalog.Capabilities[2].OrganizationContext != "" {
		t.Fatal("publication lost organization policy")
	}
	// Exercise the same HTTP PDP the external Exposure process will call.
	var optionalInput grant.HttpCapabilityDecisionInput
	for _, tc := range []struct {
		cap, resource, org string
		allow              bool
		status             int
	}{{"optional", "allowed", "", true, 200}, {"optional", "blocked", "", false, 200}, {"app-only", "allowed", "", false, 200}, {"optional", "allowed", "fixture-user", true, 200}, {"optional", "blocked", "fixture-user", false, 200}, {"required", "allowed", "", false, 400}, {"legacy", "allowed", "", false, 400}, {"legacy", "allowed", "fixture-user", true, 200}} {
		var c grant.HttpCapability
		for _, v := range stage.Capabilities {
			if v.CapabilityID == tc.cap {
				c = v
			}
		}
		name := "itemId"
		input := grant.HttpCapabilityDecisionInput{ProviderID: "inventory", PEPClientID: "fixture-pep", CatalogGeneration: 1, CatalogDigest: stage.CatalogDigest, CapabilityID: c.CapabilityID, CapabilityVersion: c.Version, ContractDigest: c.ContractDigest, Method: "GET", CanonicalPath: "/" + tc.cap + "/" + tc.resource, CanonicalQueryDigest: hash(""), ForwardedHeaderDigest: hash(""), BodyDigest: hash(""), ProviderCredentialDigest: grant.CredentialDigest("Bearer " + token), ProviderCredential: grant.HttpCapabilityCredentialEvidence{Subject: "pocketbase:" + idp.URL + "/users/fixtureuser0001", Audience: "fixture-app"}, ResourceEvidence: grant.HttpCapabilityResourceEvidence{Kind: "pathSegment", Name: &name, CanonicalValue: tc.resource}, ResourceType: "item", ResourceID: tc.resource, RequestBinding: strings.Repeat("a", 22)}
		if tc.cap == "optional" && tc.resource == "allowed" && tc.org == "" {
			optionalInput = input
		}
		raw, _ := json.Marshal(input)
		req, _ := http.NewRequest("POST", server.URL+"/api/v2/http-capabilities/decide", strings.NewReader(string(raw)))
		req.Header.Set("Content-Type", "application/json")
		req.Header.Set("Authorization", "Bearer "+token)
		req.Header.Set("X-Grant-App-Auth-Key", "fixture-app-key")
		if tc.org != "" {
			req.Header.Set("X-Organization-ID", tc.org)
		}
		resp, e := http.DefaultClient.Do(req)
		if e != nil {
			t.Fatal(e)
		}
		b, _ := io.ReadAll(resp.Body)
		resp.Body.Close()
		if resp.StatusCode != tc.status {
			t.Fatalf("%+v: %d %s", tc, resp.StatusCode, b)
		}
		if tc.status == 200 {
			var d grant.HttpCapabilityDecision
			if e = json.Unmarshal(b, &d); e != nil || d.Allowed != tc.allow {
				t.Fatalf("%+v: %s %v", tc, b, e)
			}
			if tc.org == "" && d.PrincipalDigest != grant.ExposurePrincipalDigest("fixture-user", "", "fixture-app") {
				t.Fatal("absent organization borrowed application ownership")
			}
		}
	}
	probe := func(input grant.HttpCapabilityDecisionInput, headers []string, bearer string, want int, allowed bool) {
		t.Helper()
		raw, _ := json.Marshal(input)
		req, _ := http.NewRequest("POST", server.URL+"/api/v2/http-capabilities/decide", strings.NewReader(string(raw)))
		req.Header.Set("Content-Type", "application/json")
		req.Header.Set("Authorization", bearer)
		req.Header.Set("X-Grant-App-Auth-Key", "fixture-app-key")
		for _, header := range headers {
			req.Header.Add("X-Organization-ID", header)
		}
		resp, e := http.DefaultClient.Do(req)
		if e != nil {
			t.Fatal(e)
		}
		b, e := io.ReadAll(resp.Body)
		resp.Body.Close()
		if e != nil || resp.StatusCode != want {
			t.Fatalf("probe headers=%q: %d %s %v", headers, resp.StatusCode, b, e)
		}
		if want == 200 {
			var d grant.HttpCapabilityDecision
			if e = json.Unmarshal(b, &d); e != nil || d.Allowed != allowed {
				t.Fatalf("probe: %s %v", b, e)
			}
		}
	}
	for _, headers := range [][]string{{""}, {"   "}, {"fixture-user", "fixture-user"}, {"fixture-user", "other"}} {
		probe(optionalInput, headers, "Bearer "+token, 400, false)
	}
	unknownTokenInput := optionalInput
	unknownTokenInput.ProviderCredentialDigest = grant.CredentialDigest("Bearer unknown-token")
	probe(unknownTokenInput, nil, "Bearer unknown-token", 401, false)
	tampered := optionalInput
	tampered.ContractDigest = stage.Capabilities[1].ContractDigest
	probe(tampered, nil, "Bearer "+token, 403, false)
	tampered = optionalInput
	tampered.CatalogDigest = hash("stale")
	probe(tampered, nil, "Bearer "+token, 403, false)
	tampered = optionalInput
	tampered.ProviderCredential.Audience = "other-app"
	probe(tampered, nil, "Bearer "+token, 403, false)
	binding.State = "disabled"
	call("PUT", "/ops/orgless-bindings/fixture-orgless", binding, "fixture-master", 200)
	probe(optionalInput, nil, "Bearer "+token, 200, false)
	// Tenant rights still work, proving orgless revocation does not borrow them.
	probe(optionalInput, []string{"fixture-user"}, "Bearer "+token, 200, true)
	binding.State = "active"
	call("PUT", "/ops/orgless-bindings/fixture-orgless", binding, "fixture-master", 200)
	probe(optionalInput, nil, "Bearer "+token, 200, true)
	for _, mutation := range []struct{ apply, restore string }{
		{"DELETE FROM application_actions WHERE application_id='fixture-app' AND action_id='inventory.item.read'", "INSERT INTO application_actions(application_id,action_id) VALUES('fixture-app','inventory.item.read')"},
		{"UPDATE actions SET grantable_to_applications=0 WHERE id='inventory.item.read'", "UPDATE actions SET grantable_to_applications=1 WHERE id='inventory.item.read'"},
	} {
		exec(mutation.apply)
		probe(optionalInput, nil, "Bearer "+token, 200, false)
		exec(mutation.restore)
	}
	// The operator endpoint never accepts tenant authority or overwrites tenant rows.
	call("PUT", "/ops/orgless-bindings/fixture-binding-fixture-user", binding, "fixture-master", 409)
	raw, _ := json.Marshal(binding)
	req, _ := http.NewRequest("PUT", server.URL+"/ops/orgless-bindings/tenant-forgery", strings.NewReader(string(raw)))
	req.Header.Set("Content-Type", "application/json")
	req.Header.Set("Authorization", "Bearer "+token)
	req.Header.Set("X-Organization-ID", "fixture-user")
	resp, e := http.DefaultClient.Do(req)
	if e != nil {
		t.Fatal(e)
	}
	resp.Body.Close()
	if resp.StatusCode != 401 {
		t.Fatal("tenant authenticated operator endpoint", resp.StatusCode)
	}
	if _, e = svc.CreateRoleBindingAuthorized(ctx, "fixture-user", "fixture-user", grant.CreateRoleBindingInput{ID: "tenant-forged", SubjectType: "user", SubjectID: "fixture-user", SubjectOrganizationID: "", ProviderID: "inventory", ResourceType: "item", ResourceID: "blocked", RoleID: "fixture-role", State: "active"}); grant.ErrorStatus(e) != 403 {
		t.Fatal("tenant minted orgless binding", e)
	}
}
