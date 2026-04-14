package npki

import (
	"context"
	"crypto"
	"crypto/cipher"
	"crypto/rand"
	"crypto/rsa"
	"crypto/sha1"
	"crypto/sha256"
	"crypto/x509"
	"encoding/asn1"
	"encoding/base64"
	"errors"
	"fmt"
	"io/fs"
	"log"
	"os"
	"path/filepath"
	"runtime"
	"slices"
	"strings"
	"sync"
	"time"
	"unicode"

	seedcipher "github.com/deatil/go-cryptobin/cipher/seed"
)

var (
	ErrNotImplemented      = errors.New("npki signing is not implemented on this platform")
	ErrCertificateNotFound = errors.New("certificate not found")
	ErrInvalidPassword     = errors.New("invalid certificate password")
)

var (
	oidNPKISeedCBC         = asn1.ObjectIdentifier{1, 2, 410, 200004, 1, 4}
	oidNPKISeedCBCWithSHA1 = asn1.ObjectIdentifier{1, 2, 410, 200004, 1, 15}
	oidX509SerialNumber    = asn1.ObjectIdentifier{2, 5, 4, 5}
)

var defaultDiscoverRoots = discoverDefaultRoots

type ServiceConfig struct {
	Roots                []string
	AutoDiscover         bool
	RootsTTL             time.Duration
	InitialDiscoveryWait time.Duration
	DiscoverRoots        func() []string
	Now                  func() time.Time
}

type CertificateMetadata struct {
	ID         string    `json:"id"`
	CommonName string    `json:"commonName"`
	BusinessNo string    `json:"businessNo"`
	NotAfter   time.Time `json:"notAfter"`
}

type ProofBundle struct {
	BusinessNo        string `json:"businessNo"`
	CommonName        string `json:"commonName"`
	SignCertDerBase64 string `json:"signCertDerBase64"`
	SignPriKeyBase64  string `json:"signPriKeyBase64"`
	KMCertDerBase64   string `json:"kmCertDerBase64"`
	KMPriKeyBase64    string `json:"kmPriKeyBase64"`
}

type SignRequest struct {
	CertificateID string `json:"certificateId"`
	Challenge     string `json:"challenge"`
	Password      string `json:"password"`
	BusinessNo    string `json:"businessNo,omitempty"`
}

type SignResult struct {
	CertificateID            string      `json:"certificateId"`
	Challenge                string      `json:"challenge"`
	ChallengeSignatureBase64 string      `json:"challengeSignatureBase64"`
	Bundle                   ProofBundle `json:"bundle"`
}

type Service struct {
	roots []string

	rootsTTL      time.Duration
	initialWait   time.Duration
	discoverRoots func() []string
	now           func() time.Time

	mu           sync.Mutex
	cachedRoots  []string
	cacheExpires time.Time
	rootsCached  bool
	refreshing   bool
	refreshDone  chan struct{}
}

const defaultDiscoveredRootsTTL = 60 * time.Second
const defaultInitialDiscoveryWait = 250 * time.Millisecond

func NewService(config ServiceConfig) *Service {
	service := &Service{
		roots: normalizeRoots(config.Roots),
		now:   config.Now,
	}
	if service.now == nil {
		service.now = time.Now
	}
	if config.AutoDiscover {
		service.rootsTTL = config.RootsTTL
		if service.rootsTTL <= 0 {
			service.rootsTTL = defaultDiscoveredRootsTTL
		}
		service.initialWait = config.InitialDiscoveryWait
		if service.initialWait <= 0 {
			service.initialWait = defaultInitialDiscoveryWait
		}
		service.discoverRoots = config.DiscoverRoots
		if service.discoverRoots == nil {
			service.discoverRoots = defaultDiscoverRoots
		}
	}
	return service
}

