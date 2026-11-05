package api

import (
	"context"
	"encoding/json"
	"net/http"
	"net/http/httptest"
	"strings"
	"testing"

	"github.com/mrchypark/grant/internal/grant"
)

func lifecycleCall(t *testing.T, s *Server, method, path, body, org string, master bool, want int) *httptest.ResponseRecorder {
	t.Helper()
	r := httptest.NewRequest(method, path, strings.NewReader(body))
	r.Header.Set("Content-Type", "application/json")
	if method == "PUT" && strings.Contains(path, "/enforcers/") {
		r.Header.Set("If-None-Match", "*")
	}
	r.Header.Set("Authorization", "Bearer tok")
	r.Header.Set("X-Organization-ID", org)
	if master {
		r.Header.Set("X-Grant-Master-Key", "master-secret")
	}
	w := httptest.NewRecorder()
	s.ServeHTTP(w, r)
	if w.Code != want {
		t.Fatalf("%s %s got %d want %d: %s", method, path, w.Code, want, w.Body.String())
	}
	return w
}

func TestLifecycleProviderHTTPContracts(t *testing.T) {
	s := managementTestServer(t)
	lifecycleCall(t, s, "POST", "/api/v1/admin/providers", `{"id":"lifecycle-provider","name":"Original","description":"preserve","active":true}`, "mgmt-org", true, 201)
	lifecycleCall(t, s, "PUT", "/api/v1/admin/orgs/mgmt-org/apps/app-1/internal-admission", "", "mgmt-org", true, 404)
	lifecycleCall(t, s, "POST", "/api/v1/admin/providers", `{"id":"lifecycle-provider","name":"Overwrite","active":false}`, "mgmt-org", true, 409)
	// Existing create permission cannot authorize the new update action.
	lifecycleCall(t, s, "PATCH", "/api/v1/admin/providers/lifecycle-provider", `{"name":"Denied"}`, "mgmt-org", false, 403)
	for _, body := range []string{`{"name":null,"active":false}`, `{"name":"valid","active":null}`, `{"id":"other","name":"valid"}`, `{"name":"a","name":"b"}`, `{"Name":null,"description":"x"}`, `{"name":"a","Name":"b"}`} {
		lifecycleCall(t, s, "PATCH", "/api/v1/admin/providers/lifecycle-provider", body, "mgmt-org", true, 422)
	}
	w := lifecycleCall(t, s, "PATCH", "/api/v1/admin/providers/lifecycle-provider", `{"name":"Renamed"}`, "mgmt-org", true, 200)
	var p grant.ServiceProvider
	if err := json.Unmarshal(w.Body.Bytes(), &p); err != nil {
		t.Fatal(err)
	}
	if p.Name != "Renamed" || !p.Active || p.Description == nil || *p.Description != "preserve" {
		t.Fatalf("omitted fields changed: %+v", p)
	}
	w = lifecycleCall(t, s, "PATCH", "/api/v1/admin/providers/lifecycle-provider", `{"description":null,"active":false}`, "mgmt-org", true, 200)
	if err := json.Unmarshal(w.Body.Bytes(), &p); err != nil {
		t.Fatal(err)
	}
	if p.Active {
		t.Fatal("explicit false ignored")
	}
	var raw map[string]any
	if err := json.Unmarshal(w.Body.Bytes(), &raw); err != nil {
		t.Fatal(err)
	}
	if raw["description"] != nil {
		t.Fatal("explicit null ignored", raw)
	}
	lifecycleCall(t, s, "POST", "/api/v1/admin/orgs/mgmt-org/apps/app-1/provisioning/verify", "", "mgmt-org", true, 404)
	lifecycleCall(t, s, "PUT", "/api/v1/admin/orgs/mgmt-org/apps/app-1/identity-binding", `{"clientId":"external"}`, "mgmt-org", false, 403)
}

func TestPepRegistrationHasNoApplicationField(t *testing.T) {
	s, store := managementTestServerWithStore(t)
	lifecycleCall(t, s, "PUT", "/ops/providers/grant-management/enforcers/lifecycle-pep", `{"lifecycle":"active","validUntil":null}`, "", true, 200)
	if _, err := store.GetApplicationWithConsistency(context.Background(), "lifecycle-pep", grant.ReadStrong); err == nil {
		t.Fatal("PEP registration created or required an application")
	}
	lifecycleCall(t, s, "PUT", "/ops/providers/grant-management/enforcers/legacy-pep", `{"applicationId":"app-1","lifecycle":"active"}`, "", true, 422)
}

