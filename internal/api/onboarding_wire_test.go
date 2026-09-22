package api

import (
	"bytes"
	"context"
	"crypto/sha256"
	"encoding/hex"
	"encoding/json"
	"io"
	"net/http"
	"net/http/httptest"
	"strings"
	"testing"

	"github.com/Conalog/grant/internal/grant"
)

// This test crosses the real Grant HTTP router and HTTP transport for both PDP
// and business requests. The IdP is simulated; Exposure transport has its own tests.
func TestOnboardingHTTPDecisionAndManagementFlow(t *testing.T) {
	ctx := context.Background()
	idp := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		_ = json.NewEncoder(w).Encode(map[string]any{"token": "tok", "record": map[string]any{"id": "newuser12345678", "collectionId": "users", "collectionName": "users", "name": "New user"}})
	}))
	defer idp.Close()
	cfg := grant.Config{Security: grant.SecurityConfig{BootstrapUserID: "seed", MasterKey: "master", AppAuthKey: "app-key", HTTPCapabilityPEPApplicationID: "onboarding-app"}, Auth: grant.AuthConfig{ProviderMode: "pocketbase", PocketBase: &grant.PocketBaseConfig{BaseURL: idp.URL, Collection: "users", ApplicationID: "onboarding-app"}}, Database: grant.DatabaseConfig{DataDir: t.TempDir(), DefaultReadConsistency: grant.ReadStrong}}
	store, err := grant.OpenStore(ctx, cfg)
	if err != nil {
		t.Fatal(err)
	}
	defer store.Close()
	if err = store.EnsureBootstrapData(ctx); err != nil {
		t.Fatal(err)
	}
	svc := grant.NewService(cfg, store)
	if _, err = svc.CreateServiceProviderAdmin(ctx, grant.ServiceProvider{ID: "grant-management", Name: "Management", Active: true}); err != nil {
		t.Fatal(err)
	}
	stage, actions, err := grant.ManagementCatalog(1, nil)
	if err != nil {
		t.Fatal(err)
	}
	if _, err = svc.ReplaceProviderActionsAdmin(ctx, "grant-management", actions); err != nil {
		t.Fatal(err)
	}
	if _, err = svc.CreateApplication(ctx, "seed", "seed", grant.CreateApplicationInput{ID: "onboarding-app", Slug: "onboarding-app", Name: "Onboarding", ApplicationType: "confidential", AppType: "internal", Active: true}); err != nil {
		t.Fatal(err)
	}
	ids := make([]string, 0, len(actions))
	for _, a := range actions {
		ids = append(ids, a.ID)
	}
	if _, err = svc.ReplaceApplicationActionsAuthorized(ctx, "seed", "seed", "onboarding-app", ids); err != nil {
		t.Fatal(err)
	}
	if _, err = svc.SetApplicationTrustAdmin(ctx, "seed", "onboarding-app", true); err != nil {
		t.Fatal(err)
	}
	if _, err = svc.SetProviderPepClientAdmin(ctx, "grant-management", "onboarding-pep", grant.UpsertProviderPepClient{ApplicationID: "onboarding-app", Lifecycle: "active"}); err != nil {
		t.Fatal(err)
	}
	if _, err = svc.StageHttpCapabilityGenerationAdmin(ctx, "grant-management", stage); err != nil {
		t.Fatal(err)
	}
	if _, err = svc.ActivateHttpCapabilityGenerationAdmin(ctx, "grant-management", grant.ActivateHttpCapabilityGeneration{TargetGeneration: 1, TargetCatalogDigest: stage.CatalogDigest}); err != nil {
		t.Fatal(err)
	}
	server := httptest.NewServer(New(svc, false))
	defer server.Close()
	call := func(method, path, org, body string, pdp bool, want int) ([]byte, http.Header) {
		t.Helper()
		r, e := http.NewRequest(method, server.URL+path, strings.NewReader(body))
		if e != nil {
			t.Fatal(e)
		}
		r.Header.Set("Authorization", "Bearer tok")
		if org != "" {
			r.Header.Set("X-Organization-ID", org)
		}
		if body != "" {
			r.Header.Set("Content-Type", "application/json")
		}
		if pdp {
			r.Header.Set("X-Grant-App-Auth-Key", "app-key")
		}
		response, e := server.Client().Do(r)
		if e != nil {
			t.Fatal(e)
		}
		defer response.Body.Close()
		data, e := io.ReadAll(response.Body)
		if e != nil {
			t.Fatal(e)
		}
		if response.StatusCode != want {
			t.Fatalf("%s %s org=%q: %d want %d %s", method, path, org, response.StatusCode, want, data)
		}
		return data, response.Header
	}
	digest := func(value string) string {
		sum := sha256.Sum256([]byte(value))
		return "sha256:" + hex.EncodeToString(sum[:])
	}
	decision := func(method, path, org, body string, wantStatus int, wantAllowed bool) {
		t.Helper()
		var cap grant.HttpCapability
		for _, candidate := range stage.Capabilities {
			if candidate.Method == method && strings.ReplaceAll(candidate.PathTemplate, "{organizationId}", org) == path {
				cap = candidate
				break
			}
		}
		if cap.CapabilityID == "" {
			t.Fatal("missing test capability", method, path)
		}
		resource := cap.Resource.ID.Value
		evidence := grant.HttpCapabilityResourceEvidence{Kind: cap.Resource.ID.Kind, CanonicalValue: resource}
		if evidence.Kind == "pathSegment" {
			resource = org
			name := "organizationId"
			evidence.Name = &name
			evidence.CanonicalValue = org
		}
		input := grant.HttpCapabilityDecisionInput{ProviderID: "grant-management", PEPClientID: "onboarding-pep", CatalogGeneration: 1, CatalogDigest: stage.CatalogDigest, CapabilityID: cap.CapabilityID, CapabilityVersion: cap.Version, ContractDigest: cap.ContractDigest, Method: method, CanonicalPath: path, CanonicalQueryDigest: digest(""), ForwardedHeaderDigest: digest(""), BodyDigest: digest(body), BodyLength: uint64(len(body)), ProviderCredentialDigest: grant.CredentialDigest("Bearer tok"), ProviderCredential: grant.HttpCapabilityCredentialEvidence{Subject: "pocketbase:" + idp.URL + "/users/newuser12345678", Audience: "onboarding-app"}, ResourceEvidence: evidence, ResourceType: cap.Resource.ResourceType, ResourceID: resource, RequestBinding: strings.Repeat("a", 22)}
		encoded, e := json.Marshal(input)
		if e != nil {
			t.Fatal(e)
		}
		data, _ := call("POST", "/api/v2/http-capabilities/decide", org, string(encoded), true, wantStatus)
		if wantStatus == 200 {
			var outcome grant.HttpCapabilityDecision
			if e = json.Unmarshal(data, &outcome); e != nil || outcome.Allowed != wantAllowed {
				t.Fatal("decision", string(data), e)
			}
		}
	}
	decision("POST", "/api/v1/me/materialize", "", "", 200, true)
	decision("GET", "/api/v1/providers", "", "", 400, false)
	users, err := store.Query(ctx, grant.ReadStrong, "SELECT id FROM users")
	if err != nil || len(users) != 1 {
		t.Fatal("PDP wrote a local identity", users, err)
	}
	materialized, _ := call("POST", "/api/v1/me/materialize", "", "", false, 200)
	var local struct {
		UserID string `json:"userId"`
	}
	if err = json.Unmarshal(materialized, &local); err != nil || local.UserID == "" {
		t.Fatal(err, string(materialized))
	}
	for _, path := range []string{"/api/v1/me", "/api/v1/orgs"} {
		decision("GET", path, "", "", 200, true)
		data, _ := call("GET", path, "", "", false, 200)
		if path == "/api/v1/orgs" && !bytes.Equal(bytes.TrimSpace(data), []byte("[]")) {
			t.Fatal("new identity gained memberships", string(data))
		}
	}
	create := `{"id":"self-created","name":"My organization"}`
	decision("POST", "/api/v1/orgs", "", create, 200, true)
	call("POST", "/api/v1/orgs", "", `{"id":"spoofed","name":"No","ownerUserId":"seed"}`, false, 422)
	call("POST", "/api/v1/orgs", "", create, false, 201)
	call("POST", "/api/v1/orgs", "", create, false, 409)
	decision("GET", "/api/v1/orgs/self-created", "self-created", "", 200, true)
	call("GET", "/api/v1/orgs/self-created", "self-created", "", false, 200)
	call("GET", "/api/v1/orgs/self-created/owner", "self-created", "", false, 200)
	call("GET", "/api/v1/orgs/self-created/members", "self-created", "", false, 200)
	call("PUT", "/api/v1/orgs/self-created/members/seed", "self-created", "", false, 200)
	call("DELETE", "/api/v1/orgs/self-created/members/seed", "self-created", "", false, 204)
	decision("POST", "/api/v1/orgs/self-created/deactivate", "self-created", "", 200, true)
	call("POST", "/api/v1/orgs/self-created/deactivate", "self-created", "", false, 200)
	decision("GET", "/api/v1/orgs/self-created", "self-created", "", 403, false)
	decision("POST", "/api/v1/orgs/self-created/reactivate", "self-created", "", 200, true)
	call("POST", "/api/v1/orgs/self-created/reactivate", "self-created", "", false, 200)
	if _, err = svc.SetUserActiveAdmin(ctx, local.UserID, false); err != nil {
		t.Fatal(err)
	}
	decision("GET", "/api/v1/me", "", "", 403, false)
	call("POST", "/api/v1/orgs", "", `{"id":"inactive-user","name":"No"}`, false, 403)
}