func DefaultConfig() ServiceConfig {
	if raw := strings.TrimSpace(os.Getenv("DESKTOP_AGENT_NPKI_DIRS")); raw != "" {
		return ServiceConfig{Roots: splitRootList(raw)}
	}

	home, err := os.UserHomeDir()
	if err != nil {
		return ServiceConfig{}
	}

	roots := defaultLocalRootsFor(runtime.GOOS, home)
	return ServiceConfig{
		Roots:        roots,
		AutoDiscover: runtime.GOOS == "darwin",
		RootsTTL:     defaultDiscoveredRootsTTL,
		DiscoverRoots: func() []string {
			return discoverMountedNPKIRoots("/Volumes")
		},
	}
}

func (s *Service) ListCertificates(context.Context) ([]CertificateMetadata, error) {
	bundles, err := s.discoverBundles()
	if err != nil {
		return nil, err
	}

	items := make([]CertificateMetadata, 0, len(bundles))
	for _, bundleDir := range bundles {
		metadata, err := readMetadata(bundleDir)
		if err != nil {
			continue
		}
		items = append(items, metadata)
	}
	return items, nil
}

func (s *Service) Sign(_ context.Context, req SignRequest) (SignResult, error) {
	bundleDir, err := s.resolveBundleDir(req.CertificateID)
	if err != nil {
		return SignResult{}, err
	}

	bundle, metadata, err := loadBundle(bundleDir)
	if err != nil {
		return SignResult{}, err
	}
	if err := validatePassword(bundle.signPriKey, req.Password); err != nil {
		return SignResult{}, err
	}
	challengeSignatureBase64, err := signChallenge(bundle.signPriKey, req.Password, req.Challenge)
	if err != nil {
		return SignResult{}, err
	}

	businessNo := metadata.BusinessNo
	if businessNo == "" {
		businessNo = digitsOnly(req.BusinessNo)
	}

	return SignResult{
		CertificateID:            bundleDir,
		Challenge:                req.Challenge,
		ChallengeSignatureBase64: challengeSignatureBase64,
		Bundle: ProofBundle{
			BusinessNo:        businessNo,
			CommonName:        metadata.CommonName,
			SignCertDerBase64: base64.StdEncoding.EncodeToString(bundle.signCertDER),
			SignPriKeyBase64:  base64.StdEncoding.EncodeToString(bundle.signPriKey),
			KMCertDerBase64:   base64.StdEncoding.EncodeToString(bundle.kmCertDER),
			KMPriKeyBase64:    base64.StdEncoding.EncodeToString(bundle.kmPriKey),
		},
	}, nil
}

type bundleFiles struct {
	signCertDER []byte
	signPriKey  []byte
	kmCertDER   []byte
	kmPriKey    []byte
}

func loadBundle(dir string) (bundleFiles, CertificateMetadata, error) {
	signCertDER, err := os.ReadFile(filepath.Join(dir, "signCert.der"))
	if err != nil {
		return bundleFiles{}, CertificateMetadata{}, err
	}
	signPriKey, err := os.ReadFile(filepath.Join(dir, "signPri.key"))
	if err != nil {
		return bundleFiles{}, CertificateMetadata{}, err
	}
	kmCertDER, err := os.ReadFile(filepath.Join(dir, "kmCert.der"))
	if err != nil {
		return bundleFiles{}, CertificateMetadata{}, err
	}
	kmPriKey, err := os.ReadFile(filepath.Join(dir, "kmPri.key"))
	if err != nil {
		return bundleFiles{}, CertificateMetadata{}, err
	}

	metadata, err := metadataFromCertificate(dir, signCertDER)
	if err != nil {
		return bundleFiles{}, CertificateMetadata{}, err
	}
	return bundleFiles{
		signCertDER: signCertDER,
		signPriKey:  signPriKey,
		kmCertDER:   kmCertDER,
		kmPriKey:    kmPriKey,
	}, metadata, nil
}

func readMetadata(dir string) (CertificateMetadata, error) {
	signCertDER, err := os.ReadFile(filepath.Join(dir, "signCert.der"))
	if err != nil {
		return CertificateMetadata{}, err
	}
	return metadataFromCertificate(dir, signCertDER)
}

