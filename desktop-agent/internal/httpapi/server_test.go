package httpapi

import (
	"context"
	"encoding/json"
	"net/http"
	"net/http/httptest"
	"strings"
	"sync"
	"testing"
	"time"

	"github.com/cypark/account/desktop-agent/internal/npki"
)

var testServerConfig = Config{
	AllowedOrigins: []string{"http://127.0.0.1:4173"},
	SessionTTL:     5 * time.Minute,
	SessionSecret:  []byte("0123456789abcdef0123456789abcdef"),
}

type stubNPKIService struct {
	certificates []npki.CertificateMetadata
}

func (s stubNPKIService) ListCertificates(context.Context) ([]npki.CertificateMetadata, error) {
	return s.certificates, nil
}

func (s stubNPKIService) Sign(context.Context, npki.SignRequest) (npki.SignResult, error) {
	return npki.SignResult{
		CertificateID: "cert-1",
		Challenge:     "challenge-1",
		Bundle: npki.ProofBundle{
			BusinessNo:        "2768603291",
			CommonName:        "Test Certificate",
			SignCertDerBase64: "AQID",
			SignPriKeyBase64:  "BAUG",
			KMCertDerBase64:   "BwgJ",
			KMPriKeyBase64:    "CgsM",
		},
	}, nil
}

type stubApprover struct {
	decision ApprovalDecision
	err      error
}

func (s stubApprover) PromptApproval(context.Context, ApprovalPrompt) (ApprovalDecision, error) {
	if s.err != nil {
		return ApprovalDecision{}, s.err
	}
	return s.decision, nil
}

type stubGateway struct {
	mu                 sync.Mutex
	lastEphemeral      GatewayEphemeralRequest
	lastStoredRegister GatewayStoredRegistrationRequest
}

func (s *stubGateway) RunEphemeral(_ context.Context, req GatewayEphemeralRequest) (map[string]any, error) {
	s.mu.Lock()
	s.lastEphemeral = req
	s.mu.Unlock()
	return map[string]any{
		"id":             "sync-ephemeral-1",
		"organizationId": "org-1",
		"plantId":        "FAC-001",
		"mode":           "full",
		"status":         "completed",
		"startedAt":      nil,
		"finishedAt":     nil,
		"durationMs":     42,
	}, nil
}

func (s *stubGateway) RegisterStored(_ context.Context, req GatewayStoredRegistrationRequest) (map[string]any, error) {
	s.mu.Lock()
	s.lastStoredRegister = req
	s.mu.Unlock()
	return map[string]any{
		"id":             "stored-cert-1",
		"organizationId": "org-1",
		"businessNo":     "2768603291",
		"commonName":     "Stored Certificate",
	}, nil
}

func TestServerHealthz(t *testing.T) {
	handler := newTestHandler(stubNPKIService{}, stubApprover{})

	req := httptest.NewRequest(http.MethodGet, "/healthz", nil)
	req.Header.Set("Origin", "http://127.0.0.1:4173")
	rec := httptest.NewRecorder()
	handler.ServeHTTP(rec, req)

	if rec.Code != http.StatusOK {
		t.Fatalf("status = %d, want %d", rec.Code, http.StatusOK)
	}
	if got := rec.Header().Get("Access-Control-Allow-Origin"); got != "http://127.0.0.1:4173" {
		t.Fatalf("allow origin = %q, want http://127.0.0.1:4173", got)
	}
	if strings.TrimSpace(rec.Body.String()) != "ok" {
		t.Fatalf("body = %q, want ok", rec.Body.String())
	}
}

func TestServerListsCertificates(t *testing.T) {
	handler := newTestHandler(
		stubNPKIService{
			certificates: []npki.CertificateMetadata{{
				ID:         "cert-1",
				CommonName: "Test Certificate",
				BusinessNo: "2768603291",
				NotAfter:   time.Date(2027, 4, 9, 0, 0, 0, 0, time.UTC),
			}},
		},
		stubApprover{},
	)
	sessionToken := openSession(t, handler)

	req := httptest.NewRequest(http.MethodGet, "/certificates", nil)
	req.Header.Set("Origin", "http://127.0.0.1:4173")
	req.Header.Set("X-Agent-Session", sessionToken)
	rec := httptest.NewRecorder()
	handler.ServeHTTP(rec, req)

	if rec.Code != http.StatusOK {
		t.Fatalf("status = %d, want %d", rec.Code, http.StatusOK)
	}

	var got struct {
		Items []struct {
			ID         string `json:"id"`
			CommonName string `json:"commonName"`
			BusinessNo string `json:"businessNo"`
			NotAfter   string `json:"notAfter"`
		} `json:"items"`
	}
	if err := json.Unmarshal(rec.Body.Bytes(), &got); err != nil {
		t.Fatalf("unmarshal response: %v", err)
	}
	if len(got.Items) != 1 {
		t.Fatalf("items len = %d, want 1", len(got.Items))
	}
	if got.Items[0].BusinessNo != "2768603291" {
		t.Fatalf("businessNo = %q, want 2768603291", got.Items[0].BusinessNo)
	}
}

