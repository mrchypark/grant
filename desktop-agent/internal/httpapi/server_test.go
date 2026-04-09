package httpapi

import (
	"context"
	"encoding/json"
	"net/http"
	"net/http/httptest"
	"strings"
	"testing"
	"time"

	"github.com/cypark/account/desktop-agent/internal/npki"
)

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

func TestServerHealthz(t *testing.T) {
	handler := NewServer(stubNPKIService{}).Handler()

	req := httptest.NewRequest(http.MethodGet, "/healthz", nil)
	rec := httptest.NewRecorder()
	handler.ServeHTTP(rec, req)

	if rec.Code != http.StatusOK {
		t.Fatalf("status = %d, want %d", rec.Code, http.StatusOK)
	}
	if strings.TrimSpace(rec.Body.String()) != "ok" {
		t.Fatalf("body = %q, want ok", rec.Body.String())
	}
}

func TestServerListsCertificates(t *testing.T) {
	handler := NewServer(stubNPKIService{
		certificates: []npki.CertificateMetadata{
			{
				ID:         "cert-1",
				CommonName: "Test Certificate",
				BusinessNo: "2768603291",
				NotAfter:   time.Date(2027, 4, 9, 0, 0, 0, 0, time.UTC),
			},
		},
	}).Handler()

	req := httptest.NewRequest(http.MethodGet, "/certificates", nil)
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
	handler := NewServer(stubNPKIService{}).Handler()

	testCases := []struct {
		name string
		body string
	}{
		{
			name: "missing password",
			body: `{"challenge":"nonce-1","certificateId":"cert-1"}`,
		},
		{
			name: "missing challenge",
			body: `{"password":"secret","certificateId":"cert-1"}`,
		},
	}

	for _, tc := range testCases {
		t.Run(tc.name, func(t *testing.T) {
			req := httptest.NewRequest(http.MethodPost, "/sign", strings.NewReader(tc.body))
			req.Header.Set("Content-Type", "application/json")
			rec := httptest.NewRecorder()
			handler.ServeHTTP(rec, req)

			if rec.Code != http.StatusBadRequest {
				t.Fatalf("status = %d, want %d", rec.Code, http.StatusBadRequest)
			}
		})
	}
}

func TestServerReturnsBundleOnSign(t *testing.T) {
	handler := NewServer(stubNPKIService{}).Handler()

	req := httptest.NewRequest(http.MethodPost, "/sign", strings.NewReader(`{
		"challenge":"challenge-1",
		"certificateId":"cert-1",
		"password":"secret-password"
	}`))
	req.Header.Set("Content-Type", "application/json")
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
