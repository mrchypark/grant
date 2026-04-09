package npki

import (
	"context"
	"errors"
	"strings"
	"time"
	"unicode"
)

var ErrNotImplemented = errors.New("npki signing is not implemented on this platform")

type CertificateMetadata struct {
	ID         string    `json:"id"`
	CommonName string    `json:"commonName"`
	BusinessNo string    `json:"businessNo"`
	NotAfter   time.Time `json:"notAfter"`
}

type SignRequest struct {
	CertificateID string `json:"certificateId"`
	Challenge     string `json:"challenge"`
	Password      string `json:"password"`
}

type SignResult struct {
	CertificateID string `json:"certificateId"`
	Challenge     string `json:"challenge"`
}

type Service struct {
	certificates []CertificateMetadata
}

func NewService(certificates []CertificateMetadata) *Service {
	return &Service{certificates: normalizeCertificates(certificates)}
}

func (s *Service) ListCertificates(context.Context) ([]CertificateMetadata, error) {
	copied := make([]CertificateMetadata, len(s.certificates))
	copy(copied, s.certificates)
	return copied, nil
}

func (s *Service) Sign(context.Context, SignRequest) (SignResult, error) {
	return SignResult{}, ErrNotImplemented
}

func normalizeCertificates(certificates []CertificateMetadata) []CertificateMetadata {
	normalized := make([]CertificateMetadata, 0, len(certificates))
	for _, cert := range certificates {
		normalized = append(normalized, CertificateMetadata{
			ID:         strings.TrimSpace(cert.ID),
			CommonName: strings.TrimSpace(cert.CommonName),
			BusinessNo: digitsOnly(cert.BusinessNo),
			NotAfter:   cert.NotAfter.UTC(),
		})
	}
	return normalized
}

func digitsOnly(value string) string {
	var b strings.Builder
	b.Grow(len(value))
	for _, r := range value {
		if unicode.IsDigit(r) {
			b.WriteRune(r)
		}
	}
	return b.String()
}
