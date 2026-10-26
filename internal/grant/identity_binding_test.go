package grant

import (
	"context"
	"encoding/json"
	"net/http"
	"net/http/httptest"
	"testing"
)

func TestIdentityBindingVerifiedAndFailClosed(t *testing.T) {
	ctx, s := serviceFixture(t)
	valid := false
	calls := 0
	up := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		calls++
		if r.Method != http.MethodGet || r.URL.Path != "/auth/v1/clients/external-client" || r.Header.Get("Authorization") != "API-Key reader" {
			t.Error("unexpected IdP request", r.Method, r.URL.Path)
			w.WriteHeader(400)
			return
		}
		if !valid {
			w.WriteHeader(404)
			return
		}
		_ = json.NewEncoder(w).Encode(map[string]any{"id": "external-client", "name": "Client", "enabled": true, "confidential": true, "redirect_uris": []string{}, "flows_enabled": []string{"authorization_code", "refresh_token", "client_credentials"}})
	}))
	defer up.Close()
	s.cfg.Auth.ProviderMode = "rauthy"
	s.cfg.Auth.Rauthy = RauthyConfig{BaseURL: up.URL, ClientReadAPIKey: "reader"}
	create := func(id string) Application {
		t.Helper()
		a, err := s.CreateApplication(ctx, "owner", "owner", CreateApplicationInput{ID: id, Slug: id, Name: "Client", ApplicationType: "confidential", AppType: "server", Active: true})
		if err != nil {
			t.Fatal(err)
		}
		if a.Application.RauthyClientID != nil || a.Application.ProvisioningStatus != "unprovisioned" {
			t.Fatal("registration minted a client", a)
		}
		return a.Application
	}
	create("client")
	if _, err := s.store.BindIdentityToUser(ctx, BindIdentityInput{UserID: "owner", Provider: "rauthy", ExternalUserID: "identity-user", DisplayName: "Owner", Active: true}); err != nil {
		t.Fatal(err)
	}
	identityCtx := context.WithValue(ctx, replicaIdentityKey{}, ExternalIdentity{Provider: "rauthy", ExternalUserID: "identity-user", ClientID: "external-client", Active: true})
	if _, err := s.resolveApplicationAuthorizationContext(identityCtx, "token", "owner"); ErrorStatus(err) != 403 {
		t.Fatal("unverified app authorized", err)
	}
	for _, tc := range []struct {
		org, client string
		want        int
	}{{"owner", "", 400}, {"other", "external-client", 404}, {"owner", "external-client", 409}} {
		_, err := s.BindApplicationIdentityAdmin(ctx, tc.org, "client", IdentityBindingInput{ClientID: tc.client})
		if ErrorStatus(err) != tc.want {
			t.Fatalf("%+v: %v", tc, err)
		}
	}
	a, err := s.store.GetApplicationWithConsistency(ctx, "client", ReadStrong)
	if err != nil || a.ProvisioningStatus != "unprovisioned" || a.RauthyClientID != nil {
		t.Fatal("failed verification mutated app", a, err)
	}
	for _, mode := range []string{"pocketbase", ""} {
		before := calls
		s.cfg.Auth.ProviderMode = mode
		_, err = s.BindApplicationIdentityAdmin(ctx, "owner", "client", IdentityBindingInput{ClientID: "external-client"})
		if ErrorStatus(err) != 409 || calls != before {
			t.Fatal("unsupported adapter made a request", mode, err)
		}
	}
	s.cfg.Auth.ProviderMode = "rauthy"
	valid = true
	a, err = s.BindApplicationIdentityAdmin(ctx, "owner", "client", IdentityBindingInput{ClientID: "external-client"})
	if err != nil || a.ProvisioningStatus != "ready" || ptrValue(a.RauthyClientID) != "external-client" {
		t.Fatal(a, err)
	}
	if got, err := s.resolveApplicationAuthorizationContext(identityCtx, "token", "owner"); err != nil || got.ApplicationID != "client" {
		t.Fatal("verified app not resolved", got, err)
	}
	if _, err = s.BindApplicationIdentityAdmin(ctx, "owner", "client", IdentityBindingInput{ClientID: "different-client"}); ErrorStatus(err) != 409 {
		t.Fatal("rebind accepted", err)
	}
	create("second")
	if _, err = s.BindApplicationIdentityAdmin(ctx, "owner", "second", IdentityBindingInput{ClientID: "external-client"}); ErrorStatus(err) != 409 {
		t.Fatal("duplicate binding accepted", err)
	}
}

func TestIdentityBindingRejectsChangedManifest(t *testing.T) {
	ctx, s := serviceFixture(t)
	a, err := s.CreateApplication(ctx, "owner", "owner", CreateApplicationInput{ID: "client", Slug: "client", Name: "Client", ApplicationType: "confidential", AppType: "server", Active: true})
	if err != nil {
		t.Fatal(err)
	}
	if _, err = s.store.Exec(ctx, "UPDATE applications SET name='Changed' WHERE id='client'"); err != nil {
		t.Fatal(err)
	}
	if _, err = s.store.BindApplicationIdentity(ctx, a.Application, "external-client", nil); err == nil {
		t.Fatal("stale verification accepted")
	}
	got, err := s.store.GetApplicationWithConsistency(ctx, "client", ReadStrong)
	if err != nil || got.ProvisioningStatus != "unprovisioned" {
		t.Fatal(got, err)
	}
}
