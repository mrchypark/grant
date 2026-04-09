package npki

import (
	"context"
	"testing"
	"time"
)

func TestServiceListCertificatesNormalizesMetadata(t *testing.T) {
	service := NewService([]CertificateMetadata{
		{
			ID:         "cert-1",
			CommonName: "  Test Certificate  ",
			BusinessNo: "276-86-03291",
			NotAfter:   time.Date(2027, 4, 9, 0, 0, 0, 0, time.UTC),
		},
	})

	got, err := service.ListCertificates(context.Background())
	if err != nil {
		t.Fatalf("ListCertificates() error = %v", err)
	}
	if len(got) != 1 {
		t.Fatalf("len(got) = %d, want 1", len(got))
	}
	if got[0].CommonName != "Test Certificate" {
		t.Fatalf("commonName = %q, want Test Certificate", got[0].CommonName)
	}
	if got[0].BusinessNo != "2768603291" {
		t.Fatalf("businessNo = %q, want 2768603291", got[0].BusinessNo)
	}
}