func TestServerRejectsInvalidSignRequests(t *testing.T) {
	handler := newTestHandler(stubNPKIService{}, stubApprover{})
	sessionToken := openSession(t, handler)

	testCases := []struct {
		name string
		body string
	}{
		{name: "missing password", body: `{"challenge":"nonce-1","certificateId":"cert-1"}`},
		{name: "missing challenge", body: `{"password":"secret","certificateId":"cert-1"}`},
	}

	for _, tc := range testCases {
		t.Run(tc.name, func(t *testing.T) {
			req := httptest.NewRequest(http.MethodPost, "/sign", strings.NewReader(tc.body))
			req.Header.Set("Content-Type", "application/json")
			req.Header.Set("Origin", "http://127.0.0.1:4173")
			req.Header.Set("X-Agent-Session", sessionToken)
			rec := httptest.NewRecorder()
			handler.ServeHTTP(rec, req)

			if rec.Code != http.StatusBadRequest {
				t.Fatalf("status = %d, want %d", rec.Code, http.StatusBadRequest)
			}
		})
	}
}

func TestServerReturnsBundleOnSign(t *testing.T) {
	handler := newTestHandler(stubNPKIService{}, stubApprover{})
	sessionToken := openSession(t, handler)

	req := httptest.NewRequest(http.MethodPost, "/sign", strings.NewReader(`{
		"challenge":"challenge-1",
		"certificateId":"cert-1",
		"password":"secret-password"
	}`))
	req.Header.Set("Content-Type", "application/json")
	req.Header.Set("Origin", "http://127.0.0.1:4173")
	req.Header.Set("X-Agent-Session", sessionToken)
	rec := httptest.NewRecorder()
	handler.ServeHTTP(rec, req)

	if rec.Code != http.StatusOK {
		t.Fatalf("status = %d, want %d", rec.Code, http.StatusOK)
	}

	var got struct {
		CertificateID string `json:"certificateId"`
		Bundle        struct {
			BusinessNo string `json:"businessNo"`
		} `json:"bundle"`
	}
	if err := json.Unmarshal(rec.Body.Bytes(), &got); err != nil {
		t.Fatalf("unmarshal response: %v", err)
	}
	if got.CertificateID != "cert-1" {
		t.Fatalf("certificateId = %q, want cert-1", got.CertificateID)
	}
	if got.Bundle.BusinessNo != "2768603291" {
		t.Fatalf("businessNo = %q, want 2768603291", got.Bundle.BusinessNo)
	}
}

func TestServerApproveEphemeralRun(t *testing.T) {
	gateway := &stubGateway{}
	handler := newTestHandler(
		stubNPKIService{
			certificates: []npki.CertificateMetadata{{
				ID:         "cert-1",
				CommonName: "Test Certificate",
				BusinessNo: "2768603291",
				NotAfter:   time.Date(2027, 4, 9, 0, 0, 0, 0, time.UTC),
			}},
		},
		stubApprover{decision: ApprovalDecision{
			CertificateID: "cert-1",
			Password:      "secret-password",
		}},
		gateway,
	)
	sessionToken := openSession(t, handler)

	req := httptest.NewRequest(http.MethodPost, "/approve-ephemeral-run", strings.NewReader(`{
		"gatewayBaseUrl":"http://gateway.local",
		"organizationId":"org-1",
		"businessNo":"2768603291",
		"bearerToken":"token-1",
		"activeOrganizationId":"org-1"
	}`))
	req.Header.Set("Content-Type", "application/json")
	req.Header.Set("Origin", "http://127.0.0.1:4173")
	req.Header.Set("X-Agent-Session", sessionToken)
	rec := httptest.NewRecorder()
	handler.ServeHTTP(rec, req)

	if rec.Code != http.StatusAccepted {
		t.Fatalf("status = %d, want %d", rec.Code, http.StatusAccepted)
	}
	var got map[string]any
	if err := json.Unmarshal(rec.Body.Bytes(), &got); err != nil {
		t.Fatalf("unmarshal response: %v", err)
	}
	if got["status"] != "completed" {
		t.Fatalf("status = %v, want completed", got["status"])
	}
	if gateway.lastEphemeral.Password != "secret-password" {
		t.Fatalf("password = %q, want secret-password", gateway.lastEphemeral.Password)
	}
}

