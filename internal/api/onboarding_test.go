package api

import (
	"context"
	"encoding/json"
	"github.com/mrchypark/grant/internal/grant"
	"net/http"
	"net/http/httptest"
	"strings"
	"testing"
)

func TestOrglessHTTPOnboardingAndOwnOrganization(t *testing.T) {
	ctx := context.Background()
	idp := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		if r.Header.Get("Authorization") != "Bearer tok" {
			http.Error(w, "unauthorized", http.StatusUnauthorized)
			return
		}
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
	type domainCounts struct {
		users, identities, memberships, bindings int
	}
	counts := func() domainCounts {
		t.Helper()
		var result domainCounts
		for i, table := range []string{"users", "user_identities", "organization_memberships", "role_bindings"} {
			rows, err := store.Query(ctx, grant.ReadStrong, "SELECT id FROM "+table)
			if err != nil {
				t.Fatal(err)
			}
			switch i {
			case 0:
				result.users = len(rows)
			case 1:
				result.identities = len(rows)
			case 2:
				result.memberships = len(rows)
			case 3:
				result.bindings = len(rows)
			}
		}
		return result
	}
	before := counts()
	for _, consistency := range []string{"strong", "local", "prefer-strong"} {
		w := call("GET", "/api/v1/me?consistency="+consistency, "", "", 404)
		var response struct {
			Code string `json:"code"`
		}
		if err := json.Unmarshal(w.Body.Bytes(), &response); err != nil || response.Code != "LOCAL_USER_NOT_PROVISIONED" {
			t.Fatalf("missing mapping response=%s err=%v", w.Body.String(), err)
		}
		if got := counts(); got != before {
			t.Fatalf("GET /me %s wrote domain rows: got=%v want=%v", consistency, got, before)
		}
	}
	call("POST", "/api/v1/me/ensure", `{"unexpected":true}`, "", 400)
	if got := counts(); got != before {
		t.Fatalf("body-bearing ensure wrote domain rows: got=%v want=%v", got, before)
	}
	w := call("POST", "/api/v1/me/ensure", "", "", 200)
	var ensured map[string]string
	if err := json.Unmarshal(w.Body.Bytes(), &ensured); err != nil || len(ensured) != 1 || ensured["userId"] == "" {
		t.Fatalf("ensure response=%s err=%v", w.Body.String(), err)
	}
	afterEnsure := counts()
	if afterEnsure.users != before.users+1 || afterEnsure.identities != before.identities+1 || afterEnsure.memberships != before.memberships || afterEnsure.bindings != before.bindings {
		t.Fatalf("ensure counts=%v before=%v", afterEnsure, before)
	}
	if _, err := store.Exec(ctx, "UPDATE users SET display_name='Custom profile' WHERE id=?", ensured["userId"]); err != nil {
		t.Fatal(err)
	}
	w = call("POST", "/api/v1/me/ensure", "", "", 200)
	var repeated map[string]string
	if err := json.Unmarshal(w.Body.Bytes(), &repeated); err != nil || len(repeated) != 1 || repeated["userId"] != ensured["userId"] {
		t.Fatalf("repeat ensure response=%s err=%v", w.Body.String(), err)
	}
	if got := counts(); got != afterEnsure {
		t.Fatalf("repeat ensure wrote domain rows: got=%v want=%v", got, afterEnsure)
	}
	call("GET", "/api/v1/me", "", "", 200)
	current, err := svc.CurrentUser(ctx, "Bearer tok", "", grant.ReadStrong)
	if err != nil || current.User.DisplayName != "Custom profile" {
		t.Fatalf("ensure updated profile: user=%+v err=%v", current.User, err)
	}
	call("GET", "/api/v1/me", "", "missing-org", 403)
	if got := counts(); got != afterEnsure {
		t.Fatalf("rejected organization wrote domain rows: got=%v want=%v", got, afterEnsure)
	}
	call("POST", "/api/v1/me/materialize", "", "", 404)
	bad := httptest.NewRequest("GET", "/api/v1/me", nil)
	bad.Header.Set("Authorization", "Bearer bad")
	badResponse := httptest.NewRecorder()
	api.ServeHTTP(badResponse, bad)
	if badResponse.Code != 401 {
		t.Fatalf("invalid token=%d: %s", badResponse.Code, badResponse.Body.String())
	}
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
	ownerResponse := httptest.NewRecorder()
	api.ServeHTTP(ownerResponse, r)
	if ownerResponse.Code != 200 {
		t.Fatal(ownerResponse.Code, ownerResponse.Body.String())
	}
	call("POST", "/api/v1/orgs/new-org/groups", `{"id":"old-owner-group","name":"No"}`, "new-org", 403)
	now := "2026-10-25T00:00:00Z"
	if _, err = store.Exec(ctx, "INSERT INTO users(id,display_name,active,created_at) VALUES('backup-owner','Backup owner',1,?)", now); err != nil {
		t.Fatal(err)
	}
	if _, err = store.Exec(ctx, "INSERT INTO organization_memberships(id,organization_id,user_id,membership_role,status,created_at,updated_at) VALUES('backup-owner-membership','new-org','backup-owner','owner','active',?,?)", now, now); err != nil {
		t.Fatal(err)
	}
	beforeInactive := counts()
	if _, err := svc.SetUserActiveAdmin(ctx, ensured["userId"], false); err != nil {
		t.Fatal(err)
	}
	call("POST", "/api/v1/me/ensure", "", "", 403)
	if got := counts(); got != beforeInactive {
		t.Fatalf("inactive ensure changed domain rows: got=%v want=%v", got, beforeInactive)
	}
}
