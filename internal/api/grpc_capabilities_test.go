package api

import (
	"context"
	"encoding/json"
	"net/http"
	"net/http/httptest"
	"os"
	"strings"
	"testing"
	"time"

	"github.com/mrchypark/grant/internal/grant"
)

func TestGRPCCapabilityDecisionHTTPBoundary(t *testing.T) {
	s := New(testServer().service, true)
	in := grant.GRPCCapabilityDecisionInput{
		WireVersion: 1, ProviderID: "inventory", PEPClientID: "pep", PEPPolicyRevision: 1,
		CatalogGeneration: 1, CapabilityID: "items.get", CapabilityVersion: 1,
		FullMethod: "/inventory.Items/Get", RPCKind: "unary",
		ResourceType: "items", ResourceID: "example",
		ProviderCredential: grant.GRPCCredentialEvidence{Subject: "verified-subject", Audience: "verified-audience"},
		RequestBinding:     "abcdefghijklmnopqrstuv",
	}
	// Force rejection in the service before it needs a store or issuer.
	in.DescriptorDigest = "invalid"
	digest := "sha256:" + strings.Repeat("a", 64)
	in.CatalogDigest, in.ContractDigest, in.MessageDigest = digest, digest, digest
	in.ForwardedMetadataDigest, in.ProviderCredentialDigest = digest, digest
	raw, err := json.Marshal(in)
	if err != nil {
		t.Fatal(err)
	}
	body := string(raw)
	for _, tc := range []struct {
		name, body, key string
		bearer, org     []string
		status          int
		detail          string
	}{
		{"app key required", body, "", []string{"Bearer token"}, []string{"org"}, 401, "x-grant-app-auth-key"},
		{"invalid app key", body, "wrong", []string{"Bearer token"}, []string{"org"}, 401, "invalid application authorization key"},
		{"bearer required", body, "app-secret", nil, []string{"org"}, 401, "authorization header"},
		{"duplicate bearer", body, "app-secret", []string{"Bearer token", "Bearer other"}, []string{"org"}, 400, "multiple authorization"},
		{"organization required", body, "app-secret", []string{"Bearer token"}, nil, 400, "missing x-organization-id"},
		{"duplicate organizations", body, "app-secret", []string{"Bearer token"}, []string{"org", "other"}, 400, "multiple x-organization-id"},
		{"zero length reaches service", body, "app-secret", []string{"Bearer token"}, []string{"org"}, 400, "descriptorDigest"},
		{"message length required", strings.Replace(body, `"messageLength":0,`, "", 1), "app-secret", []string{"Bearer token"}, []string{"org"}, 422, "missing field messageLength"},
		{"organization is not a body field", `{"organizationId":"org",` + body[1:], "app-secret", []string{"Bearer token"}, []string{"org"}, 422, "unknown field organizationId"},
		{"credential is closed", strings.Replace(body, `"subject":`, `"hint":"x","subject":`, 1), "app-secret", []string{"Bearer token"}, []string{"org"}, 422, "unknown field hint"},
		{"credential audience required", strings.Replace(body, `,"audience":"verified-audience"`, "", 1), "app-secret", []string{"Bearer token"}, []string{"org"}, 422, "missing field audience"},
		{"null message length", strings.Replace(body, `"messageLength":0`, `"messageLength":null`, 1), "app-secret", []string{"Bearer token"}, []string{"org"}, 422, "must not be null"},
		{"duplicate wire version", `{"wireVersion":1,` + body[1:], "app-secret", []string{"Bearer token"}, []string{"org"}, 422, "duplicate field wireVersion"},
	} {
		t.Run(tc.name, func(t *testing.T) {
			r := httptest.NewRequest(http.MethodPost, grpcDecisionPath, strings.NewReader(tc.body))
			r.Header.Set("Content-Type", "application/json")
			if tc.key != "" {
				r.Header.Set("X-Grant-App-Auth-Key", tc.key)
			}
			for _, v := range tc.bearer {
				r.Header.Add("Authorization", v)
			}
			for _, v := range tc.org {
				r.Header.Add("X-Organization-ID", v)
			}
			w := &grpcDeadlineRecorder{ResponseRecorder: httptest.NewRecorder()}
			s.ServeHTTP(w, r)
			if w.Code != tc.status || !strings.Contains(w.Body.String(), tc.detail) {
				t.Fatalf("got %d %s; want %d with %q", w.Code, w.Body, tc.status, tc.detail)
			}
			if tc.name == "zero length reaches service" && (len(w.deadlines) != 2 ||
				!w.deadlines[1].IsZero() || w.Header().Get("Connection") != "") {
				t.Fatalf("body deadline was not cleared after successful decode: %v, %v", w.deadlines, w.Header())
			}
		})
	}
}

