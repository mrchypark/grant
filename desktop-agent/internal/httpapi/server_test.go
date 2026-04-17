package httpapi

import (
	"context"
	"crypto"
	"crypto/rand"
	"crypto/rsa"
	"crypto/sha256"
	"crypto/x509"
	"crypto/x509/pkix"
	"encoding/asn1"
	"encoding/base64"
	"encoding/json"
	"math/big"
	"net/http"
	"net/http/httptest"
	"os"
	"path/filepath"
	"strings"
	"sync"
	"testing"
	"time"

	"github.com/cypark/account/desktop-agent/internal/npki"
	pkcs12 "software.sslmate.com/src/go-pkcs12"
)

var testServerConfig = Config{
	AllowedOrigins: []string{"http://127.0.0.1:4173"},
	GatewayBaseURL: "http://gateway.local",
	SessionTTL:     5 * time.Minute,
	SessionSecret:  []byte("0123456789abcdef0123456789abcdef"),
}

type stubNPKIService struct {
	certificates []npki.CertificateMetadata
	lastSignReq  npki.SignRequest
}

func (s *stubNPKIService) ListCertificates(context.Context) ([]npki.CertificateMetadata, error) {
	return s.certificates, nil
}

func (s *stubNPKIService) Sign(_ context.Context, req npki.SignRequest) (npki.SignResult, error) {
	req.PasswordBytes = append([]byte(nil), req.PasswordBytes...)
	s.lastSignReq = req
	return npki.SignResult{
		CertificateID:            "cert-1",
		Challenge:                req.Challenge,
		ChallengeSignatureBase64: "c2lnbmF0dXJl",
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
	challenge          challengeResponse
	lastChallenge      GatewayChallengeRequest
	lastEphemeral      GatewayEphemeralRequest
	lastStoredRegister GatewayStoredRegistrationRequest
}

type httpPKCS12Fixture struct {
	path string
}

func (s *stubGateway) CreateChallenge(_ context.Context, req GatewayChallengeRequest) (challengeResponse, error) {
	s.mu.Lock()
	defer s.mu.Unlock()
	s.lastChallenge = req
	if s.challenge.ChallengeID != "" {
		return s.challenge, nil
	}
	return challengeResponse{
		ChallengeID: "challenge-1",
		Payload:     "cert-gateway:org-1:ephemeral::nonce:2026-04-12T00:00:00Z",
	}, nil
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
	handler := newTestHandler(&stubNPKIService{}, stubApprover{})

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
		&stubNPKIService{
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

func TestServerListsCertificatesWithPKCS12Metadata(t *testing.T) {
	root := t.TempDir()
	fixture := writeHTTPPKCS12Fixture(t, root, "protected.p12", "Protected PKCS12 Certificate", "secret-password")
	handler := newTestHandler(
		npki.NewService(npki.ServiceConfig{Roots: []string{root}}),
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
			ID          string `json:"id"`
			CommonName  string `json:"commonName"`
			SourceKind  string `json:"sourceKind"`
			StorageKind string `json:"storageKind"`
			DisplayPath string `json:"displayPath"`
			IssuerName  string `json:"issuerName"`
		} `json:"items"`
	}
	if err := json.Unmarshal(rec.Body.Bytes(), &got); err != nil {
		t.Fatalf("unmarshal response: %v", err)
	}
	if len(got.Items) != 1 {
		t.Fatalf("items len = %d, want 1", len(got.Items))
	}
	item := got.Items[0]
	if item.ID != httpPKCS12CertificateID(fixture.path) {
		t.Fatalf("id = %q, want %q", item.ID, httpPKCS12CertificateID(fixture.path))
	}
	if item.SourceKind != "pkcs12" {
		t.Fatalf("sourceKind = %q, want pkcs12", item.SourceKind)
	}
	if item.StorageKind != "pkcs12" {
		t.Fatalf("storageKind = %q, want pkcs12", item.StorageKind)
	}
	if item.DisplayPath != "protected.p12" {
		t.Fatalf("displayPath = %q, want protected.p12", item.DisplayPath)
	}
	if item.CommonName != "protected" {
		t.Fatalf("commonName = %q, want protected", item.CommonName)
	}
	if item.IssuerName != "" {
		t.Fatalf("issuerName = %q, want empty fallback metadata", item.IssuerName)
	}
}

func TestServerSkipsCorruptPKCS12Certificates(t *testing.T) {
	root := t.TempDir()
	fixture := writeHTTPPKCS12Fixture(t, root, "protected.p12", "Protected PKCS12 Certificate", "secret-password")
	if err := os.WriteFile(filepath.Join(root, "corrupt.p12"), []byte("not-a-pkcs12-file"), 0o600); err != nil {
		t.Fatalf("write corrupt fixture: %v", err)
	}
	handler := newTestHandler(
		npki.NewService(npki.ServiceConfig{Roots: []string{root}}),
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
			ID string `json:"id"`
		} `json:"items"`
	}
	if err := json.Unmarshal(rec.Body.Bytes(), &got); err != nil {
		t.Fatalf("unmarshal response: %v", err)
	}
	if len(got.Items) != 1 {
		t.Fatalf("items len = %d, want 1", len(got.Items))
	}
	if got.Items[0].ID != httpPKCS12CertificateID(fixture.path) {
		t.Fatalf("id = %q, want only protected pkcs12 item", got.Items[0].ID)
	}
}

func TestServerRejectsInvalidSignRequests(t *testing.T) {
	handler := newTestHandler(&stubNPKIService{}, stubApprover{})
	sessionToken := openSession(t, handler)

	testCases := []struct {
		name string
		body string
	}{
		{name: "missing certificate id", body: `{"challenge":"nonce-1","password":"secret"}`},
		{name: "missing password", body: `{"challenge":"nonce-1","certificateId":"cert-1"}`},
		{name: "missing challenge", body: `{"password":"secret","certificateId":"cert-1"}`},
		{name: "whitespace only password", body: `{"challenge":"nonce-1","certificateId":"cert-1","password":"   "}`},
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
	handler := newTestHandler(&stubNPKIService{}, stubApprover{})
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

func TestServerReturnsBundleOnPKCS12Sign(t *testing.T) {
	root := t.TempDir()
	fixture := writeHTTPPKCS12Fixture(t, root, "protected.p12", "Protected PKCS12 Certificate", "secret-password")
	handler := newTestHandler(
		npki.NewService(npki.ServiceConfig{Roots: []string{root}}),
		stubApprover{},
	)
	sessionToken := openSession(t, handler)

	req := httptest.NewRequest(http.MethodPost, "/sign", strings.NewReader(`{
		"challenge":"challenge-1",
		"certificateId":"`+httpPKCS12CertificateID(fixture.path)+`",
		"password":"secret-password"
	}`))
	req.Header.Set("Content-Type", "application/json")
	req.Header.Set("Origin", "http://127.0.0.1:4173")
	req.Header.Set("X-Agent-Session", sessionToken)
	rec := httptest.NewRecorder()
	handler.ServeHTTP(rec, req)

	if rec.Code != http.StatusOK {
		t.Fatalf("status = %d, want %d: %s", rec.Code, http.StatusOK, rec.Body.String())
	}

	var got struct {
		CertificateID            string `json:"certificateId"`
		ChallengeSignatureBase64 string `json:"challengeSignatureBase64"`
		Bundle                   struct {
			CommonName        string `json:"commonName"`
			SignCertDerBase64 string `json:"signCertDerBase64"`
			SignPriKeyBase64  string `json:"signPriKeyBase64"`
			KMCertDerBase64   string `json:"kmCertDerBase64"`
			KMPriKeyBase64    string `json:"kmPriKeyBase64"`
		} `json:"bundle"`
	}
	if err := json.Unmarshal(rec.Body.Bytes(), &got); err != nil {
		t.Fatalf("unmarshal response: %v", err)
	}
	if got.CertificateID != httpPKCS12CertificateID(fixture.path) {
		t.Fatalf("certificateId = %q, want %q", got.CertificateID, httpPKCS12CertificateID(fixture.path))
	}
	if got.ChallengeSignatureBase64 == "" {
		t.Fatal("challengeSignatureBase64 is empty")
	}
	if got.Bundle.SignCertDerBase64 == "" || got.Bundle.SignPriKeyBase64 == "" {
		t.Fatalf("bundle = %#v, want sign fields populated", got.Bundle)
	}
	if got.Bundle.KMCertDerBase64 != "" || got.Bundle.KMPriKeyBase64 != "" {
		t.Fatalf("bundle km fields = %#v, want empty", got.Bundle)
	}
	if got.Bundle.CommonName != "Protected PKCS12 Certificate" {
		t.Fatalf("commonName = %q, want Protected PKCS12 Certificate", got.Bundle.CommonName)
	}

	signCertDER, err := base64.StdEncoding.DecodeString(got.Bundle.SignCertDerBase64)
	if err != nil {
		t.Fatalf("decode sign cert: %v", err)
	}
	signCert, err := x509.ParseCertificate(signCertDER)
	if err != nil {
		t.Fatalf("parse sign cert: %v", err)
	}
	signature, err := base64.StdEncoding.DecodeString(got.ChallengeSignatureBase64)
	if err != nil {
		t.Fatalf("decode challenge signature: %v", err)
	}
	digest := sha256.Sum256([]byte("challenge-1"))
	if err := rsa.VerifyPKCS1v15(signCert.PublicKey.(*rsa.PublicKey), crypto.SHA256, digest[:], signature); err != nil {
		t.Fatalf("verify challenge signature: %v", err)
	}
}

func TestServerPreservesPasswordWhitespaceOnSign(t *testing.T) {
	npkiSvc := &stubNPKIService{}
	handler := newTestHandler(npkiSvc, stubApprover{})
	sessionToken := openSession(t, handler)

	req := httptest.NewRequest(http.MethodPost, "/sign", strings.NewReader(`{
		"challenge":"challenge-1",
		"certificateId":"cert-1",
		"password":"  secret-password  "
	}`))
	req.Header.Set("Content-Type", "application/json")
	req.Header.Set("Origin", "http://127.0.0.1:4173")
	req.Header.Set("X-Agent-Session", sessionToken)
	rec := httptest.NewRecorder()
	handler.ServeHTTP(rec, req)

	if rec.Code != http.StatusOK {
		t.Fatalf("status = %d, want %d", rec.Code, http.StatusOK)
	}
	if npkiSvc.lastSignReq.Password != "  secret-password  " {
		t.Fatalf("password = %q, want surrounding whitespace preserved", npkiSvc.lastSignReq.Password)
	}
}

func TestServerRejectsOversizedSignBody(t *testing.T) {
	handler := newTestHandler(&stubNPKIService{}, stubApprover{})
	sessionToken := openSession(t, handler)
	oversizedPassword := strings.Repeat("x", int(maxJSONBodyBytes)+1)

	req := httptest.NewRequest(http.MethodPost, "/sign", strings.NewReader(`{
		"challenge":"challenge-1",
		"certificateId":"cert-1",
		"password":"`+oversizedPassword+`"
	}`))
	req.Header.Set("Content-Type", "application/json")
	req.Header.Set("Origin", "http://127.0.0.1:4173")
	req.Header.Set("X-Agent-Session", sessionToken)
	rec := httptest.NewRecorder()
	handler.ServeHTTP(rec, req)

	if rec.Code != http.StatusBadRequest {
		t.Fatalf("status = %d, want %d", rec.Code, http.StatusBadRequest)
	}
}

func TestSummarizeGatewayErrorBodyMarshalsStructuredValues(t *testing.T) {
	body := []byte(`{"error":{"code":"gateway_denied","message":"denied"}}`)

	got := summarizeGatewayErrorBody(body)

	if got != `{"code":"gateway_denied","message":"denied"}` {
		t.Fatalf("summary = %q", got)
	}
}

func TestSummarizeGatewayErrorValueDropsNonJSONValues(t *testing.T) {
	got := summarizeGatewayErrorValue(map[string]any{
		"channel": make(chan int),
	})

	if got != "" {
		t.Fatalf("summary = %q, want empty string", got)
	}
}

func TestServerApproveEphemeralRun(t *testing.T) {
	gateway := &stubGateway{
		challenge: challengeResponse{
			ChallengeID: "challenge-ephemeral-1",
			Payload:     "cert-gateway:org-1:ephemeral:FAC-001:nonce-1:2026-04-12T00:00:00Z",
		},
	}
	npkiSvc := &stubNPKIService{
		certificates: []npki.CertificateMetadata{{
			ID:         "cert-1",
			CommonName: "Test Certificate",
			BusinessNo: "2768603291",
			NotAfter:   time.Date(2027, 4, 9, 0, 0, 0, 0, time.UTC),
		}},
	}
	handler := newTestHandler(
		npkiSvc,
		stubApprover{decision: ApprovalDecision{
			CertificateID: "cert-1",
			Password:      []byte("secret-password"),
		}},
		gateway,
	)
	sessionToken := openSession(t, handler)

	req := httptest.NewRequest(http.MethodPost, "/approve-ephemeral-run", strings.NewReader(`{
		"gatewayBaseUrl":"http://gateway.local",
		"organizationId":"org-1",
		"businessNo":"2768603291",
		"bearerToken":"token-1",
		"activeOrganizationId":"org-1",
		"facilityCode":"FAC-001",
		"startMonth":"2026-01",
		"endMonth":"2026-03",
		"mode":"delta"
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
	if npkiSvc.lastSignReq.Challenge != "cert-gateway:org-1:ephemeral:FAC-001:nonce-1:2026-04-12T00:00:00Z" {
		t.Fatalf("challenge = %q", npkiSvc.lastSignReq.Challenge)
	}
	if string(npkiSvc.lastSignReq.PasswordBytes) != "secret-password" {
		t.Fatalf("passwordBytes = %q, want secret-password", npkiSvc.lastSignReq.PasswordBytes)
	}
	if gateway.lastEphemeral.Password != "secret-password" {
		t.Fatalf("password = %q, want secret-password", gateway.lastEphemeral.Password)
	}
	if gateway.lastEphemeral.ChallengeID != "challenge-ephemeral-1" {
		t.Fatalf("challengeId = %q, want challenge-ephemeral-1", gateway.lastEphemeral.ChallengeID)
	}
	if gateway.lastEphemeral.ChallengePayload != "cert-gateway:org-1:ephemeral:FAC-001:nonce-1:2026-04-12T00:00:00Z" {
		t.Fatalf("challengePayload = %q", gateway.lastEphemeral.ChallengePayload)
	}
	if gateway.lastEphemeral.ChallengeSignatureBase64 == "" {
		t.Fatal("challengeSignatureBase64 is empty")
	}
	if gateway.lastChallenge.FacilityCode != "FAC-001" {
		t.Fatalf("challenge facilityCode = %q, want FAC-001", gateway.lastChallenge.FacilityCode)
	}
	if gateway.lastEphemeral.FacilityCode != "FAC-001" {
		t.Fatalf("facilityCode = %q, want FAC-001", gateway.lastEphemeral.FacilityCode)
	}
	if gateway.lastEphemeral.StartMonth != "2026-01" {
		t.Fatalf("startMonth = %q, want 2026-01", gateway.lastEphemeral.StartMonth)
	}
	if gateway.lastEphemeral.EndMonth != "2026-03" {
		t.Fatalf("endMonth = %q, want 2026-03", gateway.lastEphemeral.EndMonth)
	}
	if gateway.lastEphemeral.Mode != "delta" {
		t.Fatalf("mode = %q, want delta", gateway.lastEphemeral.Mode)
	}
}

func TestServerApproveEphemeralRunAllowsMissingBusinessNumberWhenCertificateProvidesIt(t *testing.T) {
	gateway := &stubGateway{
		challenge: challengeResponse{
			ChallengeID: "challenge-ephemeral-cert-business-no",
			Payload:     "cert-gateway:org-1:ephemeral::nonce-1:2026-04-12T00:00:00Z",
		},
	}
	npkiSvc := &stubNPKIService{
		certificates: []npki.CertificateMetadata{{
			ID:         "cert-1",
			CommonName: "Test Certificate",
			BusinessNo: "2768603291",
			NotAfter:   time.Date(2027, 4, 9, 0, 0, 0, 0, time.UTC),
		}},
	}
	handler := newTestHandler(
		npkiSvc,
		stubApprover{decision: ApprovalDecision{
			CertificateID: "cert-1",
			Password:      []byte("secret-password"),
		}},
		gateway,
	)
	sessionToken := openSession(t, handler)

	req := httptest.NewRequest(http.MethodPost, "/approve-ephemeral-run", strings.NewReader(`{
		"gatewayBaseUrl":"http://gateway.local",
		"organizationId":"org-1"
	}`))
	req.Header.Set("Content-Type", "application/json")
	req.Header.Set("Origin", "http://127.0.0.1:4173")
	req.Header.Set("X-Agent-Session", sessionToken)
	rec := httptest.NewRecorder()
	handler.ServeHTTP(rec, req)

	if rec.Code != http.StatusAccepted {
		t.Fatalf("status = %d, want %d: %s", rec.Code, http.StatusAccepted, rec.Body.String())
	}
	if npkiSvc.lastSignReq.BusinessNo != "" {
		t.Fatalf("sign businessNo = %q, want empty when request omits it", npkiSvc.lastSignReq.BusinessNo)
	}
	if gateway.lastEphemeral.BusinessNo != "" {
		t.Fatalf("gateway businessNo = %q, want empty when request omits it", gateway.lastEphemeral.BusinessNo)
	}
	if gateway.lastEphemeral.Bundle.BusinessNo != "2768603291" {
		t.Fatalf("bundle businessNo = %q, want 2768603291 from certificate metadata", gateway.lastEphemeral.Bundle.BusinessNo)
	}
}

func TestServerApproveStoredRegistration(t *testing.T) {
	gateway := &stubGateway{
		challenge: challengeResponse{
			ChallengeID: "challenge-stored-1",
			Payload:     "cert-gateway:org-1:stored::nonce-2:2026-04-12T00:00:00Z",
		},
	}
	npkiSvc := &stubNPKIService{
		certificates: []npki.CertificateMetadata{{
			ID:         "cert-1",
			CommonName: "Test Certificate",
			BusinessNo: "2768603291",
			NotAfter:   time.Date(2027, 4, 9, 0, 0, 0, 0, time.UTC),
		}},
	}
	handler := newTestHandler(
		npkiSvc,
		stubApprover{decision: ApprovalDecision{
			CertificateID: "cert-1",
			Password:      []byte("secret-password"),
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
	if npkiSvc.lastSignReq.Challenge != "cert-gateway:org-1:stored::nonce-2:2026-04-12T00:00:00Z" {
		t.Fatalf("challenge = %q", npkiSvc.lastSignReq.Challenge)
	}
	if gateway.lastStoredRegister.Password != "secret-password" {
		t.Fatalf("password = %q, want secret-password", gateway.lastStoredRegister.Password)
	}
	if gateway.lastStoredRegister.ChallengeSignatureBase64 == "" {
		t.Fatal("challengeSignatureBase64 is empty")
	}
}

func TestServerRejectsGatewayBaseURLMismatch(t *testing.T) {
	handler := newTestHandler(
		&stubNPKIService{
			certificates: []npki.CertificateMetadata{{
				ID:         "cert-1",
				CommonName: "Test Certificate",
				BusinessNo: "2768603291",
				NotAfter:   time.Date(2027, 4, 9, 0, 0, 0, 0, time.UTC),
			}},
		},
		stubApprover{decision: ApprovalDecision{
			CertificateID: "cert-1",
			Password:      []byte("secret-password"),
		}},
		&stubGateway{},
	)
	sessionToken := openSession(t, handler)

	req := httptest.NewRequest(http.MethodPost, "/approve-ephemeral-run", strings.NewReader(`{
		"gatewayBaseUrl":"http://attacker.example",
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

	if rec.Code != http.StatusBadRequest {
		t.Fatalf("status = %d, want %d", rec.Code, http.StatusBadRequest)
	}
	if !strings.Contains(rec.Body.String(), "configured gateway backend") {
		t.Fatalf("body = %q, want configured gateway backend mismatch", rec.Body.String())
	}
}

func TestServerRejectsOversizedApprovalBody(t *testing.T) {
	handler := newTestHandler(
		&stubNPKIService{},
		stubApprover{decision: ApprovalDecision{
			CertificateID: "cert-1",
			Password:      []byte("secret-password"),
		}},
		&stubGateway{},
	)
	sessionToken := openSession(t, handler)
	oversizedBusinessNo := strings.Repeat("1", int(maxJSONBodyBytes)+1)

	req := httptest.NewRequest(http.MethodPost, "/approve-ephemeral-run", strings.NewReader(`{
		"gatewayBaseUrl":"http://gateway.local",
		"organizationId":"org-1",
		"businessNo":"`+oversizedBusinessNo+`"
	}`))
	req.Header.Set("Content-Type", "application/json")
	req.Header.Set("Origin", "http://127.0.0.1:4173")
	req.Header.Set("X-Agent-Session", sessionToken)
	rec := httptest.NewRecorder()
	handler.ServeHTTP(rec, req)

	if rec.Code != http.StatusBadRequest {
		t.Fatalf("status = %d, want %d", rec.Code, http.StatusBadRequest)
	}
}

func TestServerApproveEphemeralRunRejectsInvalidJSON(t *testing.T) {
	handler := newTestHandler(
		&stubNPKIService{},
		stubApprover{decision: ApprovalDecision{
			CertificateID: "cert-1",
			Password:      []byte("secret-password"),
		}},
		&stubGateway{},
	)
	sessionToken := openSession(t, handler)

	req := httptest.NewRequest(http.MethodPost, "/approve-ephemeral-run", strings.NewReader(`{`))
	req.Header.Set("Content-Type", "application/json")
	req.Header.Set("Origin", "http://127.0.0.1:4173")
	req.Header.Set("X-Agent-Session", sessionToken)
	rec := httptest.NewRecorder()
	handler.ServeHTTP(rec, req)

	if rec.Code != http.StatusBadRequest {
		t.Fatalf("status = %d, want %d", rec.Code, http.StatusBadRequest)
	}
}

func TestServerApproveEphemeralRunRequiresSession(t *testing.T) {
	handler := newTestHandler(
		&stubNPKIService{},
		stubApprover{decision: ApprovalDecision{
			CertificateID: "cert-1",
			Password:      []byte("secret-password"),
		}},
		&stubGateway{},
	)

	req := httptest.NewRequest(http.MethodPost, "/approve-ephemeral-run", strings.NewReader(`{
		"gatewayBaseUrl":"http://gateway.local",
		"organizationId":"org-1",
		"businessNo":"2768603291"
	}`))
	req.Header.Set("Content-Type", "application/json")
	req.Header.Set("Origin", "http://127.0.0.1:4173")
	rec := httptest.NewRecorder()
	handler.ServeHTTP(rec, req)

	if rec.Code != http.StatusUnauthorized {
		t.Fatalf("status = %d, want %d", rec.Code, http.StatusUnauthorized)
	}
}

func TestServerApproveEphemeralRunHandlesUserRejection(t *testing.T) {
	handler := newTestHandler(&stubNPKIService{}, stubApprover{err: ErrApprovalRejected})
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
	handler := newTestHandler(&stubNPKIService{}, stubApprover{})

	req := httptest.NewRequest(http.MethodGet, "/certificates", nil)
	req.Header.Set("Origin", "http://127.0.0.1:4173")
	rec := httptest.NewRecorder()
	handler.ServeHTTP(rec, req)

	if rec.Code != http.StatusUnauthorized {
		t.Fatalf("status = %d, want %d", rec.Code, http.StatusUnauthorized)
	}
}

func TestServerRejectsDisallowedOrigin(t *testing.T) {
	handler := newTestHandler(&stubNPKIService{}, stubApprover{})

	req := httptest.NewRequest(http.MethodPost, "/sessions/open", nil)
	req.Header.Set("Origin", "http://evil.example")
	rec := httptest.NewRecorder()
	handler.ServeHTTP(rec, req)

	if rec.Code != http.StatusForbidden {
		t.Fatalf("status = %d, want %d", rec.Code, http.StatusForbidden)
	}
}

func TestServerHandlesCORSPreflight(t *testing.T) {
	handler := newTestHandler(&stubNPKIService{}, stubApprover{})

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

func TestHTTPGatewayExecutorIncludesErrorBody(t *testing.T) {
	backend := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		w.Header().Set("Content-Type", "application/json")
		w.WriteHeader(http.StatusForbidden)
		_, _ = w.Write([]byte(`{"message":"invalid challenge payload"}`))
	}))
	defer backend.Close()

	executor := httpGatewayExecutor{client: backend.Client()}
	_, err := executor.postJSON(context.Background(), gatewayRequest{
		BaseURL: backend.URL,
		Path:    "/api/v1/fail",
		Body:    map[string]any{"mode": "stored"},
	})
	if err == nil {
		t.Fatal("expected postJSON error")
	}
	if !strings.Contains(err.Error(), "status 403") {
		t.Fatalf("error = %q, want status code included", err)
	}
	if !strings.Contains(err.Error(), "invalid challenge payload") {
		t.Fatalf("error = %q, want response body context included", err)
	}
}

func writeHTTPPKCS12Fixture(t *testing.T, root string, fileName string, commonName string, password string) httpPKCS12Fixture {
	t.Helper()

	path := filepath.Join(root, fileName)
	if err := os.MkdirAll(filepath.Dir(path), 0o755); err != nil {
		t.Fatalf("mkdir %s: %v", filepath.Dir(path), err)
	}

	key, err := rsa.GenerateKey(rand.Reader, 1024)
	if err != nil {
		t.Fatalf("generate key: %v", err)
	}
	certDER := createHTTPCertificateDER(t, key, commonName, "276-86-03291", time.Date(2027, 4, 9, 0, 0, 0, 0, time.UTC))
	cert, err := x509.ParseCertificate(certDER)
	if err != nil {
		t.Fatalf("parse certificate: %v", err)
	}
	pfxDER, err := pkcs12.Encode(rand.Reader, key, cert, nil, password)
	if err != nil {
		t.Fatalf("encode pkcs12: %v", err)
	}
	if err := os.WriteFile(path, pfxDER, 0o600); err != nil {
		t.Fatalf("write pkcs12 fixture: %v", err)
	}
	return httpPKCS12Fixture{path: path}
}

func createHTTPCertificateDER(t *testing.T, key *rsa.PrivateKey, commonName string, businessNo string, validUntil time.Time) []byte {
	t.Helper()

	template := &x509.Certificate{
		SerialNumber: big.NewInt(1),
		Subject: pkix.Name{
			CommonName: commonName,
			ExtraNames: []pkix.AttributeTypeAndValue{{
				Type:  asn1.ObjectIdentifier{2, 5, 4, 5},
				Value: businessNo,
			}},
		},
		NotBefore:             validUntil.Add(-24 * time.Hour),
		NotAfter:              validUntil,
		KeyUsage:              x509.KeyUsageDigitalSignature | x509.KeyUsageKeyEncipherment,
		BasicConstraintsValid: true,
	}
	der, err := x509.CreateCertificate(rand.Reader, template, template, &key.PublicKey, key)
	if err != nil {
		t.Fatalf("create certificate: %v", err)
	}
	return der
}

func httpPKCS12CertificateID(path string) string {
	if resolved, err := filepath.EvalSymlinks(path); err == nil {
		path = resolved
	}
	return "pkcs12:" + filepath.Clean(path)
}

func TestHTTPGatewayExecutorRejectsOversizedResponseBody(t *testing.T) {
	backend := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		w.Header().Set("Content-Type", "application/json")
		w.WriteHeader(http.StatusOK)
		_, _ = w.Write([]byte(strings.Repeat("x", int(maxJSONBodyBytes)+1)))
	}))
	defer backend.Close()

	executor := httpGatewayExecutor{client: backend.Client()}
	_, err := executor.postJSON(context.Background(), gatewayRequest{
		BaseURL: backend.URL,
		Path:    "/api/v1/ok",
		Body:    map[string]any{"mode": "stored"},
	})
	if err == nil {
		t.Fatal("expected postJSON error")
	}
	if !strings.Contains(err.Error(), "gateway response body exceeds") {
		t.Fatalf("error = %q, want oversized body failure", err)
	}
}

func TestNewHTTPGatewayExecutorUsesLongRunningRequestTimeout(t *testing.T) {
	executor, ok := NewHTTPGatewayExecutor().(httpGatewayExecutor)
	if !ok {
		t.Fatal("expected httpGatewayExecutor implementation")
	}
	if executor.client == nil {
		t.Fatal("expected configured http client")
	}
	if executor.client.Timeout != gatewayRequestTimeout {
		t.Fatalf("timeout = %v, want %v", executor.client.Timeout, gatewayRequestTimeout)
	}
}

func newTestHandler(npkiSvc npkiService, approver stubApprover, gateways ...*stubGateway) http.Handler {
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