func metadataFromCertificate(dir string, signCertDER []byte) (CertificateMetadata, error) {
	cert, err := x509.ParseCertificate(signCertDER)
	if err != nil {
		return CertificateMetadata{}, fmt.Errorf("parse signCert.der: %w", err)
	}

	return CertificateMetadata{
		ID:         dir,
		CommonName: strings.TrimSpace(cert.Subject.CommonName),
		BusinessNo: extractBusinessNo(cert),
		NotAfter:   cert.NotAfter.UTC(),
	}, nil
}

func extractBusinessNo(cert *x509.Certificate) string {
	for _, name := range cert.Subject.Names {
		if name.Type.Equal(oidX509SerialNumber) {
			if businessNo := normalizeBusinessNoValue(name.Value); businessNo != "" {
				return businessNo
			}
		}
	}
	for _, name := range cert.Subject.ExtraNames {
		if name.Type.Equal(oidX509SerialNumber) {
			if businessNo := normalizeBusinessNoValue(name.Value); businessNo != "" {
				return businessNo
			}
		}
	}
	return ""
}

func normalizeBusinessNoValue(value any) string {
	digits := digitsOnly(fmt.Sprint(value))
	if len(digits) == 10 {
		return digits
	}
	return ""
}

func (s *Service) resolveBundleDir(id string) (string, error) {
	id = strings.TrimSpace(id)
	if id == "" {
		return "", ErrCertificateNotFound
	}
	bundles, err := s.discoverBundles()
	if err != nil {
		return "", err
	}
	for _, bundleDir := range bundles {
		if bundleDir == id {
			return bundleDir, nil
		}
	}
	return "", ErrCertificateNotFound
}

func (s *Service) discoverBundles() ([]string, error) {
	dirs := make([]string, 0)
	walked := make(map[string]struct{})
	seenBundles := make(map[string]struct{})

	for _, root := range s.bundleRoots() {
		if err := walkBundleRoot(root, &dirs, walked, seenBundles); err != nil {
			return nil, err
		}
	}

	slices.Sort(dirs)
	return dirs, nil
}

func (s *Service) bundleRoots() []string {
	roots := append([]string(nil), s.roots...)
	if s.discoverRoots == nil {
		return roots
	}
	return append(roots, s.cachedDiscoveredRoots()...)
}

func (s *Service) cachedDiscoveredRoots() []string {
	s.mu.Lock()
	now := s.now()
	if s.rootsCached && now.Before(s.cacheExpires) {
		roots := append([]string(nil), s.cachedRoots...)
		s.mu.Unlock()
		return roots
	}

	staleRoots := append([]string(nil), s.cachedRoots...)
	hadCached := s.rootsCached
	if s.refreshing {
		done := s.refreshDone
		s.mu.Unlock()
		if !hadCached && done != nil {
			return s.waitForDiscoveredRoots(done, staleRoots)
		}
		return staleRoots
	}
	s.refreshing = true
	s.refreshDone = make(chan struct{})
	done := s.refreshDone
	s.mu.Unlock()

	if !hadCached {
		go s.refreshDiscoveredRoots(done)
		return s.waitForDiscoveredRoots(done, staleRoots)
	}

	go s.refreshDiscoveredRoots()
	return staleRoots
}

func (s *Service) waitForDiscoveredRoots(done <-chan struct{}, fallback []string) []string {
	timer := time.NewTimer(s.initialWait)
	defer timer.Stop()

	select {
	case <-done:
		s.mu.Lock()
		roots := append([]string(nil), s.cachedRoots...)
		s.mu.Unlock()
		return roots
	case <-timer.C:
		return fallback
	}
}