func TestGRPCCapabilityHTTPRouteAuthentication(t *testing.T) {
	s := testServer()
	r := httptest.NewRequest(http.MethodGet, strings.Replace(grpcCatalogPath, "{providerId}", "inventory", 1), nil)
	w := httptest.NewRecorder()
	s.ServeHTTP(w, r)
	if w.Code != 401 {
		t.Fatalf("catalog without app key: %d %s", w.Code, w.Body)
	}

	for _, path := range []string{grpcGenerationPath, grpcActivationPath} {
		r := httptest.NewRequest(http.MethodPost, strings.Replace(path, "{providerId}", "inventory", 1), strings.NewReader("{"))
		r.Header.Set("Content-Type", "application/json")
		r.Header.Set("X-Grant-App-Auth-Key", "app-secret")
		r.Body = grpcReadFunc(func([]byte) (int, error) {
			t.Fatal("read body without master key")
			return 0, os.ErrDeadlineExceeded
		})
		w := httptest.NewRecorder()
		s.ServeHTTP(w, r)
		if w.Code != 401 {
			t.Fatalf("%s without master key: %d %s", path, w.Code, w.Body)
		}

		learner := New(nil, true)
		r = httptest.NewRequest(http.MethodPost, r.URL.Path, nil)
		deadlineRec := &grpcDeadlineRecorder{ResponseRecorder: httptest.NewRecorder()}
		learner.ServeHTTP(deadlineRec, r)
		if deadlineRec.Code != 503 || len(deadlineRec.deadlines) != 1 || deadlineRec.deadlines[0].IsZero() {
			t.Fatalf("%s on learner: %d %s, deadlines=%v", path, deadlineRec.Code, deadlineRec.Body, deadlineRec.deadlines)
		}
	}
}

func TestGRPCCapabilityCatalogHTTPPublication(t *testing.T) {
	management, store := managementTestServerWithStore(t)
	cfg := management.service.Config()
	cfg.Security.HTTPCapabilityPEPClientID = "inventory-pep"
	s := New(grant.NewService(cfg, store), false)
	const provider = "grant"
	mode := "user"
	if _, err := store.UpsertProviderPepClient(context.Background(), provider, "inventory-pep", "owner", grant.UpsertProviderPepClient{
		Lifecycle: "active", AuthorizationMode: &mode,
	}); err != nil {
		t.Fatal(err)
	}
	cap := grant.GRPCCapability{
		CapabilityID: "inventory.get", Version: 1, Enabled: true, FullMethod: "/inventory.Items/Get",
		DescriptorDigest: "sha256:" + strings.Repeat("a", 64), ActionID: "organization.profile.read",
		ResourceType: "organization", ResourceID: "owner", MaxMessageBytes: 1024,
	}
	var err error
	cap.ContractDigest, err = grant.GRPCCapabilityContractDigest(cap)
	if err != nil {
		t.Fatal(err)
	}
	stage := grant.StageGRPCCapabilityGeneration{SchemaVersion: 1, Generation: 1, Capabilities: []grant.GRPCCapability{cap}}
	stage.CatalogDigest, err = grant.GRPCCapabilityCatalogDigest(provider, stage)
	if err != nil {
		t.Fatal(err)
	}
	request := func(method, path string, input any, status int) *httptest.ResponseRecorder {
		t.Helper()
		raw, err := json.Marshal(input)
		if err != nil {
			t.Fatal(err)
		}
		r := httptest.NewRequest(method, strings.Replace(path, "{providerId}", provider, 1), strings.NewReader(string(raw)))
		r.Header.Set("Content-Type", "application/json")
		if method == http.MethodPost {
			r.Header.Set("X-Grant-Master-Key", "master-secret")
		} else {
			r.Header.Set("X-Grant-App-Auth-Key", "app-secret")
		}
		w := httptest.NewRecorder()
		s.ServeHTTP(&grpcDeadlineRecorder{ResponseRecorder: w}, r)
		if w.Code != status {
			t.Fatalf("%s %s: got %d %s; want %d", method, path, w.Code, w.Body, status)
		}
		return w
	}
	request(http.MethodPost, grpcGenerationPath, stage, 201)
	request(http.MethodGet, grpcCatalogPath, nil, 404)
	request(http.MethodPost, grpcActivationPath, grant.ActivateGRPCCapabilityGeneration{
		TargetGeneration: 1, TargetCatalogDigest: stage.CatalogDigest,
	}, 200)
	// Exercise the learner router's allowlist using the same fixture authority.
	s = New(s.service, true)
	w := request(http.MethodGet, grpcCatalogPath, nil, 200)
	var out grant.ActiveGRPCCatalog
	if err := json.Unmarshal(w.Body.Bytes(), &out); err != nil {
		t.Fatal(err)
	}
	if out.SchemaVersion != 1 || out.CatalogDigest != stage.CatalogDigest || out.PEPClientID != "inventory-pep" ||
		out.AuthorizationMode != "user" || out.PEPPolicyRevision != 1 || len(out.Capabilities) != 1 ||
		out.Capabilities[0] != cap || w.Header().Get("Cache-Control") != "no-store" {
		t.Fatalf("bad catalog: %+v (%v)", out, w.Header())
	}
}

