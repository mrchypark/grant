package api

import (
	"context"
	"encoding/json"
	"github.com/Conalog/grant/internal/grant"
	"net/http"
	"net/http/httptest"
	"strings"
	"testing"
)

func TestOrglessHTTPOnboardingAndOwnOrganization(t *testing.T) {
	ctx := context.Background()
	idp := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		_ = json.NewEncoder(w).Encode(map[string]any{"token": "tok", "record": map[string]any{"id": "newuser12345678", "collectionId": "users", "collectionName": "users", "name": "New user"}})
	}))
	defer idp.Close()
	cfg := grant.Config{Auth: grant.AuthConfig{ProviderMode: "pocketbase", PocketBase: &grant.PocketBaseConfig{BaseURL: idp.URL, Collection: "users", ApplicationID: "app"}}, Database: grant.DatabaseConfig{DataDir: t.TempDir(), DefaultReadConsistency: grant.ReadStrong}}
	store, err := grant.OpenStore(ctx, cfg)
	if err != nil {
		t.Fatal(err)
	}
	defer store.Close()
	if err = store.EnsureBootstrapData(ctx); err != nil {
		t.Fatal(err)
	}
	svc := grant.NewService(cfg, store)
	if _, err = svc.UpsertServiceProviderAdmin(ctx, grant.ServiceProvider{ID: "grant-management", Name: "Management", Active: true}); err != nil {
		t.Fatal(err)
	}
	_, actions, err := grant.ManagementCatalog(1, nil)
	if err != nil {
		t.Fatal(err)
	}
	if _, err = svc.ReplaceProviderActionsAdmin(ctx, "grant-management", actions); err != nil {
		t.Fatal(err)
	}
	api := New(svc, false)
	call := func(method, path, body, org string, want int) *httptest.ResponseRecorder {
		t.Helper()
		r := httptest.NewRequest(method, path, strings.NewReader(body))
		r.Header.Set("Authorization", "Bearer tok")
		if org != "" {
			r.Header.Set("X-Organization-ID", org)
		}
		if body != "" {
			r.Header.Set("Content-Type", "application/json")
		}
		w := httptest.NewRecorder()
		api.ServeHTTP(w, r)
		if w.Code != want {
			t.Fatalf("%s %s=%d want%d: %s", method, path, w.Code, want, w.Body.String())
		}
		return w
	}
	call("POST", "/api/v1/me/materialize", "", "", 200)
	call("GET", "/api/v1/me", "", "", 200)
	if w := call("GET", "/api/v1/orgs", "", "", 200); strings.TrimSpace(w.Body.String()) != "[]" {
		t.Fatal("new identity gained organization", w.Body.String())
	}
	call("POST", "/api/v1/orgs", `{"id":"new-org","name":"Mine","ownerUserId":"conalog-grant"}`, "", 422)
	call("POST", "/api/v1/orgs", `{"id":"new-org","name":"Mine"}`, "", 201)
	call("POST", "/api/v1/orgs", `{"id":"new-org","name":"Overwrite"}`, "", 409)
	call("GET", "/api/v1/orgs/new-org", "", "new-org", 200)
	call("GET", "/api/v1/orgs/new-org", "", "", 400)
	call("POST", "/api/v1/orgs/new-org/groups", `{"id":"new-group","name":"Mine"}`, "new-org", 201)
	call("PUT", "/api/v1/orgs/new-org/members/conalog-grant", "", "new-org", 200)
	owner := call("GET", "/api/v1/orgs/new-org/owner", "", "new-org", 200)
	r := httptest.NewRequest("PUT", "/api/v1/orgs/new-org/owner", strings.NewReader(`{"ownerUserId":"conalog-grant"}`))
	r.Header.Set("Authorization", "Bearer tok")
	r.Header.Set("X-Organization-ID", "new-org")
	r.Header.Set("Content-Type", "application/json")
	r.Header.Set("If-Match", owner.Header().Get("ETag"))
	w := httptest.NewRecorder()
	api.ServeHTTP(w, r)
	if w.Code != 200 {
		t.Fatal(w.Code, w.Body.String())
	}
	call("POST", "/api/v1/orgs/new-org/groups", `{"id":"old-owner-group","name":"No"}`, "new-org", 403)
}