func (s *Service) refreshDiscoveredRoots(done ...chan struct{}) {
	var doneCh chan struct{}
	if len(done) > 0 {
		doneCh = done[0]
	}
	roots := normalizeRoots(s.discoverRoots())
	expiresAt := s.now().Add(s.rootsTTL)

	s.mu.Lock()
	s.cachedRoots = roots
	s.cacheExpires = expiresAt
	s.rootsCached = true
	s.refreshing = false
	if doneCh == nil {
		doneCh = s.refreshDone
	}
	s.refreshDone = nil
	s.mu.Unlock()
	if doneCh != nil {
		close(doneCh)
	}
}

func walkBundleRoot(root string, dirs *[]string, walked map[string]struct{}, seenBundles map[string]struct{}) error {
	info, err := os.Stat(root)
	if err != nil || !info.IsDir() {
		return nil
	}
	walkKey := root
	if resolvedRoot, err := filepath.EvalSymlinks(root); err == nil {
		walkKey = resolvedRoot
	}
	if _, ok := walked[walkKey]; ok {
		return nil
	}
	walked[walkKey] = struct{}{}

	return filepath.WalkDir(root, func(path string, d fs.DirEntry, walkErr error) error {
		if walkErr != nil {
			log.Printf("npki walk error path=%s err=%v", path, walkErr)
			return nil
		}
		if d.Type()&os.ModeSymlink != 0 {
			resolved, err := filepath.EvalSymlinks(path)
			if err != nil {
				return nil
			}
			info, err := os.Stat(resolved)
			if err != nil || !info.IsDir() {
				return nil
			}
			if isBundleDirectory(resolved) {
				if _, ok := seenBundles[resolved]; !ok {
					seenBundles[resolved] = struct{}{}
					*dirs = append(*dirs, path)
				}
				return filepath.SkipDir
			}
			if err := walkBundleRoot(path, dirs, walked, seenBundles); err != nil {
				return err
			}
			return filepath.SkipDir
		}
		if !d.IsDir() {
			return nil
		}
		if isBundleDirectory(path) {
			bundleKey := path
			if resolved, err := filepath.EvalSymlinks(path); err == nil {
				bundleKey = resolved
			}
			if _, ok := seenBundles[bundleKey]; !ok {
				seenBundles[bundleKey] = struct{}{}
				*dirs = append(*dirs, path)
			}
			return filepath.SkipDir
		}
		return nil
	})
}

func isBundleDirectory(path string) bool {
	required := []string{"signCert.der", "signPri.key", "kmCert.der", "kmPri.key"}
	for _, name := range required {
		info, err := os.Stat(filepath.Join(path, name))
		if err != nil || info.IsDir() {
			return false
		}
	}
	return true
}

func normalizeRoots(roots []string) []string {
	normalized := make([]string, 0, len(roots))
	seen := make(map[string]struct{})
	for _, root := range roots {
		root = strings.TrimSpace(root)
		if root == "" {
			continue
		}
		expanded := os.ExpandEnv(root)
		if !looksLikeWindowsPath(expanded) {
			absolute, err := filepath.Abs(expanded)
			if err != nil {
				continue
			}
			expanded = absolute
		}
		if _, ok := seen[expanded]; ok {
			continue
		}
		seen[expanded] = struct{}{}
		normalized = append(normalized, expanded)
	}
	return normalized
}

func discoverDefaultRoots() []string {
	if raw := strings.TrimSpace(os.Getenv("DESKTOP_AGENT_NPKI_DIRS")); raw != "" {
		return splitRootList(raw)
	}

	home, err := os.UserHomeDir()
	if err != nil {
		return nil
	}

	return discoverDefaultRootsFor(runtime.GOOS, home, "/Volumes")
}

func discoverDefaultRootsFor(goos string, home string, volumesRoot string) []string {
	roots := defaultLocalRootsFor(goos, home)
	if goos == "darwin" {
		roots = append(roots, discoverMountedNPKIRoots(volumesRoot)...)
	}
	return normalizeRoots(roots)
}