func TestLifecycleOrganizationAndMetadataHTTP(t *testing.T) {
	s := managementTestServer(t)
	lifecycleCall(t, s, "POST", "/api/v1/admin/orgs", `{"id":"created-org","name":"Created","ownerUserId":"owner"}`, "mgmt-org", false, 403)
	lifecycleCall(t, s, "POST", "/api/v1/admin/orgs", `{"id":"created-org","name":"Created","ownerUserId":"owner"}`, "mgmt-org", true, 201)
	lifecycleCall(t, s, "POST", "/api/v1/admin/orgs", `{"id":"created-org","name":"Overwrite","ownerUserId":"owner"}`, "mgmt-org", true, 409)
	lifecycleCall(t, s, "GET", "/api/v1/orgs/created-org", "", "created-org", false, 200)
	lifecycleCall(t, s, "GET", "/api/v1/orgs/created-org", "", "owner", false, 403)
	lifecycleCall(t, s, "PATCH", "/api/v1/orgs/created-org", `{"name":"Renamed"}`, "created-org", false, 200)
	lifecycleCall(t, s, "PATCH", "/api/v1/orgs/created-org", `{"name":null}`, "created-org", false, 422)
	w := lifecycleCall(t, s, "GET", "/api/v1/orgs", "", "created-org", false, 200)
	var orgs []grant.Organization
	if err := json.Unmarshal(w.Body.Bytes(), &orgs); err != nil || len(orgs) != 3 {
		t.Fatal(w.Body.String(), err)
	}
	r := httptest.NewRequest("GET", "/api/v1/orgs", nil)
	r.Header.Set("Authorization", "Bearer tok")
	w = httptest.NewRecorder()
	s.ServeHTTP(w, r)
	if w.Code != 200 {
		t.Fatalf("list without selected organization: %d %s", w.Code, w.Body.String())
	}
	lifecycleCall(t, s, "POST", "/api/v1/orgs/created-org/groups", `{"id":"lifecycle-group","name":"Group","description":"keep"}`, "created-org", false, 201)
	w = lifecycleCall(t, s, "PATCH", "/api/v1/orgs/created-org/groups/lifecycle-group", `{"name":"Renamed"}`, "created-org", false, 200)
	var g grant.Group
	if err := json.Unmarshal(w.Body.Bytes(), &g); err != nil || g.Description == nil || *g.Description != "keep" {
		t.Fatal(w.Body.String(), err)
	}
	lifecycleCall(t, s, "PATCH", "/api/v1/orgs/created-org/groups/lifecycle-group", `{"organizationId":"owner","name":"bad"}`, "created-org", false, 422)
	lifecycleCall(t, s, "GET", "/api/v1/orgs/created-org/members", "", "created-org", false, 200)
	lifecycleCall(t, s, "DELETE", "/api/v1/orgs/created-org/members/owner", "", "created-org", false, 409)
	lifecycleCall(t, s, "POST", "/api/v1/orgs/created-org/roles", `{"id":"lifecycle-role","code":"role","name":"Role"}`, "created-org", false, 201)
	lifecycleCall(t, s, "PATCH", "/api/v1/orgs/created-org/roles/lifecycle-role", `{"name":"Renamed"}`, "created-org", false, 200)
	lifecycleCall(t, s, "DELETE", "/api/v1/orgs/created-org/roles/lifecycle-role", "", "created-org", false, 204)
}

