package npki

import (
	"context"
	"crypto/cipher"
	"crypto/rand"
	"crypto/rsa"
	"crypto/sha1"
	"crypto/x509"
	"crypto/x509/pkix"
	"encoding/asn1"
	"encoding/base64"
	"errors"
	"math/big"
	"os"
	"path/filepath"
	"reflect"
	"testing"
	"time"

	seedcipher "github.com/deatil/go-cryptobin/cipher/seed"
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
	if got[0].ID != fixture.path {
		t.Fatalf("id = %q, want %q", got[0].ID, fixture.path)
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
		CertificateID: fixture.path,
		Challenge:     "challenge-1",
		Password:      "secret-password",
	})
	if err != nil {
		t.Fatalf("Sign() error = %v", err)
	}
	if result.CertificateID != fixture.path {
		t.Fatalf("certificateID = %q, want %q", result.CertificateID, fixture.path)
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
		CertificateID: fixture.path,
		Challenge:     "challenge-1",
		Password:      "wrong-password",
	})
	if !errors.Is(err, ErrInvalidPassword) {
		t.Fatalf("Sign() error = %v, want ErrInvalidPassword", err)
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
		CertificateID: fixture.path,
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
		CertificateID: fixture.path,
		Challenge:     "challenge-1",
		Password:      "secret-password",
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
	want := []string{`C:\NPKI`, `D:\NPKI`, `/custom/npki`}
	if !reflect.DeepEqual(got, want) {
		t.Fatalf("splitRootList() = %#v, want %#v", got, want)
	}
}

func TestDiscoverDefaultRootsForDarwinIncludesMountedNPKIRoots(t *testing.T) {
	home := t.TempDir()
	volumesRoot := t.TempDir()
	volumeNPKI := filepath.Join(volumesRoot, "Lexar", "NPKI")
	if err := os.MkdirAll(volumeNPKI, 0o755); err != nil {
		t.Fatalf("mkdir mounted npki root: %v", err)
	}

	got := discoverDefaultRootsFor("darwin", home, volumesRoot)
	want := filepath.Clean(volumeNPKI)

	for _, root := range got {
		if root == want {
			return
		}
	}
	t.Fatalf("discoverDefaultRootsFor() = %#v, want mounted root %q included", got, want)
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
		DiscoverRoots: func() []string {
			return append([]string(nil), discoveredRoots...)
		},
	})

	got, err := service.ListCertificates(context.Background())
	if err != nil {
		t.Fatalf("initial ListCertificates() error = %v", err)
	}
	if ids := certificateIDs(got); !reflect.DeepEqual(ids, []string{first.path}) {
		t.Fatalf("initial certificate IDs = %#v, want %#v", ids, []string{first.path})
	}

	discoveredRoots = []string{secondRoot}
	got, err = service.ListCertificates(context.Background())
	if err != nil {
		t.Fatalf("cached ListCertificates() error = %v", err)
	}
	if ids := certificateIDs(got); !reflect.DeepEqual(ids, []string{first.path}) {
		t.Fatalf("cached certificate IDs = %#v, want %#v before TTL expiry", ids, []string{first.path})
	}

	now = now.Add(6 * time.Second)
	got, err = service.ListCertificates(context.Background())
	if err != nil {
		t.Fatalf("refreshed ListCertificates() error = %v", err)
	}
	if ids := certificateIDs(got); !reflect.DeepEqual(ids, []string{second.path}) {
		t.Fatalf("refreshed certificate IDs = %#v, want %#v after TTL expiry", ids, []string{second.path})
	}
}

func TestServiceExplicitRootsBypassAutoDiscoveryRefresh(t *testing.T) {
	explicitRoot := t.TempDir()
	discoveredRoot := t.TempDir()
	explicit := writeFixtureBundle(t, explicitRoot, fixtureOptions{
		dirName:    "cert-1",
		password:   "secret-password",
		commonName: "Explicit Certificate",
		businessNo: "276-86-03291",
		validUntil: time.Date(2027, 4, 9, 0, 0, 0, 0, time.UTC),
		vidRandom:  []byte{37, 38, 39, 40},
	})
	_ = writeFixtureBundle(t, discoveredRoot, fixtureOptions{
		dirName:    "cert-2",
		password:   "secret-password",
		commonName: "Discovered Certificate",
		businessNo: "276-86-03291",
		validUntil: time.Date(2027, 4, 9, 0, 0, 0, 0, time.UTC),
		vidRandom:  []byte{41, 42, 43, 44},
	})

	now := time.Date(2026, 4, 14, 15, 5, 0, 0, time.UTC)
	service := NewService(ServiceConfig{
		Roots:        []string{explicitRoot},
		AutoDiscover: true,
		RootsTTL:     5 * time.Second,
		Now: func() time.Time {
			return now
		},
		DiscoverRoots: func() []string {
			return []string{discoveredRoot}
		},
	})

	got, err := service.ListCertificates(context.Background())
	if err != nil {
		t.Fatalf("ListCertificates() error = %v", err)
	}
	if ids := certificateIDs(got); !reflect.DeepEqual(ids, []string{explicit.path}) {
		t.Fatalf("certificate IDs = %#v, want explicit root only %#v", ids, []string{explicit.path})
	}
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

func certificateIDs(items []CertificateMetadata) []string {
	ids := make([]string, 0, len(items))
	for _, item := range items {
		ids = append(ids, item.ID)
	}
	return ids
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