func defaultLocalRootsFor(goos string, home string) []string {
	switch goos {
	case "windows":
		localAppData := os.Getenv("LOCALAPPDATA")
		appData := os.Getenv("APPDATA")
		return []string{
			filepath.Join(home, "NPKI"),
			filepath.Join(home, "Documents", "NPKI"),
			filepath.Join(localAppData, "NPKI"),
			filepath.Join(appData, "NPKI"),
		}
	case "darwin":
		return []string{
			filepath.Join(home, "NPKI"),
			filepath.Join(home, "Documents", "NPKI"),
			filepath.Join(home, "Library", "Preferences", "NPKI"),
		}
	default:
		return []string{
			filepath.Join(home, "NPKI"),
			filepath.Join(home, "Documents", "NPKI"),
		}
	}
}

func discoverMountedNPKIRoots(volumesRoot string) []string {
	if strings.TrimSpace(volumesRoot) == "" {
		return nil
	}

	entries, err := os.ReadDir(volumesRoot)
	if err != nil {
		return nil
	}

	roots := make([]string, 0, len(entries))
	for _, entry := range entries {
		if !entry.IsDir() {
			continue
		}
		npkiRoot := filepath.Join(volumesRoot, entry.Name(), "NPKI")
		if info, err := os.Stat(npkiRoot); err == nil && info.IsDir() {
			roots = append(roots, npkiRoot)
		}
	}
	return roots
}

func splitRootList(value string) []string {
	fields := make([]string, 0)
	start := 0
	for i, r := range value {
		if r == ';' || r == ',' || (r == ':' && !isWindowsDriveDelimiter(value, i)) {
			fields = append(fields, value[start:i])
			start = i + 1
		}
	}
	fields = append(fields, value[start:])
	return normalizeRoots(fields)
}

func isWindowsDriveDelimiter(value string, index int) bool {
	if index <= 0 || index >= len(value) {
		return false
	}
	drive := value[index-1]
	if !unicode.IsLetter(rune(drive)) {
		return false
	}
	if index > 1 {
		prev := value[index-2]
		if prev != ';' && prev != ',' && prev != ':' && !unicode.IsSpace(rune(prev)) {
			return false
		}
	}
	if index == len(value)-1 {
		return true
	}
	next := value[index+1]
	return next == '\\' || next == '/'
}

func looksLikeWindowsPath(value string) bool {
	if len(value) >= 2 && unicode.IsLetter(rune(value[0])) && value[1] == ':' {
		return true
	}
	return strings.HasPrefix(value, `\\`)
}

type encryptedPrivateKey struct {
	Meta struct {
		Algorithm asn1.ObjectIdentifier
		Params    struct {
			Salt      []byte
			Iteration int
		}
	}
	CipherText []byte
}

type plainPrivateKey struct {
	Version    int
	Algorithm  asn1.RawValue
	PrivateKey []byte
	Extra      asn1.RawValue `asn1:"optional"`
}

func withDecryptedSignerKey(signPriKey []byte, password string, fn func(*rsa.PrivateKey) error) error {
	encrypted, err := parseEncryptedPrivateKey(signPriKey)
	if err != nil {
		return err
	}
	passwordBytes := []byte(password)
	defer zeroBytes(passwordBytes)
	key, iv, err := deriveDecryptionKey(
		encrypted.Meta.Algorithm,
		encrypted.Meta.Params.Salt,
		encrypted.Meta.Params.Iteration,
		passwordBytes,
	)
	if err != nil {
		return err
	}
	defer zeroBytes(key)
	defer zeroBytes(iv)
	plainDER, err := decryptSEEDCBC(key, iv, encrypted.CipherText)
	if err != nil {
		return ErrInvalidPassword
	}
	defer zeroBytes(plainDER[:cap(plainDER)])

	var decoded plainPrivateKey
	if _, err := asn1.Unmarshal(plainDER, &decoded); err != nil {
		return ErrInvalidPassword
	}
	defer zeroBytes(decoded.PrivateKey)
	privateKey, err := x509.ParsePKCS1PrivateKey(decoded.PrivateKey)
	if err != nil {
		return ErrInvalidPassword
	}
	return fn(privateKey)
}

