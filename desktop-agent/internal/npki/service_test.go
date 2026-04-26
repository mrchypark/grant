package npki

import (
	"context"
	"crypto"
	"crypto/cipher"
	"crypto/ecdsa"
	"crypto/ed25519"
	"crypto/elliptic"
	"crypto/rand"
	"crypto/rsa"
	"crypto/sha1"
	"crypto/sha256"
	"crypto/x509"
	"crypto/x509/pkix"
	"encoding/asn1"
	"encoding/base64"
	"errors"
	"math/big"
	"os"
	"path/filepath"
	"reflect"
	"slices"
	"sync/atomic"
	"testing"
	"time"

	seedcipher "github.com/deatil/go-cryptobin/cipher/seed"
	pkcs12 "software.sslmate.com/src/go-pkcs12"
)

func TestServiceListCertificatesNormalizesMetadata(t *testing.T) {
	root := t.TempDir()
	fixture := writeFixtureBundle(t, root, fixtureOptions{
		dirName:    "cert-1",
		password:   "secret-password",
		commonName: "  Test Certificate  ",
		businessNo: "276-86-03291",
		validUntil: time.Date(2027, 4, 9, 0, 0, 0, 0, time.UTC),
		vidRandom:  []byte{1, 2, 3, 4},
	})
	service := NewService(ServiceConfig{Roots: []string{root}})

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
	if got[0].ID != bundleCertificateID(fixture.path) {
		t.Fatalf("id = %q, want %q", got[0].ID, bundleCertificateID(fixture.path))
	}
}

func TestServiceListCertificatesIncludesSourceAwareBundleMetadata(t *testing.T) {
	root := t.TempDir()
	npkiFixture := writeFixtureBundle(t, filepath.Join(root, "NPKI"), fixtureOptions{
		dirName:    "cert-1",
		password:   "secret-password",
		commonName: "NPKI Certificate",
		businessNo: "276-86-03291",
		validUntil: time.Date(2027, 4, 9, 0, 0, 0, 0, time.UTC),
		vidRandom:  []byte{1, 2, 3, 4},
	})
	gpkiFixture := writeFixtureBundle(t, filepath.Join(root, "GPKI"), fixtureOptions{
		dirName:    "cert-2",
		password:   "secret-password",
		commonName: "GPKI Certificate",
		businessNo: "276-86-03291",
		validUntil: time.Date(2027, 4, 9, 0, 0, 0, 0, time.UTC),
		vidRandom:  []byte{5, 6, 7, 8},
	})

	service := NewService(ServiceConfig{Roots: []string{root}})
	got, err := service.ListCertificates(context.Background())
	if err != nil {
		t.Fatalf("ListCertificates() error = %v", err)
	}
	if len(got) != 2 {
		t.Fatalf("len(got) = %d, want 2", len(got))
	}

	byID := map[string]CertificateMetadata{}
	for _, item := range got {
		byID[item.ID] = item
	}

	for _, tc := range []struct {
		name        string
		fixture     fixtureBundle
		storageKind CertificateStorageKind
		displayPath string
	}{
		{name: "npki", fixture: npkiFixture, storageKind: CertificateStorageKindNPKI, displayPath: "NPKI/cert-1"},
		{name: "gpki", fixture: gpkiFixture, storageKind: CertificateStorageKindGPKI, displayPath: "GPKI/cert-2"},
	} {
		item, ok := byID[bundleCertificateID(tc.fixture.path)]
		if !ok {
			t.Fatalf("%s certificate missing from ListCertificates result: %#v", tc.name, got)
		}
		if item.SourceKind != "bundle" {
			t.Fatalf("%s sourceKind = %q, want bundle", tc.name, item.SourceKind)
		}
		if item.StorageKind != tc.storageKind {
			t.Fatalf("%s storageKind = %q, want %s", tc.name, item.StorageKind, tc.storageKind)
		}
		if item.DisplayPath != tc.displayPath {
			t.Fatalf("%s displayPath = %q, want %q", tc.name, item.DisplayPath, tc.displayPath)
		}
		if item.IssuerName == "" {
			t.Fatalf("%s issuerName = empty, want issuer metadata", tc.name)
		}
		if !item.HasKmMaterial {
			t.Fatalf("%s hasKmMaterial = false, want true for bundle sources", tc.name)
		}
	}
}

func TestPKCS12ListCertificatesIncludesMetadataAndSourceAwareIDs(t *testing.T) {
	root := t.TempDir()
	first := writeFixturePKCS12(t, root, fixtureOptions{
		dirName:    "user.p12",
		password:   "",
		commonName: "PKCS12 Certificate",
		businessNo: "276-86-03291",
		validUntil: time.Date(2027, 4, 9, 0, 0, 0, 0, time.UTC),
		vidRandom:  []byte{101, 102, 103, 104},
	})
	second := writeFixturePKCS12(t, root, fixtureOptions{
		dirName:    "backup.pfx",
		password:   "",
		commonName: "Backup PKCS12 Certificate",
		businessNo: "276-86-03291",
		validUntil: time.Date(2027, 4, 9, 0, 0, 0, 0, time.UTC),
		vidRandom:  []byte{105, 106, 107, 108},
	})

	service := NewService(ServiceConfig{Roots: []string{root}})
	got, err := service.ListCertificates(context.Background())
	if err != nil {
		t.Fatalf("ListCertificates() error = %v", err)
	}
	if len(got) != 2 {
		t.Fatalf("len(got) = %d, want 2", len(got))
	}

	byID := map[string]CertificateMetadata{}
	for _, item := range got {
		byID[item.ID] = item
	}

	for _, tc := range []struct {
		name        string
		fixture     fixturePKCS12
		wantCN      string
		wantDisplay string
	}{
		{name: "p12", fixture: first, wantCN: "PKCS12 Certificate", wantDisplay: "user.p12"},
		{name: "pfx", fixture: second, wantCN: "Backup PKCS12 Certificate", wantDisplay: "backup.pfx"},
	} {
		item, ok := byID[pkcs12CertificateID(tc.fixture.path)]
		if !ok {
			t.Fatalf("%s certificate missing from ListCertificates result: %#v", tc.name, got)
		}
		if item.SourceKind != CertificateSourceKindPKCS12 {
			t.Fatalf("%s sourceKind = %q, want pkcs12", tc.name, item.SourceKind)
		}
		if item.StorageKind != CertificateStorageKindPKCS12 {
			t.Fatalf("%s storageKind = %q, want pkcs12", tc.name, item.StorageKind)
		}
		if item.DisplayPath != tc.wantDisplay {
			t.Fatalf("%s displayPath = %q, want %q", tc.name, item.DisplayPath, tc.wantDisplay)
		}
		if item.CommonName != tc.wantCN {
			t.Fatalf("%s commonName = %q, want %q", tc.name, item.CommonName, tc.wantCN)
		}
		if item.IssuerName == "" {
			t.Fatalf("%s issuerName = empty, want metadata", tc.name)
		}
		if item.HasKmMaterial {
			t.Fatalf("%s hasKmMaterial = true, want false when PKCS#12 does not expose KM material", tc.name)
		}
	}
}

