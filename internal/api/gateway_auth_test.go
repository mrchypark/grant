package api

import (
	"bytes"
	"crypto/sha256"
	"encoding/hex"
	"encoding/json"
	"net/http"
	"net/http/httptest"
	"strings"
	"testing"

	"github.com/mrchypark/grant/internal/grant"
)

func TestTrustedGatewayAuthentication(t *testing.T) {
	store, err := grant.OpenStore(t.Context(), grant.Config{Database: grant.DatabaseConfig{DataDir: t.TempDir(), DefaultReadConsistency: grant.ReadStrong}})
	if err != nil {
		t.Fatal(err)
	}
	defer store.Close()
	if err := store.EnsureBootstrapData(t.Context()); err != nil {
		t.Fatal(err)
	}

	gatewayKey := strings.Repeat("g", 32)
	srv := New(grant.NewService(grant.Config{
		Security: grant.SecurityConfig{
			MasterKey:         strings.Repeat("m", 32),
			AppAuthKey:        strings.Repeat("a", 32),
			TrustedGatewayKey: gatewayKey,
		},
		Auth: grant.AuthConfig{ProviderMode: "oidc"},
	}, store), false)

	req := httptest.NewRequest("POST", "/api/v1/me/ensure", nil)
	req.Header.Set("X-Grant-Gateway-Key", gatewayKey)
	req.Header.Set("X-Grant-Subject", "gw-user-1")
	req.Header.Set("X-Grant-Client-Id", "gw-app")
	rec := httptest.NewRecorder()
	srv.ServeHTTP(rec, req)
	if rec.Code != http.StatusOK {
		t.Fatalf("materialize expected 200, got %d: %s", rec.Code, rec.Body.String())
	}

	req = httptest.NewRequest("GET", "/api/v1/me", nil)
	req.Header.Set("X-Grant-Gateway-Key", gatewayKey)
	req.Header.Set("X-Grant-Subject", "gw-user-1")
	req.Header.Set("X-Grant-Client-Id", "gw-app")
	rec = httptest.NewRecorder()
	srv.ServeHTTP(rec, req)
	if rec.Code != http.StatusOK {
		t.Fatalf("expected 200, got %d: %s", rec.Code, rec.Body.String())
	}

	req = httptest.NewRequest("GET", "/api/v1/me", nil)
	req.Header.Set("X-Grant-Gateway-Key", gatewayKey)
	rec = httptest.NewRecorder()
	srv.ServeHTTP(rec, req)
	if rec.Code != http.StatusUnauthorized {
		t.Fatalf("expected 401, got %d", rec.Code)
	}

	req = httptest.NewRequest("GET", "/api/v1/me", nil)
	req.Header.Set("X-Grant-Gateway-Key", "wrong-key")
	req.Header.Set("X-Grant-Subject", "gw-user-1")
	rec = httptest.NewRecorder()
	srv.ServeHTTP(rec, req)
	if rec.Code != http.StatusUnauthorized {
		t.Fatalf("expected 401, got %d", rec.Code)
	}

	disabledSrv := New(grant.NewService(grant.Config{
		Security: grant.SecurityConfig{MasterKey: "m", AppAuthKey: "a"},
		Auth:     grant.AuthConfig{ProviderMode: "oidc"},
	}, store), false)
	req = httptest.NewRequest("GET", "/api/v1/me", nil)
	req.Header.Set("X-Grant-Gateway-Key", gatewayKey)
	req.Header.Set("X-Grant-Subject", "gw-user-1")
	rec = httptest.NewRecorder()
	disabledSrv.ServeHTTP(rec, req)
	if rec.Code != http.StatusUnauthorized {
		t.Fatalf("expected 401 when gateway auth disabled, got %d", rec.Code)
	}
}