type grpcDeadlineRecorder struct {
	*httptest.ResponseRecorder
	deadlines []time.Time
}

func (w *grpcDeadlineRecorder) SetReadDeadline(deadline time.Time) error {
	w.deadlines = append(w.deadlines, deadline)
	return nil
}

type grpcReadFunc func([]byte) (int, error)

func (f grpcReadFunc) Read(b []byte) (int, error) { return f(b) }
func (grpcReadFunc) Close() error                 { return nil }

func TestGRPCDecisionBodyReadDeadline(t *testing.T) {
	s := testServer()
	for _, tc := range []struct {
		name, key, bearer, org string
		deadlineSupported      bool
		status                 int
		read                   bool
	}{
		{"missing app key", "", "Bearer token", "org", true, 401, false},
		{"missing Bearer", "app-secret", "", "org", true, 401, false},
		{"missing organization", "app-secret", "Bearer token", "", true, 400, false},
		{"recorder has no deadline support", "app-secret", "Bearer token", "org", false, 503, false},
		{"body hits read deadline", "app-secret", "Bearer token", "org", true, 400, true},
	} {
		t.Run(tc.name, func(t *testing.T) {
			r := httptest.NewRequest(http.MethodPost, grpcDecisionPath, nil)
			r.Header.Set("Content-Type", "application/json")
			for k, v := range map[string]string{"X-Grant-App-Auth-Key": tc.key, "Authorization": tc.bearer, "X-Organization-ID": tc.org} {
				if v != "" {
					r.Header.Set(k, v)
				}
			}
			rec := httptest.NewRecorder()
			deadlineRec := &grpcDeadlineRecorder{ResponseRecorder: rec}
			var w http.ResponseWriter = rec
			if tc.deadlineSupported {
				w = deadlineRec
			}
			before := time.Now()
			read := false
			r.Body = grpcReadFunc(func([]byte) (int, error) {
				read = true
				if len(deadlineRec.deadlines) != 1 || deadlineRec.deadlines[0].Before(before.Add(grpcBodyTimeout)) {
					t.Fatal("body read started without a deadline")
				}
				return 0, os.ErrDeadlineExceeded
			})
			s.ServeHTTP(w, r)
			if rec.Code != tc.status || read != tc.read || rec.Header().Get("Connection") != "close" {
				t.Fatalf("status=%d read=%t headers=%v body=%s", rec.Code, read, rec.Header(), rec.Body)
			}
		})
	}
}