func TestLifecycleApplicationRedirectPatchHTTP(t *testing.T) {
	s, store := managementTestServerWithStore(t)
	ctx := context.Background()
	now := "2025-01-01T00:00:00Z"
	if _, err := store.Exec(ctx, "INSERT INTO applications(id,organization_id,slug,name,application_type,app_type,first_party,consent_policy,active,provisioning_status,created_by_user_id,created_at,updated_at) VALUES('patch-app','owner','patch-app','Original','confidential','web',0,'explicit',1,'unprovisioned','owner',?,?)", now, now); err != nil {
		t.Fatal(err)
	}
	if _, err := store.Exec(ctx, "INSERT INTO applications(id,organization_id,slug,name,application_type,app_type,first_party,consent_policy,active,provisioning_status,created_by_user_id,created_at,updated_at) VALUES('patch-ready','owner','patch-ready','Ready','confidential','web',0,'explicit',1,'ready','owner',?,?)", now, now); err != nil {
		t.Fatal(err)
	}
	path := "/api/v1/orgs/owner/apps/patch-app"
	for _, body := range []string{
		`{"redirectUri":null}`,
		`{"redirectUri":""}`,
		`{"redirectUri":"https://operable.dev.conalog.com/callback?unsafe=1"}`,
		`{"applicationType":"public"}`,
		`{"active":false}`,
		`{"clientId":"attacker"}`,
		`{"description":"not part of this patch contract"}`,
		`{"redirectUri":"https://one.example/cb","redirectUri":"https://two.example/cb"}`,
	} {
		want := 422
		if strings.Contains(body, `"redirectUri":""`) || strings.Contains(body, "unsafe=1") {
			want = 400
		}
		lifecycleCall(t, s, "PATCH", path, body, "owner", false, want)
	}
	callback := "https://operable.dev.conalog.com/oauth/callback"
	w := lifecycleCall(t, s, "PATCH", path, `{"redirectUri":"https://operable.dev.conalog.com/oauth/callback"}`, "owner", false, 200)
	var patched grant.Application
	if err := json.Unmarshal(w.Body.Bytes(), &patched); err != nil || patched.Name != "Original" || patched.ProvisioningStatus != "unprovisioned" {
		t.Fatalf("unexpected patch result: %+v %v %s", patched, err, w.Body.String())
	}

	// Existing metadata PATCH behavior stays partial: changing the name must not
	// remove the callback installed above.
	lifecycleCall(t, s, "PATCH", path, `{"name":"Renamed"}`, "owner", false, 200)
	w = lifecycleCall(t, s, "GET", path, "", "owner", false, 200)
	var detail struct {
		grant.Application
		RedirectURI *string `json:"redirectUri"`
	}
	if err := json.Unmarshal(w.Body.Bytes(), &detail); err != nil || detail.Name != "Renamed" || detail.RedirectURI == nil || *detail.RedirectURI != callback {
		t.Fatalf("omitted redirectUri changed: %+v %v %s", patched, err, w.Body.String())
	}

	lifecycleCall(t, s, "PATCH", path, `{"redirectUri":"https://operable.dev.conalog.com/another"}`, "mgmt-org", false, 403)
	// Ready/bound applications cannot have their callback changed through this route.
	lifecycleCall(t, s, "PATCH", "/api/v1/orgs/owner/apps/patch-ready", `{"redirectUri":"https://operable.dev.conalog.com/oauth/callback"}`, "owner", false, 409)
}

func TestLifecycleHTTPMemberRemovalRevokesAccess(t *testing.T) {
	ctx := context.Background()
	up := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		subject := "ownerexternal"
		if strings.Contains(r.Header.Get("Authorization"), "member") {
			subject = "memberexternal"
		}
		_ = json.NewEncoder(w).Encode(map[string]any{"token": "tok", "record": map[string]any{"id": subject, "collectionId": "users", "collectionName": "users", "verified": true, "name": subject}})
	}))
	defer up.Close()
	cfg := grant.Config{Security: grant.SecurityConfig{BootstrapUserID: "owner", MasterKey: "master-secret"}, Auth: grant.AuthConfig{ProviderMode: "pocketbase", PocketBase: &grant.PocketBaseConfig{BaseURL: up.URL, Collection: "users"}}, Database: grant.DatabaseConfig{DataDir: t.TempDir(), DefaultReadConsistency: grant.ReadStrong}}
	store, err := grant.OpenStore(ctx, cfg)
	if err != nil {
		t.Fatal(err)
	}
	defer store.Close()
	if err = store.EnsureBootstrapData(ctx); err != nil {
		t.Fatal(err)
	}
	if _, err = store.Exec(ctx, "INSERT INTO users(id,display_name,active,created_at) VALUES('member','Member',1,'now')"); err != nil {
		t.Fatal(err)
	}
	for _, user := range []string{"owner", "member"} {
		if _, err = store.Exec(ctx, "INSERT INTO user_identities(id,user_id,provider,issuer,collection_id,external_user_id,last_seen_at,created_at,updated_at) VALUES(?,?,'pocketbase',?,'users',?,'now','now','now')", user, user, up.URL, user+"external"); err != nil {
			t.Fatal(err)
		}
	}
	svc := grant.NewService(cfg, store)
	s := New(svc, false)
	lifecycleCall(t, s, "PUT", "/api/v1/orgs/owner/members/member", "", "owner", false, 200)
	lifecycleCall(t, s, "DELETE", "/api/v1/orgs/owner/members/member", "", "owner", false, 204)
	r := httptest.NewRequest("GET", "/api/v1/orgs/owner/members", nil)
	r.Header.Set("Authorization", "Bearer member")
	r.Header.Set("X-Organization-ID", "owner")
	w := httptest.NewRecorder()
	s.ServeHTTP(w, r)
	if w.Code != 403 {
		t.Fatalf("removed member still authenticates: %d %s", w.Code, w.Body.String())
	}
}