func TestTrustedGatewayHTTPCapabilityDecide(t *testing.T) {
	ctx := t.Context()
	store, err := grant.OpenStore(ctx, grant.Config{Database: grant.DatabaseConfig{DataDir: t.TempDir(), DefaultReadConsistency: grant.ReadStrong}})
	if err != nil {
		t.Fatal(err)
	}
	defer store.Close()
	if err := store.EnsureBootstrapData(t.Context()); err != nil {
		t.Fatal(err)
	}

	gatewayKey := strings.Repeat("g", 32)
	appAuthKey := strings.Repeat("a", 32)
	masterKey := strings.Repeat("m", 32)
	svc := grant.NewService(grant.Config{
		Security: grant.SecurityConfig{
			MasterKey:                 masterKey,
			AppAuthKey:                appAuthKey,
			TrustedGatewayKey:         gatewayKey,
			HTTPCapabilityPEPClientID: "exposure-pep",
		},
		Auth: grant.AuthConfig{
			ProviderMode: "goauthy",
			Goauthy: &grant.GoauthyConfig{
				Issuer:        "https://auth.example.test",
				ApplicationID: "operable-app",
			},
		},
	}, store)

	// Setup org, user, membership, provider, action, role, binding, PEP, capability catalog.
	if _, err := svc.UpsertServiceProviderAdmin(ctx, grant.ServiceProvider{ID: "operable", Name: "Operable", Active: true}); err != nil {
		t.Fatal(err)
	}
	actionID := "operable.entity.read"
	if _, err := svc.ReplaceProviderActionsAdmin(ctx, "operable", []grant.Action{{ID: actionID, ProviderID: "operable", Code: "entity.read", Name: "Read", RiskLevel: "low", GrantableToApplications: true}}); err != nil {
		t.Fatal(err)
	}
	mode := "user"
	if _, err := store.UpsertProviderPepClient(ctx, "operable", "exposure-pep", "conalog-grant", grant.UpsertProviderPepClient{Lifecycle: "active", AuthorizationMode: &mode}); err != nil {
		t.Fatal(err)
	}
	cap := grant.HttpCapability{
		CapabilityID: "entity.read.0", Version: 1, Enabled: true, AccessMode: "authorized",
		Method: "GET", PathTemplate: "/entities/{id}", ActionID: &actionID,
		Resource: &grant.HttpCapabilityResource{ResourceType: "entity", ID: grant.HttpResourceExtractor{Kind: "pathSegment", Name: "id", Format: "asciiToken"}},
		Query:    grant.HttpQueryPolicy{Mode: "none"},
		Request:  grant.HttpRequestPolicy{Body: "none", MaxBytes: 0, ForwardHeaders: []string{}},
		Response: grant.HttpResponsePolicy{MaxBytes: 1 << 20, ForwardHeaders: []string{}, Redirects: "deny"},
	}
	cap.ContractDigest, err = grant.HttpCapabilityContractDigest(cap)
	if err != nil {
		t.Fatal(err)
	}
	stage := grant.StageHttpCapabilityGeneration{SchemaVersion: 1, Generation: 1, Capabilities: []grant.HttpCapability{cap}}
	stage.CatalogDigest, err = grant.HttpCapabilityCatalogDigest("operable", stage)
	if err != nil {
		t.Fatal(err)
	}
	if _, err := store.StageHttpCapabilityGeneration(ctx, "operable", stage); err != nil {
		t.Fatal(err)
	}
	if _, err := store.ActivateHttpCapabilityGeneration(ctx, "operable", "conalog-grant", grant.ActivateHttpCapabilityGeneration{TargetGeneration: 1, TargetCatalogDigest: stage.CatalogDigest}); err != nil {
		t.Fatal(err)
	}

	srv := New(svc, false)

	// Materialize gw-user-1 and grant membership + role binding in conalog-grant org.
	ensureReq := httptest.NewRequest("POST", "/api/v1/me/ensure", nil)
	ensureReq.Header.Set("X-Grant-Gateway-Key", gatewayKey)
	ensureReq.Header.Set("X-Grant-Subject", "gw-user-1")
	ensureReq.Header.Set("X-Grant-Client-Id", "operable-app")
	ensureRec := httptest.NewRecorder()
	srv.ServeHTTP(ensureRec, ensureReq)
	if ensureRec.Code != http.StatusOK {
		t.Fatalf("ensure failed: %d %s", ensureRec.Code, ensureRec.Body.String())
	}
	var ensureResp struct {
		ID string `json:"userId"`
	}
	if err := json.Unmarshal(ensureRec.Body.Bytes(), &ensureResp); err != nil {
		t.Fatal(err)
	}
	if _, err := store.Exec(ctx, "INSERT INTO organization_memberships(id,organization_id,user_id,membership_role,status,created_at,updated_at) VALUES('gw-mem','conalog-grant',?,'owner','active','now','now')", ensureResp.ID); err != nil {
		t.Fatal(err)
	}
	if _, err := store.Exec(ctx, "INSERT INTO role_actions(role_id,action_id) VALUES('role-system-bootstrap-owner',?)", actionID); err != nil {
		t.Fatal(err)
	}
	if _, err := store.Exec(ctx, "INSERT INTO role_bindings(id,subject_type,subject_id,subject_organization_id,provider_id,resource_type,resource_id,role_id,state,created_by_user_id,created_at,updated_at) VALUES('gw-bind','user',?,'conalog-grant','operable','entity','ent-1','role-system-bootstrap-owner','active','conalog-grant','now','now')", ensureResp.ID); err != nil {
		t.Fatal(err)
	}

	emptySum := sha256.Sum256(nil)
	emptyDigest := "sha256:" + hex.EncodeToString(emptySum[:])
	idName := "id"
	decideBody, err := json.Marshal(grant.HttpCapabilityDecisionInput{
		WireVersion:              2,
		PEPPolicyRevision:        1,
		ProviderID:               "operable",
		PEPClientID:              "exposure-pep",
		CatalogGeneration:        1,
		CatalogDigest:            stage.CatalogDigest,
		CapabilityID:             cap.CapabilityID,
		CapabilityVersion:        1,
		ContractDigest:           cap.ContractDigest,
		Method:                   "GET",
		CanonicalPath:            "/entities/ent-1",
		CanonicalQueryDigest:     emptyDigest,
		ForwardedHeaderDigest:    emptyDigest,
		BodyDigest:               emptyDigest,
		ProviderCredentialDigest: emptyDigest,
		ProviderCredential:       grant.HttpCapabilityCredentialEvidence{Subject: "gw-user-1", Audience: "operable-app"},
		ResourceEvidence:         grant.HttpCapabilityResourceEvidence{Kind: "pathSegment", Name: &idName, CanonicalValue: "ent-1"},
		ResourceType:             "entity",
		ResourceID:               "ent-1",
		RequestBinding:           strings.Repeat("a", 22),
	})
	if err != nil {
		t.Fatal(err)
	}

	// Call POST /api/v2/http-capabilities/decide WITHOUT Authorization header, using X-Grant-Gateway-Key.
	req := httptest.NewRequest("POST", "/api/v2/http-capabilities/decide", bytes.NewReader(decideBody))
	req.Header.Set("Content-Type", "application/json")
	req.Header.Set("X-Grant-App-Auth-Key", appAuthKey)
	req.Header.Set("X-Grant-Gateway-Key", gatewayKey)
	req.Header.Set("X-Grant-Subject", "gw-user-1")
	req.Header.Set("X-Grant-Client-Id", "operable-app")
	req.Header.Set("X-Organization-ID", "conalog-grant")
	rec := httptest.NewRecorder()
	srv.ServeHTTP(rec, req)
	if rec.Code != http.StatusOK {
		t.Fatalf("decide with trusted gateway expected 200, got %d: %s", rec.Code, rec.Body.String())
	}
	var decision grant.HttpCapabilityDecision
	if err := json.Unmarshal(rec.Body.Bytes(), &decision); err != nil {
		t.Fatal(err)
	}
	if !decision.Allowed {
		t.Fatalf("expected allowed=true, got %+v", decision)
	}
}