func TestPKCS12PasswordProtectedFileIsListedWithFallbackMetadata(t *testing.T) {
	root := t.TempDir()
	fixture := writeFixturePKCS12(t, root, fixtureOptions{
		dirName:    "protected.p12",
		password:   "secret-password",
		commonName: "Protected PKCS12 Certificate",
		businessNo: "276-86-03291",
		validUntil: time.Date(2027, 4, 9, 0, 0, 0, 0, time.UTC),
		vidRandom:  []byte{201, 202, 203, 204},
	})

	service := NewService(ServiceConfig{Roots: []string{root}})
	got, err := service.ListCertificates(context.Background())
	if err != nil {
		t.Fatalf("ListCertificates() error = %v", err)
	}
	if len(got) != 1 {
		t.Fatalf("len(got) = %d, want 1", len(got))
	}

	item := got[0]
	if item.ID != pkcs12CertificateID(fixture.path) {
		t.Fatalf("id = %q, want %q", item.ID, pkcs12CertificateID(fixture.path))
	}
	if item.SourceKind != CertificateSourceKindPKCS12 {
		t.Fatalf("sourceKind = %q, want pkcs12", item.SourceKind)
	}
	if item.StorageKind != CertificateStorageKindPKCS12 {
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
	if item.BusinessNo != "" {
		t.Fatalf("businessNo = %q, want empty fallback metadata", item.BusinessNo)
	}
	if !item.NotAfter.IsZero() {
		t.Fatalf("notAfter = %v, want zero time for fallback metadata", item.NotAfter)
	}
	if item.HasKmMaterial {
		t.Fatalf("hasKmMaterial = %v, want false for fallback metadata", item.HasKmMaterial)
	}
}

func TestPKCS12CorruptFileIsSkippedFromListCertificates(t *testing.T) {
	root := t.TempDir()
	mustWriteFile(t, filepath.Join(root, "corrupt.p12"), []byte("not-a-pkcs12-file"))

	service := NewService(ServiceConfig{Roots: []string{root}})
	got, err := service.ListCertificates(context.Background())
	if err != nil {
		t.Fatalf("ListCertificates() error = %v", err)
	}
	if len(got) != 0 {
		t.Fatalf("len(got) = %d, want 0 for corrupt pkcs12", len(got))
	}
}

func TestPKCS12UnsupportedTrustStoreIsSkippedFromListCertificates(t *testing.T) {
	root := t.TempDir()
	key, err := rsa.GenerateKey(rand.Reader, 1024)
	if err != nil {
		t.Fatalf("generate key: %v", err)
	}
	certDER := createCertificateDER(t, key, "Trust Store Certificate", "276-86-03291", time.Date(2027, 4, 9, 0, 0, 0, 0, time.UTC))
	cert, err := x509.ParseCertificate(certDER)
	if err != nil {
		t.Fatalf("parse certificate: %v", err)
	}
	trustStoreDER, err := pkcs12.EncodeTrustStore(rand.Reader, []*x509.Certificate{cert}, "")
	if err != nil {
		t.Fatalf("encode trust store: %v", err)
	}
	mustWriteFile(t, filepath.Join(root, "truststore.pfx"), trustStoreDER)

	service := NewService(ServiceConfig{Roots: []string{root}})
	got, err := service.ListCertificates(context.Background())
	if err != nil {
		t.Fatalf("ListCertificates() error = %v", err)
	}
	if len(got) != 0 {
		t.Fatalf("len(got) = %d, want 0 for unsupported pkcs12 trust store", len(got))
	}
}

func TestServiceSignLoadsBundleAndValidatesPassword(t *testing.T) {
	root := t.TempDir()
	fixture := writeFixtureBundle(t, root, fixtureOptions{
		dirName:    "cert-1",
		password:   "secret-password",
		commonName: "Test Certificate",
		businessNo: "276-86-03291",
		validUntil: time.Date(2027, 4, 9, 0, 0, 0, 0, time.UTC),
		vidRandom:  []byte{5, 6, 7, 8},
	})
	service := NewService(ServiceConfig{Roots: []string{root}})

	result, err := service.Sign(context.Background(), SignRequest{
		CertificateID: bundleCertificateID(fixture.path),
		Challenge:     "challenge-1",
		Password:      "secret-password",
	})
	if err != nil {
		t.Fatalf("Sign() error = %v", err)
	}
	if result.CertificateID != bundleCertificateID(fixture.path) {
		t.Fatalf("certificateID = %q, want %q", result.CertificateID, bundleCertificateID(fixture.path))
	}
	if result.Bundle.CommonName != "Test Certificate" {
		t.Fatalf("commonName = %q, want Test Certificate", result.Bundle.CommonName)
	}
	if result.Bundle.BusinessNo != "2768603291" {
		t.Fatalf("businessNo = %q, want 2768603291", result.Bundle.BusinessNo)
	}
	if result.Bundle.SignCertDerBase64 != fixture.signCertB64 {
		t.Fatalf("signCertDerBase64 mismatch")
	}
}

func TestServiceSignUsesSourceAwareCertificateIDForBundleSources(t *testing.T) {
	root := t.TempDir()
	fixture := writeFixtureBundle(t, filepath.Join(root, "NPKI"), fixtureOptions{
		dirName:    "cert-1",
		password:   "secret-password",
		commonName: "Test Certificate",
		businessNo: "276-86-03291",
		validUntil: time.Date(2027, 4, 9, 0, 0, 0, 0, time.UTC),
		vidRandom:  []byte{5, 6, 7, 8},
	})
	service := NewService(ServiceConfig{Roots: []string{root}})

	result, err := service.Sign(context.Background(), SignRequest{
		CertificateID: bundleCertificateID(fixture.path),
		Challenge:     "challenge-1",
		Password:      "secret-password",
	})
	if err != nil {
		t.Fatalf("Sign() error = %v", err)
	}
	if result.CertificateID != bundleCertificateID(fixture.path) {
		t.Fatalf("certificateID = %q, want %q", result.CertificateID, bundleCertificateID(fixture.path))
	}
}

func TestServiceSignRejectsWrongPassword(t *testing.T) {
	root := t.TempDir()
	fixture := writeFixtureBundle(t, root, fixtureOptions{
		dirName:    "cert-1",
		password:   "secret-password",
		commonName: "Test Certificate",
		businessNo: "276-86-03291",
		validUntil: time.Date(2027, 4, 9, 0, 0, 0, 0, time.UTC),
		vidRandom:  []byte{9, 10, 11, 12},
	})
	service := NewService(ServiceConfig{Roots: []string{root}})

	_, err := service.Sign(context.Background(), SignRequest{
		CertificateID: bundleCertificateID(fixture.path),
		Challenge:     "challenge-1",
		Password:      "wrong-password",
	})
	if !errors.Is(err, ErrInvalidPassword) {
		t.Fatalf("Sign() error = %v, want ErrInvalidPassword", err)
	}
}

func TestPKCS12SignSignsChallengeAndLeavesKmFieldsEmpty(t *testing.T) {
	root := t.TempDir()
	fixture := writeFixturePKCS12(t, root, fixtureOptions{
		dirName:    "protected.p12",
		password:   "secret-password",
		commonName: "Protected PKCS12 Certificate",
		businessNo: "276-86-03291",
		validUntil: time.Date(2027, 4, 9, 0, 0, 0, 0, time.UTC),
		vidRandom:  []byte{109, 110, 111, 112},
	})
	service := NewService(ServiceConfig{Roots: []string{root}})

	result, err := service.Sign(context.Background(), SignRequest{
		CertificateID: pkcs12CertificateID(fixture.path),
		Challenge:     "challenge-1",
		Password:      "secret-password",
	})
	if err != nil {
		t.Fatalf("Sign() error = %v", err)
	}
	if result.CertificateID != pkcs12CertificateID(fixture.path) {
		t.Fatalf("certificateID = %q, want %q", result.CertificateID, pkcs12CertificateID(fixture.path))
	}
	if result.Bundle.SignCertDerBase64 == "" || result.Bundle.SignPriKeyBase64 == "" {
		t.Fatalf("bundle = %#v, want sign fields populated", result.Bundle)
	}
	if result.Bundle.KMCertDerBase64 != "" || result.Bundle.KMPriKeyBase64 != "" {
		t.Fatalf("bundle km fields = %#v, want empty", result.Bundle)
	}
	if result.Bundle.CommonName != "Protected PKCS12 Certificate" {
		t.Fatalf("commonName = %q, want Protected PKCS12 Certificate", result.Bundle.CommonName)
	}
	signPriKeyDER, err := base64.StdEncoding.DecodeString(result.Bundle.SignPriKeyBase64)
	if err != nil {
		t.Fatalf("decode sign private key: %v", err)
	}
	pkcs8Key, err := x509.ParsePKCS8PrivateKey(signPriKeyDER)
	if err != nil {
		t.Fatalf("parse sign private key as PKCS#8: %v", err)
	}
	if _, ok := pkcs8Key.(*rsa.PrivateKey); !ok {
		t.Fatalf("parsed PKCS#8 key type = %T, want *rsa.PrivateKey", pkcs8Key)
	}

	signCertDER, err := base64.StdEncoding.DecodeString(result.Bundle.SignCertDerBase64)
	if err != nil {
		t.Fatalf("decode sign cert: %v", err)
	}
	signCert, err := x509.ParseCertificate(signCertDER)
	if err != nil {
		t.Fatalf("parse sign cert: %v", err)
	}
	signature, err := base64.StdEncoding.DecodeString(result.ChallengeSignatureBase64)
	if err != nil {
		t.Fatalf("decode challenge signature: %v", err)
	}
	digest := sha256.Sum256([]byte("challenge-1"))
	if err := rsa.VerifyPKCS1v15(signCert.PublicKey.(*rsa.PublicKey), crypto.SHA256, digest[:], signature); err != nil {
		t.Fatalf("verify challenge signature: %v", err)
	}
}

func TestPKCS12SignRejectsWrongPassword(t *testing.T) {
	root := t.TempDir()
	fixture := writeFixturePKCS12(t, root, fixtureOptions{
		dirName:    "user.p12",
		password:   "",
		commonName: "PKCS12 Certificate",
		businessNo: "276-86-03291",
		validUntil: time.Date(2027, 4, 9, 0, 0, 0, 0, time.UTC),
		vidRandom:  []byte{113, 114, 115, 116},
	})
	service := NewService(ServiceConfig{Roots: []string{root}})

	_, err := service.Sign(context.Background(), SignRequest{
		CertificateID: pkcs12CertificateID(fixture.path),
		Challenge:     "challenge-1",
		Password:      "wrong-password",
	})
	if !errors.Is(err, ErrInvalidPassword) {
		t.Fatalf("Sign() error = %v, want ErrInvalidPassword", err)
	}
}

func TestSupportedPKCS12SignerRejectsUnsupportedAlgorithms(t *testing.T) {
	rsaKey, err := rsa.GenerateKey(rand.Reader, 1024)
	if err != nil {
		t.Fatalf("generate rsa key: %v", err)
	}
	if _, err := supportedPKCS12Signer(rsaKey); err != nil {
		t.Fatalf("supportedPKCS12Signer(rsa) error = %v", err)
	}

	ecdsaKey, err := ecdsa.GenerateKey(elliptic.P256(), rand.Reader)
	if err != nil {
		t.Fatalf("generate ecdsa key: %v", err)
	}
	if _, err := supportedPKCS12Signer(ecdsaKey); err != nil {
		t.Fatalf("supportedPKCS12Signer(ecdsa) error = %v", err)
	}

	_, ed25519Key, err := ed25519.GenerateKey(rand.Reader)
	if err != nil {
		t.Fatalf("generate ed25519 key: %v", err)
	}
	if _, err := supportedPKCS12Signer(ed25519Key); err == nil {
		t.Fatal("supportedPKCS12Signer(ed25519) error = nil, want unsupported key error")
	}
}

func TestServiceBusinessNumberPrefersSubjectSerialNumberOverHeuristicDigits(t *testing.T) {
	root := t.TempDir()
	fixture := writeFixtureBundle(t, root, fixtureOptions{
		dirName:    "cert-1",
		password:   "secret-password",
		commonName: "Invoice 1234567890 Test Certificate",
		businessNo: "276-86-03291",
		validUntil: time.Date(2027, 4, 9, 0, 0, 0, 0, time.UTC),
		vidRandom:  []byte{13, 14, 15, 16},
	})
	service := NewService(ServiceConfig{Roots: []string{root}})

	result, err := service.Sign(context.Background(), SignRequest{
		CertificateID: bundleCertificateID(fixture.path),
		Challenge:     "challenge-1",
		Password:      "secret-password",
	})
	if err != nil {
		t.Fatalf("Sign() error = %v", err)
	}
	if result.Bundle.BusinessNo != "2768603291" {
		t.Fatalf("businessNo = %q, want 2768603291", result.Bundle.BusinessNo)
	}
}

func TestServiceBusinessNumberDoesNotFallbackToHeuristicDigits(t *testing.T) {
	root := t.TempDir()
	fixture := writeFixtureBundle(t, root, fixtureOptions{
		dirName:         "cert-1",
		password:        "secret-password",
		commonName:      "Invoice 1234567890 Test Certificate",
		businessNo:      "",
		validUntil:      time.Date(2027, 4, 9, 0, 0, 0, 0, time.UTC),
		vidRandom:       []byte{17, 18, 19, 20},
		includeSerialNo: false,
	})
	service := NewService(ServiceConfig{Roots: []string{root}})

	result, err := service.Sign(context.Background(), SignRequest{
		CertificateID: bundleCertificateID(fixture.path),
		Challenge:     "challenge-1",
		Password:      "secret-password",
		BusinessNo:    "2768603291",
	})
	if err != nil {
		t.Fatalf("Sign() error = %v", err)
	}
	if result.Bundle.BusinessNo != "" {
		t.Fatalf("businessNo = %q, want empty string when serial-number OID is absent", result.Bundle.BusinessNo)
	}
}

func TestServiceListCertificatesFollowsSymlinkedBundleDirectories(t *testing.T) {
	root := t.TempDir()
	realRoot := t.TempDir()
	fixture := writeFixtureBundle(t, realRoot, fixtureOptions{
		dirName:    "cert-1",
		password:   "secret-password",
		commonName: "Symlink Certificate",
		businessNo: "276-86-03291",
		validUntil: time.Date(2027, 4, 9, 0, 0, 0, 0, time.UTC),
		vidRandom:  []byte{21, 22, 23, 24},
	})
	linkPath := filepath.Join(root, "linked-cert")
	if err := os.Symlink(fixture.path, linkPath); err != nil {
		t.Fatalf("symlink bundle: %v", err)
	}

	service := NewService(ServiceConfig{Roots: []string{root}})
	got, err := service.ListCertificates(context.Background())
	if err != nil {
		t.Fatalf("ListCertificates() error = %v", err)
	}
	if len(got) != 1 {
		t.Fatalf("len(got) = %d, want 1", len(got))
	}
	if got[0].CommonName != "Symlink Certificate" {
		t.Fatalf("commonName = %q, want Symlink Certificate", got[0].CommonName)
	}
}

func TestServiceCanonicalizesSymlinkedBundleMetadataFromResolvedGPKIRoot(t *testing.T) {
	root := t.TempDir()
	realRoot := t.TempDir()
	fixture := writeFixtureBundle(t, filepath.Join(realRoot, "GPKI"), fixtureOptions{
		dirName:    "cert-1",
		password:   "secret-password",
		commonName: "Canonical GPKI Certificate",
		businessNo: "276-86-03291",
		validUntil: time.Date(2027, 4, 9, 0, 0, 0, 0, time.UTC),
		vidRandom:  []byte{21, 22, 23, 24},
	})
	linkPath := filepath.Join(root, "alias-cert")
	if err := os.Symlink(fixture.path, linkPath); err != nil {
		t.Fatalf("symlink bundle: %v", err)
	}

	service := NewService(ServiceConfig{Roots: []string{root}})
	got, err := service.ListCertificates(context.Background())
	if err != nil {
		t.Fatalf("ListCertificates() error = %v", err)
	}
	if len(got) != 1 {
		t.Fatalf("len(got) = %d, want 1", len(got))
	}

	item := got[0]
	wantID := canonicalBundleID(fixture.path)
	if item.ID != wantID {
		t.Fatalf("id = %q, want canonical id %q", item.ID, wantID)
	}
	if item.StorageKind != CertificateStorageKindGPKI {
		t.Fatalf("storageKind = %q, want gpki from resolved bundle location", item.StorageKind)
	}
	if item.DisplayPath != "GPKI/cert-1" {
		t.Fatalf("displayPath = %q, want GPKI/cert-1", item.DisplayPath)
	}

	result, err := service.Sign(context.Background(), SignRequest{
		CertificateID: wantID,
		Challenge:     "challenge-1",
		Password:      "secret-password",
	})
	if err != nil {
		t.Fatalf("Sign() with canonical id error = %v", err)
	}
	if result.CertificateID != wantID {
		t.Fatalf("sign certificateID = %q, want canonical id %q", result.CertificateID, wantID)
	}

	aliasResult, err := service.Sign(context.Background(), SignRequest{
		CertificateID: linkPath,
		Challenge:     "challenge-2",
		Password:      "secret-password",
	})
	if err != nil {
		t.Fatalf("Sign() with symlink alias error = %v", err)
	}
	if aliasResult.CertificateID != wantID {
		t.Fatalf("alias certificateID = %q, want canonical id %q", aliasResult.CertificateID, wantID)
	}
}

func TestServiceListCertificatesDeduplicatesSymlinksToSameBundle(t *testing.T) {
	root := t.TempDir()
	realRoot := t.TempDir()
	fixture := writeFixtureBundle(t, realRoot, fixtureOptions{
		dirName:    "cert-1",
		password:   "secret-password",
		commonName: "Symlink Certificate",
		businessNo: "276-86-03291",
		validUntil: time.Date(2027, 4, 9, 0, 0, 0, 0, time.UTC),
		vidRandom:  []byte{25, 26, 27, 28},
	})
	for _, linkName := range []string{"linked-cert-a", "linked-cert-b"} {
		if err := os.Symlink(fixture.path, filepath.Join(root, linkName)); err != nil {
			t.Fatalf("symlink bundle %s: %v", linkName, err)
		}
	}

	service := NewService(ServiceConfig{Roots: []string{root}})
	got, err := service.ListCertificates(context.Background())
	if err != nil {
		t.Fatalf("ListCertificates() error = %v", err)
	}
	if len(got) != 1 {
		t.Fatalf("len(got) = %d, want 1", len(got))
	}
}

func TestSplitRootListPreservesWindowsDrivePaths(t *testing.T) {
	got := splitRootList(`C:\NPKI;D:\NPKI,/custom/npki`)

	nonWindowsRoot, err := filepath.Abs("/custom/npki")
	if err != nil {
		t.Fatalf("filepath.Abs(/custom/npki): %v", err)
	}

	want := []string{`C:\NPKI`, `D:\NPKI`, nonWindowsRoot}
	if !reflect.DeepEqual(got, want) {
		t.Fatalf("splitRootList() = %#v, want %#v", got, want)
	}
}

func TestDiscoverMountedCertificateRootsIncludesMountedVolumeForGPKI(t *testing.T) {
	volumesRoot := t.TempDir()
	mountRoot := filepath.Join(volumesRoot, "Lexar")
	_ = writeFixtureBundle(t, filepath.Join(mountRoot, "GPKI"), fixtureOptions{
		dirName:    "cert-1",
		password:   "secret-password",
		commonName: "Mounted GPKI Certificate",
		businessNo: "276-86-03291",
		validUntil: time.Date(2027, 4, 9, 0, 0, 0, 0, time.UTC),
		vidRandom:  []byte{77, 78, 79, 80},
	})

	got := discoverMountedCertificateRoots(context.Background(), volumesRoot)
	want := []string{filepath.Clean(mountRoot)}
	if !reflect.DeepEqual(got, want) {
		t.Fatalf("discoverMountedCertificateRoots() = %#v, want %#v", got, want)
	}
}

func TestDiscoverMountedCertificateRootsIncludesMountedVolumeForTopLevelPKCS12(t *testing.T) {
	volumesRoot := t.TempDir()
	mountRoot := filepath.Join(volumesRoot, "Lexar")
	if err := os.MkdirAll(mountRoot, 0o755); err != nil {
		t.Fatalf("mkdir mounted root: %v", err)
	}
	mustWriteFile(t, filepath.Join(mountRoot, "user.p12"), []byte("p12"))
	mustWriteFile(t, filepath.Join(mountRoot, "backup.pfx"), []byte("pfx"))

	got := discoverMountedCertificateRoots(context.Background(), volumesRoot)
	want := []string{filepath.Clean(mountRoot)}
	if !reflect.DeepEqual(got, want) {
		t.Fatalf("discoverMountedCertificateRoots() = %#v, want %#v", got, want)
	}
}

func TestDiscoverMountedCertificateRootsFollowsSymlinkedGPKIRoot(t *testing.T) {
	volumesRoot := t.TempDir()
	mountRoot := filepath.Join(volumesRoot, "Lexar")
	realRoot := t.TempDir()
	gpkiRoot := filepath.Join(realRoot, "GPKI")
	_ = writeFixtureBundle(t, gpkiRoot, fixtureOptions{
		dirName:    "cert-1",
		password:   "secret-password",
		commonName: "Symlinked Mounted GPKI Certificate",
		businessNo: "276-86-03291",
		validUntil: time.Date(2027, 4, 9, 0, 0, 0, 0, time.UTC),
		vidRandom:  []byte{85, 86, 87, 88},
	})
	if err := os.MkdirAll(mountRoot, 0o755); err != nil {
		t.Fatalf("mkdir mounted root: %v", err)
	}
	if err := os.Symlink(gpkiRoot, filepath.Join(mountRoot, "GPKI")); err != nil {
		t.Fatalf("symlink gpki root: %v", err)
	}

	got := discoverMountedCertificateRoots(context.Background(), volumesRoot)
	want := []string{filepath.Clean(mountRoot)}
	if !reflect.DeepEqual(got, want) {
		t.Fatalf("discoverMountedCertificateRoots() = %#v, want %#v", got, want)
	}
}

func TestDiscoverMountedCertificateRootsSkipsFailedMountReads(t *testing.T) {
	volumesRoot := t.TempDir()
	readableMount := filepath.Join(volumesRoot, "Lexar")
	if err := os.MkdirAll(filepath.Join(readableMount, "GPKI"), 0o755); err != nil {
		t.Fatalf("mkdir readable mount: %v", err)
	}
	failingMount := filepath.Join(volumesRoot, "Broken")
	if err := os.MkdirAll(failingMount, 0o755); err != nil {
		t.Fatalf("mkdir broken mount: %v", err)
	}
	mustWriteFile(t, filepath.Join(failingMount, "broken.p12"), []byte("p12"))
	originalReadDir := readDir
	readDir = func(name string) ([]os.DirEntry, error) {
		if name == failingMount {
			return nil, os.ErrPermission
		}
		return originalReadDir(name)
	}
	t.Cleanup(func() {
		readDir = originalReadDir
	})

	got := discoverMountedCertificateRoots(context.Background(), volumesRoot)
	want := []string{filepath.Clean(readableMount)}
	if !reflect.DeepEqual(got, want) {
		t.Fatalf("discoverMountedCertificateRoots() = %#v, want %#v", got, want)
	}
}

func TestServiceListCertificatesSkipsMissingRoots(t *testing.T) {
	goodRoot := t.TempDir()
	_ = writeFixtureBundle(t, goodRoot, fixtureOptions{
		dirName:    "cert-1",
		password:   "secret-password",
		commonName: "Readable Certificate",
		businessNo: "276-86-03291",
		validUntil: time.Date(2027, 4, 9, 0, 0, 0, 0, time.UTC),
		vidRandom:  []byte{81, 82, 83, 84},
	})
	missingRoot := filepath.Join(t.TempDir(), "missing-root")

	service := NewService(ServiceConfig{Roots: []string{goodRoot, missingRoot}})
	got, err := service.ListCertificates(context.Background())
	if err != nil {
		t.Fatalf("ListCertificates() error = %v", err)
	}
	if len(got) != 1 {
		t.Fatalf("len(got) = %d, want 1", len(got))
	}
	if got[0].CommonName != "Readable Certificate" {
		t.Fatalf("commonName = %q, want Readable Certificate", got[0].CommonName)
	}
}

func TestDefaultDiscoveredRootsTTLIsLongEnough(t *testing.T) {
	if defaultDiscoveredRootsTTL < 30*time.Second {
		t.Fatalf("defaultDiscoveredRootsTTL = %s, want at least 30s", defaultDiscoveredRootsTTL)
	}
}

func TestServiceRefreshDiscoveredRootsClosesOwnedDoneChannel(t *testing.T) {
	service := NewService(ServiceConfig{
		AutoDiscover: true,
		DiscoverRoots: func(context.Context) []string {
			return nil
		},
	})

	done := make(chan struct{})
	service.mu.Lock()
	service.refreshing = true
	service.refreshDone = done
	service.mu.Unlock()

	service.refreshDiscoveredRoots()

	select {
	case <-done:
	case <-time.After(200 * time.Millisecond):
		t.Fatal("refreshDone was not closed")
	}

	service.mu.Lock()
	defer service.mu.Unlock()
	if service.refreshing {
		t.Fatal("refreshing = true, want false")
	}
	if service.refreshDone != nil {
		t.Fatalf("refreshDone = %v, want nil", service.refreshDone)
	}
	if !service.rootsCached {
		t.Fatal("rootsCached = false, want true")
	}
}

func TestServiceRefreshesAutoDiscoveredRootsAfterTTL(t *testing.T) {
	firstRoot := t.TempDir()
	secondRoot := t.TempDir()
	first := writeFixtureBundle(t, firstRoot, fixtureOptions{
		dirName:    "cert-1",
		password:   "secret-password",
		commonName: "First Certificate",
		businessNo: "276-86-03291",
		validUntil: time.Date(2027, 4, 9, 0, 0, 0, 0, time.UTC),
		vidRandom:  []byte{29, 30, 31, 32},
	})
	second := writeFixtureBundle(t, secondRoot, fixtureOptions{
		dirName:    "cert-2",
		password:   "secret-password",
		commonName: "Second Certificate",
		businessNo: "276-86-03291",
		validUntil: time.Date(2027, 4, 9, 0, 0, 0, 0, time.UTC),
		vidRandom:  []byte{33, 34, 35, 36},
	})

	now := time.Date(2026, 4, 14, 15, 0, 0, 0, time.UTC)
	discoveredRoots := []string{firstRoot}
	service := NewService(ServiceConfig{
		AutoDiscover: true,
		RootsTTL:     5 * time.Second,
		Now: func() time.Time {
			return now
		},
		DiscoverRoots: func(context.Context) []string {
			return append([]string(nil), discoveredRoots...)
		},
	})

	waitForCertificateIDs(t, service, []string{bundleCertificateID(first.path)})

	discoveredRoots = []string{secondRoot}
	got, err := service.ListCertificates(context.Background())
	if err != nil {
		t.Fatalf("cached ListCertificates() error = %v", err)
	}
	if ids := certificateIDs(got); !reflect.DeepEqual(ids, []string{bundleCertificateID(first.path)}) {
		t.Fatalf("cached certificate IDs = %#v, want %#v before TTL expiry", ids, []string{bundleCertificateID(first.path)})
	}

	now = now.Add(6 * time.Second)
	waitForCertificateIDs(t, service, []string{bundleCertificateID(second.path)})
}

func TestServiceReturnsStaticRootsWhenColdDiscoveryExceedsInitialWait(t *testing.T) {
	staticRoot := t.TempDir()
	mountedRoot := t.TempDir()
	staticCert := writeFixtureBundle(t, staticRoot, fixtureOptions{
		dirName:    "cert-1",
		password:   "secret-password",
		commonName: "Static Certificate",
		businessNo: "276-86-03291",
		validUntil: time.Date(2027, 4, 9, 0, 0, 0, 0, time.UTC),
		vidRandom:  []byte{61, 62, 63, 64},
	})
	mountedCert := writeFixtureBundle(t, mountedRoot, fixtureOptions{
		dirName:    "cert-2",
		password:   "secret-password",
		commonName: "Mounted Certificate",
		businessNo: "276-86-03291",
		validUntil: time.Date(2027, 4, 9, 0, 0, 0, 0, time.UTC),
		vidRandom:  []byte{65, 66, 67, 68},
	})

	releaseRefresh := make(chan struct{})
	service := NewService(ServiceConfig{
		Roots:                []string{staticRoot},
		AutoDiscover:         true,
		RootsTTL:             5 * time.Second,
		InitialDiscoveryWait: 20 * time.Millisecond,
		DiscoverRoots: func(context.Context) []string {
			<-releaseRefresh
			return []string{mountedRoot}
		},
	})

	start := time.Now()
	got, err := service.ListCertificates(context.Background())
	if err != nil {
		t.Fatalf("initial ListCertificates() error = %v", err)
	}
	if elapsed := time.Since(start); elapsed > 200*time.Millisecond {
		t.Fatalf("initial ListCertificates() took %s, want bounded wait", elapsed)
	}
	if ids := certificateIDs(got); !reflect.DeepEqual(ids, []string{bundleCertificateID(staticCert.path)}) {
		t.Fatalf("initial certificate IDs = %#v, want static roots only %#v", ids, []string{bundleCertificateID(staticCert.path)})
	}

	close(releaseRefresh)
	waitForCertificateIDs(t, service, []string{bundleCertificateID(staticCert.path), bundleCertificateID(mountedCert.path)})
}

func TestServiceReturnsStaleRootsWhileRefreshRunsInBackground(t *testing.T) {
	firstRoot := t.TempDir()
	secondRoot := t.TempDir()
	first := writeFixtureBundle(t, firstRoot, fixtureOptions{
		dirName:    "cert-1",
		password:   "secret-password",
		commonName: "First Certificate",
		businessNo: "276-86-03291",
		validUntil: time.Date(2027, 4, 9, 0, 0, 0, 0, time.UTC),
		vidRandom:  []byte{37, 38, 39, 40},
	})
	second := writeFixtureBundle(t, secondRoot, fixtureOptions{
		dirName:    "cert-2",
		password:   "secret-password",
		commonName: "Second Certificate",
		businessNo: "276-86-03291",
		validUntil: time.Date(2027, 4, 9, 0, 0, 0, 0, time.UTC),
		vidRandom:  []byte{41, 42, 43, 44},
	})

	now := time.Date(2026, 4, 14, 15, 5, 0, 0, time.UTC)
	discoveredRoots := []string{firstRoot}
	service := NewService(ServiceConfig{
		AutoDiscover: true,
		RootsTTL:     5 * time.Second,
		Now: func() time.Time {
			return now
		},
		DiscoverRoots: func(context.Context) []string {
			return append([]string(nil), discoveredRoots...)
		},
	})

	waitForCertificateIDs(t, service, []string{bundleCertificateID(first.path)})

	now = now.Add(6 * time.Second)
	refreshStarted := make(chan struct{}, 1)
	releaseRefresh := make(chan struct{})
	service.discoverRoots = func(context.Context) []string {
		refreshStarted <- struct{}{}
		<-releaseRefresh
		return []string{secondRoot}
	}

	done := make(chan []string, 1)
	errCh := make(chan error, 1)
	go func() {
		got, err := service.ListCertificates(context.Background())
		if err != nil {
			errCh <- err
			return
		}
		done <- certificateIDs(got)
	}()

	select {
	case ids := <-done:
		if !reflect.DeepEqual(ids, []string{bundleCertificateID(first.path)}) {
			t.Fatalf("certificate IDs during refresh = %#v, want stale cache %#v", ids, []string{bundleCertificateID(first.path)})
		}
	case err := <-errCh:
		t.Fatalf("ListCertificates() during refresh error = %v", err)
	case <-time.After(200 * time.Millisecond):
		t.Fatal("ListCertificates() blocked on root refresh")
	}

	select {
	case <-refreshStarted:
	case <-time.After(200 * time.Millisecond):
		t.Fatal("background refresh did not start")
	}
	close(releaseRefresh)

	waitForCertificateIDs(t, service, []string{bundleCertificateID(second.path)})
}

func TestServiceMergesStaticRootsWithAsyncDiscoveredRoots(t *testing.T) {
	staticRoot := t.TempDir()
	mountedRoot := t.TempDir()
	staticCert := writeFixtureBundle(t, staticRoot, fixtureOptions{
		dirName:    "cert-1",
		password:   "secret-password",
		commonName: "Static Certificate",
		businessNo: "276-86-03291",
		validUntil: time.Date(2027, 4, 9, 0, 0, 0, 0, time.UTC),
		vidRandom:  []byte{45, 46, 47, 48},
	})
	mountedCert := writeFixtureBundle(t, mountedRoot, fixtureOptions{
		dirName:    "cert-2",
		password:   "secret-password",
		commonName: "Mounted Certificate",
		businessNo: "276-86-03291",
		validUntil: time.Date(2027, 4, 9, 0, 0, 0, 0, time.UTC),
		vidRandom:  []byte{49, 50, 51, 52},
	})

	service := NewService(ServiceConfig{
		Roots:        []string{staticRoot},
		AutoDiscover: true,
		RootsTTL:     5 * time.Second,
		DiscoverRoots: func(context.Context) []string {
			return []string{mountedRoot}
		},
	})

	got, err := service.ListCertificates(context.Background())
	if err != nil {
		t.Fatalf("initial ListCertificates() error = %v", err)
	}
	if ids := certificateIDs(got); !reflect.DeepEqual(ids, []string{bundleCertificateID(staticCert.path), bundleCertificateID(mountedCert.path)}) {
		t.Fatalf("initial certificate IDs = %#v, want both static and mounted roots %#v", ids, []string{bundleCertificateID(staticCert.path), bundleCertificateID(mountedCert.path)})
	}
	waitForCertificateIDs(t, service, []string{bundleCertificateID(staticCert.path), bundleCertificateID(mountedCert.path)})
}

func TestServiceRetriesDiscoveryAfterTimeout(t *testing.T) {
	staticRoot := t.TempDir()
	mountedRoot := t.TempDir()
	staticCert := writeFixtureBundle(t, staticRoot, fixtureOptions{
		dirName:    "cert-1",
		password:   "secret-password",
		commonName: "Static Certificate",
		businessNo: "276-86-03291",
		validUntil: time.Date(2027, 4, 9, 0, 0, 0, 0, time.UTC),
		vidRandom:  []byte{69, 70, 71, 72},
	})
	mountedCert := writeFixtureBundle(t, mountedRoot, fixtureOptions{
		dirName:    "cert-2",
		password:   "secret-password",
		commonName: "Mounted Certificate",
		businessNo: "276-86-03291",
		validUntil: time.Date(2027, 4, 9, 0, 0, 0, 0, time.UTC),
		vidRandom:  []byte{73, 74, 75, 76},
	})

	var calls atomic.Int32
	service := NewService(ServiceConfig{
		Roots:                []string{staticRoot},
		AutoDiscover:         true,
		RootsTTL:             5 * time.Second,
		InitialDiscoveryWait: 20 * time.Millisecond,
		DiscoveryTimeout:     30 * time.Millisecond,
		DiscoverRoots: func(ctx context.Context) []string {
			if calls.Add(1) == 1 {
				<-ctx.Done()
				return []string{mountedRoot}
			}
			return []string{mountedRoot}
		},
	})

	got, err := service.ListCertificates(context.Background())
	if err != nil {
		t.Fatalf("initial ListCertificates() error = %v", err)
	}
	if ids := certificateIDs(got); !reflect.DeepEqual(ids, []string{bundleCertificateID(staticCert.path)}) {
		t.Fatalf("initial certificate IDs = %#v, want static roots only %#v", ids, []string{bundleCertificateID(staticCert.path)})
	}

	waitForRefreshIdle(t, service)
	if gotCalls := calls.Load(); gotCalls != 1 {
		t.Fatalf("discoverRoots call count = %d, want 1 after timed-out refresh", gotCalls)
	}

	waitForCertificateIDs(t, service, []string{bundleCertificateID(staticCert.path), bundleCertificateID(mountedCert.path)})
	if gotCalls := calls.Load(); gotCalls < 2 {
		t.Fatalf("discoverRoots call count = %d, want retry after timeout", gotCalls)
	}
}

func TestServiceAutoDiscoverUsesDefaultRootsEvenWithStaticRoots(t *testing.T) {
	staticRoot := t.TempDir()
	mountedRoot := t.TempDir()
	staticCert := writeFixtureBundle(t, staticRoot, fixtureOptions{
		dirName:    "cert-1",
		password:   "secret-password",
		commonName: "Static Certificate",
		businessNo: "276-86-03291",
		validUntil: time.Date(2027, 4, 9, 0, 0, 0, 0, time.UTC),
		vidRandom:  []byte{53, 54, 55, 56},
	})
	mountedCert := writeFixtureBundle(t, mountedRoot, fixtureOptions{
		dirName:    "cert-2",
		password:   "secret-password",
		commonName: "Mounted Certificate",
		businessNo: "276-86-03291",
		validUntil: time.Date(2027, 4, 9, 0, 0, 0, 0, time.UTC),
		vidRandom:  []byte{57, 58, 59, 60},
	})

	originalDefaultDiscoverRoots := defaultDiscoverRoots
	defer func() {
		defaultDiscoverRoots = originalDefaultDiscoverRoots
	}()
	defaultDiscoverRoots = func(context.Context) []string {
		return []string{mountedRoot}
	}

	service := NewService(ServiceConfig{
		Roots:        []string{staticRoot},
		AutoDiscover: true,
	})

	got, err := service.ListCertificates(context.Background())
	if err != nil {
		t.Fatalf("ListCertificates() error = %v", err)
	}
	if ids := certificateIDs(got); !reflect.DeepEqual(ids, []string{bundleCertificateID(staticCert.path), bundleCertificateID(mountedCert.path)}) {
		t.Fatalf("certificate IDs = %#v, want %#v", ids, []string{bundleCertificateID(staticCert.path), bundleCertificateID(mountedCert.path)})
	}
}

func waitForRefreshIdle(t *testing.T, service *Service) {
	t.Helper()

	deadline := time.Now().Add(2 * time.Second)
	for time.Now().Before(deadline) {
		service.mu.Lock()
		refreshing := service.refreshing
		service.mu.Unlock()
		if !refreshing {
			return
		}
		time.Sleep(10 * time.Millisecond)
	}

	service.mu.Lock()
	defer service.mu.Unlock()
	t.Fatalf("service refresh remained active: refreshing=%v", service.refreshing)
}

func waitForCertificateIDs(t *testing.T, service *Service, want []string) {
	t.Helper()
	slices.Sort(want)

	deadline := time.Now().Add(2 * time.Second)
	for time.Now().Before(deadline) {
		got, err := service.ListCertificates(context.Background())
		if err != nil {
			t.Fatalf("ListCertificates() error = %v", err)
		}
		if ids := certificateIDs(got); reflect.DeepEqual(ids, want) {
			return
		}
		time.Sleep(10 * time.Millisecond)
	}

	got, err := service.ListCertificates(context.Background())
	if err != nil {
		t.Fatalf("final ListCertificates() error = %v", err)
	}
	t.Fatalf("certificate IDs = %#v, want %#v", certificateIDs(got), want)
}

func certificateIDs(items []CertificateMetadata) []string {
	ids := make([]string, 0, len(items))
	for _, item := range items {
		ids = append(ids, item.ID)
	}
	slices.Sort(ids)
	return ids
}

func bundleCertificateID(path string) string {
	canonicalPath, _ := canonicalizeBundlePath(path)
	return "bundle:" + canonicalPath
}

func canonicalBundleID(path string) string {
	canonicalPath, _ := canonicalizeBundlePath(path)
	return bundleCertificateID(canonicalPath)
}

type fixtureOptions struct {
	dirName         string
	password        string
	commonName      string
	businessNo      string
	validUntil      time.Time
	vidRandom       []byte
	includeSerialNo bool
}

type fixtureBundle struct {
	path        string
	signCertB64 string
}

type fixturePKCS12 struct {
	path        string
	signCertB64 string
}

func writeFixtureBundle(t *testing.T, root string, opts fixtureOptions) fixtureBundle {
	t.Helper()

	dir := filepath.Join(root, opts.dirName)
	if err := os.MkdirAll(dir, 0o755); err != nil {
		t.Fatalf("mkdir %s: %v", dir, err)
	}

	signKey, err := rsa.GenerateKey(rand.Reader, 1024)
	if err != nil {
		t.Fatalf("generate sign key: %v", err)
	}
	kmKey, err := rsa.GenerateKey(rand.Reader, 1024)
	if err != nil {
		t.Fatalf("generate km key: %v", err)
	}

	signCertDER := createCertificateDER(t, signKey, opts.commonName, opts.businessNo, opts.validUntil)
	kmCertDER := createCertificateDER(t, kmKey, "KM "+opts.commonName, opts.businessNo, opts.validUntil)
	signPriDER := createEncryptedPrivateKeyDER(t, signKey, opts.password, opts.vidRandom)
	kmPriDER := x509.MarshalPKCS1PrivateKey(kmKey)

	mustWriteFile(t, filepath.Join(dir, "signCert.der"), signCertDER)
	mustWriteFile(t, filepath.Join(dir, "signPri.key"), signPriDER)
	mustWriteFile(t, filepath.Join(dir, "kmCert.der"), kmCertDER)
	mustWriteFile(t, filepath.Join(dir, "kmPri.key"), kmPriDER)

	return fixtureBundle{
		path:        dir,
		signCertB64: base64.StdEncoding.EncodeToString(signCertDER),
	}
}

func writeFixturePKCS12(t *testing.T, root string, opts fixtureOptions) fixturePKCS12 {
	t.Helper()

	path := filepath.Join(root, opts.dirName)
	if err := os.MkdirAll(filepath.Dir(path), 0o755); err != nil {
		t.Fatalf("mkdir %s: %v", filepath.Dir(path), err)
	}

	signKey, err := rsa.GenerateKey(rand.Reader, 1024)
	if err != nil {
		t.Fatalf("generate sign key: %v", err)
	}
	signCertDER := createCertificateDER(t, signKey, opts.commonName, opts.businessNo, opts.validUntil)
	signCert, err := x509.ParseCertificate(signCertDER)
	if err != nil {
		t.Fatalf("parse sign certificate: %v", err)
	}
	pfxDER, err := pkcs12.Encode(rand.Reader, signKey, signCert, nil, opts.password)
	if err != nil {
		t.Fatalf("encode pkcs12: %v", err)
	}
	mustWriteFile(t, path, pfxDER)

	return fixturePKCS12{
		path:        path,
		signCertB64: base64.StdEncoding.EncodeToString(signCertDER),
	}
}

func createCertificateDER(
	t *testing.T,
	key *rsa.PrivateKey,
	commonName string,
	businessNo string,
	validUntil time.Time,
) []byte {
	t.Helper()

	extraNames := []pkix.AttributeTypeAndValue{}
	if businessNo != "" {
		extraNames = append(extraNames, pkix.AttributeTypeAndValue{
			Type:  asn1.ObjectIdentifier{2, 5, 4, 5},
			Value: businessNo,
		})
	}

	template := &x509.Certificate{
		SerialNumber:          big.NewInt(1),
		Subject:               pkix.Name{CommonName: commonName, ExtraNames: extraNames},
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

func createEncryptedPrivateKeyDER(t *testing.T, key *rsa.PrivateKey, password string, vidRandom []byte) []byte {
	t.Helper()

	privateDER := x509.MarshalPKCS1PrivateKey(key)
	plainDER := marshalPlainPrivateKey(t, privateDER, vidRandom)
	salt := []byte("12345678")
	iteration := 2048
	derived := pbkdf1SHA1([]byte(password), salt, iteration, 20)
	seedKey := derived[:16]
	ivDigest := sha1.Sum(derived[16:20])
	iv := ivDigest[:16]
	cipherText := encryptSeedCBCPKCS7(t, seedKey, iv, plainDER)

	type params struct {
		Salt      []byte
		Iteration int
	}
	type meta struct {
		Algorithm asn1.ObjectIdentifier
		Params    params
	}
	type encrypted struct {
		Meta       meta
		CipherText []byte
	}

	der, err := asn1.Marshal(encrypted{
		Meta: meta{
			Algorithm: asn1.ObjectIdentifier{1, 2, 410, 200004, 1, 15},
			Params: params{
				Salt:      salt,
				Iteration: iteration,
			},
		},
		CipherText: cipherText,
	})
	if err != nil {
		t.Fatalf("marshal encrypted private key: %v", err)
	}
	return der
}

func marshalPlainPrivateKey(t *testing.T, privateDER []byte, vidRandom []byte) []byte {
	t.Helper()

	type plain struct {
		Version    int
		Algorithm  asn1.RawValue
		PrivateKey []byte
		VidRandom  asn1.RawValue `asn1:"optional,explicit,tag:0"`
	}

	vidInner, err := asn1.Marshal(struct {
		Value asn1.BitString
	}{
		Value: asn1.BitString{
			Bytes:     vidRandom,
			BitLength: len(vidRandom) * 8,
		},
	})
	if err != nil {
		t.Fatalf("marshal vidRandom: %v", err)
	}

	der, err := asn1.Marshal(plain{
		Version:    0,
		Algorithm:  asn1.RawValue{FullBytes: []byte{0x05, 0x00}},
		PrivateKey: privateDER,
		VidRandom: asn1.RawValue{
			Class:      asn1.ClassContextSpecific,
			Tag:        0,
			IsCompound: true,
			Bytes:      vidInner,
		},
	})
	if err != nil {
		t.Fatalf("marshal plain private key: %v", err)
	}
	return der
}

func encryptSeedCBCPKCS7(t *testing.T, key []byte, iv []byte, plaintext []byte) []byte {
	t.Helper()

	block, err := seedcipher.NewCipher(key)
	if err != nil {
		t.Fatalf("seed cipher: %v", err)
	}
	padded := pkcs7Pad(plaintext, block.BlockSize())
	cipherText := make([]byte, len(padded))
	mode := cipher.NewCBCEncrypter(block, iv)
	mode.CryptBlocks(cipherText, padded)
	return cipherText
}

func pkcs7Pad(input []byte, blockSize int) []byte {
	padding := blockSize - (len(input) % blockSize)
	out := make([]byte, len(input)+padding)
	copy(out, input)
	for i := len(input); i < len(out); i++ {
		out[i] = byte(padding)
	}
	return out
}

func mustWriteFile(t *testing.T, path string, data []byte) {
	t.Helper()
	if err := os.WriteFile(path, data, 0o600); err != nil {
		t.Fatalf("write %s: %v", path, err)
	}
}

func pkcs12CertificateID(path string) string {
	canonicalPath, _ := canonicalizeBundlePath(path)
	return "pkcs12:" + canonicalPath
}