func validatePassword(signPriKey []byte, password string) error {
	return withDecryptedSignerKey(signPriKey, password, func(*rsa.PrivateKey) error {
		return nil
	})
}

func signChallenge(signPriKey []byte, password string, challenge string) (string, error) {
	var encoded string
	err := withDecryptedSignerKey(signPriKey, password, func(privateKey *rsa.PrivateKey) error {
		digest := sha256.Sum256([]byte(challenge))
		signature, err := rsa.SignPKCS1v15(rand.Reader, privateKey, crypto.SHA256, digest[:])
		if err != nil {
			return fmt.Errorf("sign challenge: %w", err)
		}
		encoded = base64.StdEncoding.EncodeToString(signature)
		return nil
	})
	if err != nil {
		return "", err
	}
	return encoded, nil
}

func parseEncryptedPrivateKey(input []byte) (encryptedPrivateKey, error) {
	var decoded encryptedPrivateKey
	if _, err := asn1.Unmarshal(input, &decoded); err != nil {
		return encryptedPrivateKey{}, fmt.Errorf("parse signPri.key: %w", err)
	}
	return decoded, nil
}

func deriveDecryptionKey(algorithm asn1.ObjectIdentifier, salt []byte, iteration int, password []byte) ([]byte, []byte, error) {
	if len(salt) != 8 {
		return nil, nil, fmt.Errorf("invalid salt length %d", len(salt))
	}
	if iteration <= 0 {
		return nil, nil, fmt.Errorf("invalid iteration %d", iteration)
	}

	digest := pbkdf1SHA1(password, salt, iteration, 20)
	defer zeroBytes(digest)
	key := append([]byte{}, digest[:16]...)

	switch {
	case algorithm.Equal(oidNPKISeedCBCWithSHA1):
		sum := sha1.Sum(digest[16:20])
		return key, append([]byte{}, sum[:16]...), nil
	case algorithm.Equal(oidNPKISeedCBC):
		return key, []byte("0123456789012345"), nil
	default:
		return nil, nil, fmt.Errorf("unsupported NPKI private key algorithm")
	}
}

func zeroBytes(buf []byte) {
	clear(buf)
}

func decryptSEEDCBC(key []byte, iv []byte, cipherText []byte) ([]byte, error) {
	block, err := seedcipher.NewCipher(key)
	if err != nil {
		return nil, err
	}
	if len(cipherText)%block.BlockSize() != 0 {
		return nil, fmt.Errorf("invalid ciphertext length %d", len(cipherText))
	}

	plain := make([]byte, len(cipherText))
	defer zeroBytes(plain)
	cipher.NewCBCDecrypter(block, iv).CryptBlocks(plain, cipherText)
	return unpadPKCS7(plain, block.BlockSize())
}

func unpadPKCS7(input []byte, blockSize int) ([]byte, error) {
	if len(input) == 0 || len(input)%blockSize != 0 {
		return nil, fmt.Errorf("invalid padded length %d", len(input))
	}
	padding := int(input[len(input)-1])
	if padding == 0 || padding > blockSize || padding > len(input) {
		return nil, fmt.Errorf("invalid padding size %d", padding)
	}
	for _, value := range input[len(input)-padding:] {
		if int(value) != padding {
			return nil, errors.New("invalid PKCS7 padding")
		}
	}
	return append([]byte{}, input[:len(input)-padding]...), nil
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

func pbkdf1SHA1(password []byte, salt []byte, iterations int, length int) []byte {
	h := sha1.New()
	_, _ = h.Write(password)
	_, _ = h.Write(salt)
	digest := h.Sum(nil)
	for i := 1; i < iterations; i++ {
		h.Reset()
		_, _ = h.Write(digest)
		next := h.Sum(nil)
		zeroBytes(digest)
		digest = next
	}
	out := append([]byte{}, digest[:length]...)
	zeroBytes(digest)
	return out
}