func TestServerApproveStoredRegistration(t *testing.T) {
	gateway := &stubGateway{}
	handler := newTestHandler(
		stubNPKIService{
			certificates: []npki.CertificateMetadata{{
				ID:         "cert-1",
				CommonName: "Test Certificate",
				BusinessNo: "2768603291",
				NotAfter:   time.Date(2027, 4, 9, 0, 0, 0, 0, time.UTC),
			}},
		},
		stubApprover{decision: ApprovalDecision{
			CertificateID: "cert-1",
			Password:      "secret-password",
		}},
		gateway,
	)
	sessionToken := openSession(t, handler)

	req := httptest.NewRequest(http.MethodPost, "/approve-stored-registration", strings.NewReader(`{
		"gatewayBaseUrl":"http://gateway.local",
		"organizationId":"org-1",
		"businessNo":"2768603291",
		"bearerToken":"token-1",
		"activeOrganizationId":"org-1"
	}`))
	req.Header.Set("Content-Type", "application/json")
	req.Header.Set("Origin", "http://127.0.0.1:4173")
	req.Header.Set("X-Agent-Session", sessionToken)
	rec := httptest.NewRecorder()
	handler.ServeHTTP(rec, req)

	if rec.Code != http.StatusCreated {
		t.Fatalf("status = %d, want %d", rec.Code, http.StatusCreated)
	}
	var got map[string]any
	if err := json.Unmarshal(rec.Body.Bytes(), &got); err != nil {
		t.Fatalf("unmarshal response: %v", err)
	}
	if got["id"] != "stored-cert-1" {
		t.Fatalf("id = %v, want stored-cert-1", got["id"])
	}
	if gateway.lastStoredRegister.Password != "secret-password" {
		t.Fatalf("password = %q, want secret-password", gateway.lastStoredRegister.Password)
	}
}

func TestServerApproveEphemeralRunHandlesUserRejection(t *testing.T) {
	handler := newTestHandler(stubNPKIService{}, stubApprover{err: ErrApprovalRejected})
	sessionToken := openSession(t, handler)

	req := httptest.NewRequest(http.MethodPost, "/approve-ephemeral-run", strings.NewReader(`{
		"gatewayBaseUrl":"http://gateway.local",
		"organizationId":"org-1",
		"businessNo":"2768603291"
	}`))
	req.Header.Set("Content-Type", "application/json")
	req.Header.Set("Origin", "http://127.0.0.1:4173")
	req.Header.Set("X-Agent-Session", sessionToken)
	rec := httptest.NewRecorder()
	handler.ServeHTTP(rec, req)

	if rec.Code != http.StatusForbidden {
		t.Fatalf("status = %d, want %d", rec.Code, http.StatusForbidden)
	}
}

func TestServerRequiresSessionForSensitiveRoutes(t *testing.T) {
	handler := newTestHandler(stubNPKIService{}, stubApprover{})

	req := httptest.NewRequest(http.MethodGet, "/certificates", nil)
	req.Header.Set("Origin", "http://127.0.0.1:4173")
	rec := httptest.NewRecorder()
	handler.ServeHTTP(rec, req)

	if rec.Code != http.StatusUnauthorized {
		t.Fatalf("status = %d, want %d", rec.Code, http.StatusUnauthorized)
	}
}

func TestServerRejectsDisallowedOrigin(t *testing.T) {
	handler := newTestHandler(stubNPKIService{}, stubApprover{})

	req := httptest.NewRequest(http.MethodPost, "/sessions/open", nil)
	req.Header.Set("Origin", "http://evil.example")
	rec := httptest.NewRecorder()
	handler.ServeHTTP(rec, req)

	if rec.Code != http.StatusForbidden {
		t.Fatalf("status = %d, want %d", rec.Code, http.StatusForbidden)
	}
}

func TestServerHandlesCORSPreflight(t *testing.T) {
	handler := newTestHandler(stubNPKIService{}, stubApprover{})

	req := httptest.NewRequest(http.MethodOptions, "/sessions/open", nil)
	req.Header.Set("Origin", "http://127.0.0.1:4173")
	req.Header.Set("Access-Control-Request-Method", "POST")
	req.Header.Set("Access-Control-Request-Headers", "content-type")
	rec := httptest.NewRecorder()
	handler.ServeHTTP(rec, req)

	if rec.Code != http.StatusNoContent {
		t.Fatalf("status = %d, want %d", rec.Code, http.StatusNoContent)
	}
	if got := rec.Header().Get("Access-Control-Allow-Origin"); got != "http://127.0.0.1:4173" {
		t.Fatalf("allow origin = %q, want http://127.0.0.1:4173", got)
	}
	if got := rec.Header().Get("Access-Control-Allow-Methods"); !strings.Contains(got, "POST") {
		t.Fatalf("allow methods = %q, want POST included", got)
	}
}

func newTestHandler(npkiSvc stubNPKIService, approver stubApprover, gateways ...*stubGateway) http.Handler {
	gateway := &stubGateway{}
	if len(gateways) > 0 && gateways[0] != nil {
		gateway = gateways[0]
	}
	return NewServer(npkiSvc, approver, gateway, testServerConfig).Handler()
}

func openSession(t *testing.T, handler http.Handler) string {
	t.Helper()

	req := httptest.NewRequest(http.MethodPost, "/sessions/open", nil)
	req.Header.Set("Origin", "http://127.0.0.1:4173")
	rec := httptest.NewRecorder()
	handler.ServeHTTP(rec, req)

	if rec.Code != http.StatusCreated {
		t.Fatalf("status = %d, want %d", rec.Code, http.StatusCreated)
	}

	var got struct {
		SessionToken string `json:"sessionToken"`
	}
	if err := json.Unmarshal(rec.Body.Bytes(), &got); err != nil {
		t.Fatalf("unmarshal response: %v", err)
	}
	if got.SessionToken == "" {
		t.Fatal("sessionToken is empty")
	}
	return got.SessionToken
}
