package httpapi

import (
	"bytes"
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
	"errors"
	"math/big"
	"net/http"
	"net/http/httptest"
	"os"
	"path/filepath"
	"strings"
	"sync"
	"testing"
	"time"

	"desktop-agent/internal/npki"
	pkcs12 "software.sslmate.com/src/go-pkcs12"
)

var testServerConfig = Config{
	AllowedOrigins: []string{"http://127.0.0.1:4173"},
	GatewayBaseURL: "https://gateway.local",
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
	lastStoredRegister GatewayStoredRegistrationRequest
}

type stubLocalExecutor struct {
	mu            sync.Mutex
	lastEphemeral LocalEphemeralRequest
	err           error
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

func (s *stubLocalExecutor) RunEphemeral(_ context.Context, req LocalEphemeralRequest) (map[string]any, error) {
	s.mu.Lock()
	s.lastEphemeral = req
	s.mu.Unlock()
	if s.err != nil {
		return nil, s.err
	}
	return map[string]any{
		"id":             "sync-local-ephemeral-1",
		"organizationId": "org-1",
		"plantId":        "FAC-001",
		"mode":           "full",
		"status":         "completed",
		"startedAt":      nil,
		"finishedAt":     nil,
		"durationMs":     42,
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
		{name: "non gateway challenge", body: `{"challenge":"login:attacker","certificateId":"cert-1","password":"secret"}`},
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

func TestServerReturnsSignatureOnlyOnSign(t *testing.T) {
	handler := newTestHandler(&stubNPKIService{}, stubApprover{})
	sessionToken := openSession(t, handler)

	req := httptest.NewRequest(http.MethodPost, "/sign", strings.NewReader(`{
		"challenge":"cert-gateway:challenge-1",
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
		CertificateID            string `json:"certificateId"`
		ChallengeSignatureBase64 string `json:"challengeSignatureBase64"`
		Bundle                   any    `json:"bundle"`
		Password                 string `json:"password"`
		SignPriKeyBase64         string `json:"signPriKeyBase64"`
		KMPriKeyBase64           string `json:"kmPriKeyBase64"`
		Certificate              struct {
			BusinessNo        string `json:"businessNo"`
			CommonName        string `json:"commonName"`
			SignCertDerBase64 string `json:"signCertDerBase64"`
		} `json:"certificate"`
	}
	if err := json.Unmarshal(rec.Body.Bytes(), &got); err != nil {
		t.Fatalf("unmarshal response: %v", err)
	}
	if got.CertificateID != "cert-1" {
		t.Fatalf("certificateId = %q, want cert-1", got.CertificateID)
	}
	if got.ChallengeSignatureBase64 == "" {
		t.Fatal("challengeSignatureBase64 is empty")
	}
	if got.Bundle != nil || got.Password != "" || got.SignPriKeyBase64 != "" || got.KMPriKeyBase64 != "" {
		t.Fatalf("sign response leaked secret fields: %s", rec.Body.String())
	}
	if got.Certificate.BusinessNo != "2768603291" {
		t.Fatalf("businessNo = %q, want 2768603291", got.Certificate.BusinessNo)
	}
}

func TestServerReturnsSignatureOnlyOnPKCS12Sign(t *testing.T) {
	root := t.TempDir()
	fixture := writeHTTPPKCS12Fixture(t, root, "protected.p12", "Protected PKCS12 Certificate", "secret-password")
	handler := newTestHandler(
		npki.NewService(npki.ServiceConfig{Roots: []string{root}}),
		stubApprover{},
	)
	sessionToken := openSession(t, handler)

	body, err := json.Marshal(map[string]string{
		"challenge":     "cert-gateway:challenge-1",
		"certificateId": httpPKCS12CertificateID(fixture.path),
		"password":      "secret-password",
	})
	if err != nil {
		t.Fatalf("marshal request: %v", err)
	}

	req := httptest.NewRequest(http.MethodPost, "/sign", strings.NewReader(string(body)))
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
		Bundle                   any    `json:"bundle"`
		Certificate              struct {
			CommonName        string `json:"commonName"`
			SignCertDerBase64 string `json:"signCertDerBase64"`
		} `json:"certificate"`
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
	if got.Bundle != nil {
		t.Fatalf("bundle leaked in sign response: %#v", got.Bundle)
	}
	if got.Certificate.SignCertDerBase64 == "" {
		t.Fatal("certificate sign cert is empty")
	}
	if got.Certificate.CommonName != "Protected PKCS12 Certificate" {
		t.Fatalf("commonName = %q, want Protected PKCS12 Certificate", got.Certificate.CommonName)
	}

	signCertDER, err := base64.StdEncoding.DecodeString(got.Certificate.SignCertDerBase64)
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
	digest := sha256.Sum256([]byte("cert-gateway:challenge-1"))
	if err := rsa.VerifyPKCS1v15(signCert.PublicKey.(*rsa.PublicKey), crypto.SHA256, digest[:], signature); err != nil {
		t.Fatalf("verify challenge signature: %v", err)
	}
}

func TestServerPreservesPasswordWhitespaceOnSign(t *testing.T) {
	npkiSvc := &stubNPKIService{}
	handler := newTestHandler(npkiSvc, stubApprover{})
	sessionToken := openSession(t, handler)

	req := httptest.NewRequest(http.MethodPost, "/sign", strings.NewReader(`{
		"challenge":"cert-gateway:challenge-1",
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
		"challenge":"cert-gateway:challenge-1",
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

func TestChallengeMonthBoundsPreservesChallengeValuesForFullSync(t *testing.T) {
	startMonth, endMonth := challengeMonthBounds(" 2026-02 ", " 2026-04 ")

	if startMonth != "2026-02" || endMonth != "2026-04" {
		t.Fatalf("bounds = %q/%q, want 2026-02/2026-04", startMonth, endMonth)
	}
}

func TestServerApproveEphemeralRun(t *testing.T) {
	gateway := &stubGateway{
		challenge: challengeResponse{
			ChallengeID:  "challenge-ephemeral-1",
			Payload:      "cert-gateway:org-1:ephemeral:FAC-BOUND:nonce-1:2026-04-12T00:00:00Z",
			SyncMode:     "full",
			FacilityCode: "FAC-BOUND",
		},
	}
	local := &stubLocalExecutor{}
	npkiSvc := &stubNPKIService{
		certificates: []npki.CertificateMetadata{{
			ID:                "cert-1",
			CommonName:        "Test Certificate",
			BusinessNo:        "2768603291",
			NotAfter:          time.Date(2027, 4, 9, 0, 0, 0, 0, time.UTC),
			SignCertSHA256Hex: "sha256-cert-1",
		}},
	}
	handler := newTestHandler(
		npkiSvc,
		stubApprover{decision: ApprovalDecision{
			CertificateID: "cert-1",
			Password:      []byte("secret-password"),
		}},
		gateway,
		local,
	)
	sessionToken := openSession(t, handler)

	req := httptest.NewRequest(http.MethodPost, "/approve-ephemeral-run", strings.NewReader(`{
		"gatewayBaseUrl":"https://gateway.local",
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
	if npkiSvc.lastSignReq.Challenge != "cert-gateway:org-1:ephemeral:FAC-BOUND:nonce-1:2026-04-12T00:00:00Z" {
		t.Fatalf("challenge = %q", npkiSvc.lastSignReq.Challenge)
	}
	if string(npkiSvc.lastSignReq.PasswordBytes) != "secret-password" {
		t.Fatalf("passwordBytes = %q, want secret-password", npkiSvc.lastSignReq.PasswordBytes)
	}
	if local.lastEphemeral.Password != "secret-password" {
		t.Fatalf("local password = %q, want secret-password", local.lastEphemeral.Password)
	}
	if local.lastEphemeral.ChallengeID != "challenge-ephemeral-1" {
		t.Fatalf("challengeId = %q, want challenge-ephemeral-1", local.lastEphemeral.ChallengeID)
	}
	if local.lastEphemeral.ChallengePayload != "cert-gateway:org-1:ephemeral:FAC-BOUND:nonce-1:2026-04-12T00:00:00Z" {
		t.Fatalf("challengePayload = %q", local.lastEphemeral.ChallengePayload)
	}
	if local.lastEphemeral.ChallengeSignatureBase64 == "" {
		t.Fatal("challengeSignatureBase64 is empty")
	}
	if gateway.lastChallenge.FacilityCode != "FAC-001" {
		t.Fatalf("challenge facilityCode = %q, want FAC-001", gateway.lastChallenge.FacilityCode)
	}
	if gateway.lastChallenge.Origin != "http://127.0.0.1:4173" {
		t.Fatalf("challenge origin = %q", gateway.lastChallenge.Origin)
	}
	if gateway.lastChallenge.BusinessNo != "2768603291" {
		t.Fatalf("challenge businessNo = %q", gateway.lastChallenge.BusinessNo)
	}
	if gateway.lastChallenge.SyncMode != "delta" {
		t.Fatalf("challenge syncMode = %q, want delta", gateway.lastChallenge.SyncMode)
	}
	if gateway.lastChallenge.StartMonth != "2026-01" || gateway.lastChallenge.EndMonth != "2026-03" {
		t.Fatalf("challenge date range = %q/%q", gateway.lastChallenge.StartMonth, gateway.lastChallenge.EndMonth)
	}
	if gateway.lastChallenge.SignCertSHA256Hex != "sha256-cert-1" {
		t.Fatalf("challenge sign cert fingerprint = %q", gateway.lastChallenge.SignCertSHA256Hex)
	}
	if local.lastEphemeral.FacilityCode != "FAC-BOUND" {
		t.Fatalf("facilityCode = %q, want FAC-BOUND", local.lastEphemeral.FacilityCode)
	}
	if local.lastEphemeral.StartMonth != "" {
		t.Fatalf("startMonth = %q, want empty for challenge full sync", local.lastEphemeral.StartMonth)
	}
	if local.lastEphemeral.EndMonth != "" {
		t.Fatalf("endMonth = %q, want empty for challenge full sync", local.lastEphemeral.EndMonth)
	}
	if local.lastEphemeral.Mode != "full" {
		t.Fatalf("mode = %q, want full", local.lastEphemeral.Mode)
	}
}

func TestServerApproveEphemeralRunDoesNotFallbackToRequestScopeWhenChallengeOmitsScope(t *testing.T) {
	gateway := &stubGateway{
		challenge: challengeResponse{
			ChallengeID: "challenge-ephemeral-unscoped",
			Payload:     "cert-gateway:org-1:ephemeral::nonce-1:2026-04-12T00:00:00Z",
		},
	}
	local := &stubLocalExecutor{}
	npkiSvc := &stubNPKIService{
		certificates: []npki.CertificateMetadata{{
			ID:                "cert-1",
			CommonName:        "Test Certificate",
			BusinessNo:        "2768603291",
			NotAfter:          time.Date(2027, 4, 9, 0, 0, 0, 0, time.UTC),
			SignCertSHA256Hex: "sha256-cert-1",
		}},
	}
	handler := newTestHandler(
		npkiSvc,
		stubApprover{decision: ApprovalDecision{
			CertificateID: "cert-1",
			Password:      []byte("secret-password"),
		}},
		gateway,
		local,
	)
	sessionToken := openSession(t, handler)

	req := httptest.NewRequest(http.MethodPost, "/approve-ephemeral-run", strings.NewReader(`{
		"gatewayBaseUrl":"https://gateway.local",
		"organizationId":"org-1",
		"businessNo":"2768603291",
		"bearerToken":"token-1",
		"activeOrganizationId":"org-1",
		"facilityCode":"FAC-CALLER",
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
	if gateway.lastChallenge.FacilityCode != "FAC-CALLER" {
		t.Fatalf("challenge facilityCode = %q, want FAC-CALLER", gateway.lastChallenge.FacilityCode)
	}
	if gateway.lastChallenge.SyncMode != "delta" {
		t.Fatalf("challenge syncMode = %q, want delta", gateway.lastChallenge.SyncMode)
	}
	if local.lastEphemeral.FacilityCode != "" {
		t.Fatalf("facilityCode = %q, want empty challenge-authorized scope", local.lastEphemeral.FacilityCode)
	}
	if local.lastEphemeral.Mode != "" {
		t.Fatalf("mode = %q, want empty challenge-authorized scope", local.lastEphemeral.Mode)
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
		&stubLocalExecutor{},
	)
	sessionToken := openSession(t, handler)

	req := httptest.NewRequest(http.MethodPost, "/approve-ephemeral-run", strings.NewReader(`{
		"gatewayBaseUrl":"https://gateway.local",
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
}

func TestServerApproveEphemeralRunRejectsCertificateBusinessNumberMismatch(t *testing.T) {
	gateway := &stubGateway{
		challenge: challengeResponse{
			ChallengeID: "challenge-ephemeral-cert-business-mismatch",
			Payload:     "cert-gateway:org-1:ephemeral::nonce-1:2026-04-12T00:00:00Z",
		},
	}
	local := &stubLocalExecutor{}
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
		gateway,
		local,
	)
	sessionToken := openSession(t, handler)

	req := httptest.NewRequest(http.MethodPost, "/approve-ephemeral-run", strings.NewReader(`{
		"gatewayBaseUrl":"https://gateway.local",
		"organizationId":"org-1",
		"businessNo":"123-45-67890"
	}`))
	req.Header.Set("Content-Type", "application/json")
	req.Header.Set("Origin", "http://127.0.0.1:4173")
	req.Header.Set("X-Agent-Session", sessionToken)
	rec := httptest.NewRecorder()
	handler.ServeHTTP(rec, req)

	if rec.Code != http.StatusBadRequest {
		t.Fatalf("status = %d, want %d: %s", rec.Code, http.StatusBadRequest, rec.Body.String())
	}
	if local.lastEphemeral.ChallengeID != "" {
		t.Fatalf("local executor was called with challenge %q", local.lastEphemeral.ChallengeID)
	}
}

func TestServerApproveEphemeralRunAllowsMissingCertificateMetadataBusinessNumberWhenSignedBundleMatches(t *testing.T) {
	gateway := &stubGateway{
		challenge: challengeResponse{
			ChallengeID: "challenge-ephemeral-cert-business-missing",
			Payload:     "cert-gateway:org-1:ephemeral::nonce-1:2026-04-12T00:00:00Z",
		},
	}
	local := &stubLocalExecutor{}
	handler := newTestHandler(
		&stubNPKIService{
			certificates: []npki.CertificateMetadata{{
				ID:         "cert-1",
				CommonName: "Test Certificate",
				BusinessNo: "",
				NotAfter:   time.Date(2027, 4, 9, 0, 0, 0, 0, time.UTC),
			}},
		},
		stubApprover{decision: ApprovalDecision{
			CertificateID: "cert-1",
			Password:      []byte("secret-password"),
		}},
		gateway,
		local,
	)
	sessionToken := openSession(t, handler)

	req := httptest.NewRequest(http.MethodPost, "/approve-ephemeral-run", strings.NewReader(`{
		"gatewayBaseUrl":"https://gateway.local",
		"organizationId":"org-1",
		"businessNo":"2768603291"
	}`))
	req.Header.Set("Content-Type", "application/json")
	req.Header.Set("Origin", "http://127.0.0.1:4173")
	req.Header.Set("X-Agent-Session", sessionToken)
	rec := httptest.NewRecorder()
	handler.ServeHTTP(rec, req)

	if rec.Code != http.StatusAccepted {
		t.Fatalf("status = %d, want %d: %s", rec.Code, http.StatusAccepted, rec.Body.String())
	}
	if local.lastEphemeral.ChallengeID != "challenge-ephemeral-cert-business-missing" {
		t.Fatalf("local executor challenge = %q, want challenge-ephemeral-cert-business-missing", local.lastEphemeral.ChallengeID)
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
		"gatewayBaseUrl":"https://gateway.local",
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
		"gatewayBaseUrl":"https://attacker.example",
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

func TestServerRejectsNonLoopbackHTTPGatewayBaseURL(t *testing.T) {
	cfg := testServerConfig
	cfg.GatewayBaseURL = "http://gateway.local"
	handler := NewServer(
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
		&stubLocalExecutor{},
		cfg,
	).Handler()
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

	if rec.Code != http.StatusBadRequest {
		t.Fatalf("status = %d, want %d", rec.Code, http.StatusBadRequest)
	}
	if !strings.Contains(rec.Body.String(), "https") {
		t.Fatalf("body = %q, want https requirement", rec.Body.String())
	}
}

func TestServerAcceptsLoopbackHTTPGatewayBaseURL(t *testing.T) {
	cfg := testServerConfig
	cfg.GatewayBaseURL = "http://127.0.0.1:18080"
	handler := NewServer(
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
		&stubLocalExecutor{},
		cfg,
	).Handler()
	sessionToken := openSession(t, handler)

	req := httptest.NewRequest(http.MethodPost, "/approve-ephemeral-run", strings.NewReader(`{
		"gatewayBaseUrl":"http://127.0.0.1:18080",
		"organizationId":"org-1",
		"businessNo":"2768603291"
	}`))
	req.Header.Set("Content-Type", "application/json")
	req.Header.Set("Origin", "http://127.0.0.1:4173")
	req.Header.Set("X-Agent-Session", sessionToken)
	rec := httptest.NewRecorder()
	handler.ServeHTTP(rec, req)

	if rec.Code != http.StatusAccepted {
		t.Fatalf("status = %d, want %d: %s", rec.Code, http.StatusAccepted, rec.Body.String())
	}
}

func TestNormalizeTrustedGatewayBaseURLPreservesEncodedPath(t *testing.T) {
	normalized, err := normalizeTrustedGatewayBaseURL("https://gateway.local/base%2Ftenant/")
	if err != nil {
		t.Fatalf("normalizeTrustedGatewayBaseURL: %v", err)
	}
	if normalized != "https://gateway.local/base%2Ftenant" {
		t.Fatalf("normalized = %q, want encoded path preserved", normalized)
	}

	normalized, err = normalizeTrustedGatewayBaseURL("https://gateway.local/base%2Ftenant/%2F/")
	if err != nil {
		t.Fatalf("normalizeTrustedGatewayBaseURL with raw path: %v", err)
	}
	if normalized != "https://gateway.local/base%2Ftenant/%2F" {
		t.Fatalf("normalized = %q, want escaped path and raw path kept in sync", normalized)
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
		"gatewayBaseUrl":"https://gateway.local",
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
		"gatewayBaseUrl":"https://gateway.local",
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
		"gatewayBaseUrl":"https://gateway.local",
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

func TestHTTPGatewayExecutorEscapesOrganizationIDInPaths(t *testing.T) {
	var gotPath string
	var gotBody map[string]any
	backend := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		gotPath = r.URL.EscapedPath()
		if err := json.NewDecoder(r.Body).Decode(&gotBody); err != nil {
			t.Fatalf("decode body: %v", err)
		}
		w.Header().Set("Content-Type", "application/json")
		_, _ = w.Write([]byte(`{"challengeId":"challenge-1","payload":"payload-1","facilityCode":"FAC-BOUND"}`))
	}))
	defer backend.Close()

	executor := httpGatewayExecutor{client: backend.Client()}
	challenge, err := executor.CreateChallenge(context.Background(), GatewayChallengeRequest{
		BaseURL:           backend.URL,
		OrganizationID:    "org/../evil",
		Mode:              "ephemeral",
		SyncMode:          "delta",
		Origin:            "http://127.0.0.1:4173",
		BusinessNo:        "2768603291",
		FacilityCode:      "FAC-001",
		StartMonth:        "2026-01",
		EndMonth:          "2026-03",
		SignCertSHA256Hex: "sha256-cert-1",
	})
	if err != nil {
		t.Fatalf("create challenge: %v", err)
	}
	if challenge.FacilityCode != "FAC-BOUND" {
		t.Fatalf("challenge facilityCode = %q, want FAC-BOUND", challenge.FacilityCode)
	}
	if gotPath != "/api/v1/organizations/org%2F..%2Fevil/cert-gateway/challenges" {
		t.Fatalf("path = %q, want escaped organization id", gotPath)
	}
	if gotBody["origin"] != "http://127.0.0.1:4173" ||
		gotBody["businessNo"] != "2768603291" ||
		gotBody["syncMode"] != "delta" ||
		gotBody["facilityCode"] != "FAC-001" ||
		gotBody["startMonth"] != "2026-01" ||
		gotBody["endMonth"] != "2026-03" ||
		gotBody["signCertSha256Hex"] != "sha256-cert-1" {
		t.Fatalf("challenge body = %#v", gotBody)
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

func TestLimitedBufferWriterCapsOutputAndRecordsTruncation(t *testing.T) {
	var buf bytes.Buffer
	writer := &limitedBufferWriter{w: &buf, limit: 4}

	n, err := writer.Write([]byte("abcdef"))
	if err != nil {
		t.Fatalf("write: %v", err)
	}
	if n != 6 {
		t.Fatalf("n = %d, want 6", n)
	}
	if got := buf.String(); got != "abcd" {
		t.Fatalf("buffer = %q, want abcd", got)
	}
	if !writer.truncated {
		t.Fatal("writer.truncated = false, want true")
	}
}

func TestLocalExecutorRunErrorReportsTruncatedStderr(t *testing.T) {
	var stderr bytes.Buffer
	limit := &limitedBufferWriter{w: &stderr, limit: maxLocalExecutorStderr, truncated: true}

	err := localExecutorRunError(errors.New("exit status 1"), &stderr, limit)
	if !strings.Contains(err.Error(), "local executor stderr exceeds") {
		t.Fatalf("error = %q, want stderr truncation failure", err)
	}
	if !strings.Contains(err.Error(), "exit status 1") {
		t.Fatalf("error = %q, want original command error", err)
	}
}

func newTestHandler(npkiSvc npkiService, approver stubApprover, opts ...any) http.Handler {
	gateway := &stubGateway{}
	local := &stubLocalExecutor{}
	for _, opt := range opts {
		switch value := opt.(type) {
		case *stubGateway:
			if value != nil {
				gateway = value
			}
		case *stubLocalExecutor:
			if value != nil {
				local = value
			}
		}
	}
	return NewServer(npkiSvc, approver, gateway, local, testServerConfig).Handler()
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
