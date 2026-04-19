package npki

import (
	"context"
	"crypto"
	"crypto/cipher"
	"crypto/ecdsa"
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
	"math/big"
	"log"
	"os"
	"path/filepath"
	"runtime"
	"sort"
	"strings"
	"sync"
	"time"
	"unicode"

	seedcipher "github.com/deatil/go-cryptobin/cipher/seed"
	pkcs12 "software.sslmate.com/src/go-pkcs12"
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
var readDir = os.ReadDir

type ServiceConfig struct {
	Roots                []string
	AutoDiscover         bool
	RootsTTL             time.Duration
	InitialDiscoveryWait time.Duration
	DiscoveryTimeout     time.Duration
	DiscoverRoots        func(context.Context) []string
	Now                  func() time.Time
}

type CertificateMetadata struct {
	ID            string                 `json:"id"`
	CommonName    string                 `json:"commonName"`
	BusinessNo    string                 `json:"businessNo"`
	NotAfter      time.Time              `json:"notAfter"`
	SourceKind    CertificateSourceKind  `json:"sourceKind"`
	StorageKind   CertificateStorageKind `json:"storageKind"`
	DisplayPath   string                 `json:"displayPath"`
	IssuerName    string                 `json:"issuerName"`
	HasKmMaterial bool                   `json:"hasKmMaterial"`
	Removable     bool                   `json:"removable"`
}

type CertificateSourceKind string

const (
	CertificateSourceKindBundle CertificateSourceKind = "bundle"
	CertificateSourceKindPKCS12 CertificateSourceKind = "pkcs12"
)

type CertificateStorageKind string

const (
	CertificateStorageKindUnknown CertificateStorageKind = "unknown"
	CertificateStorageKindNPKI    CertificateStorageKind = "npki"
	CertificateStorageKindGPKI    CertificateStorageKind = "gpki"
	CertificateStorageKindPKCS12  CertificateStorageKind = "pkcs12"
)

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
	PasswordBytes []byte `json:"-"`
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

	rootsTTL         time.Duration
	initialWait      time.Duration
	discoveryTimeout time.Duration
	discoverRoots    func(context.Context) []string
	now              func() time.Time

	mu           sync.Mutex
	cachedRoots  []string
	cacheExpires time.Time
	rootsCached  bool
	refreshing   bool
	refreshDone  chan struct{}
}

const defaultDiscoveredRootsTTL = 60 * time.Second
const defaultInitialDiscoveryWait = 250 * time.Millisecond
const defaultDiscoveryTimeout = 2 * time.Second

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
		service.discoveryTimeout = config.DiscoveryTimeout
		if service.discoveryTimeout <= 0 {
			service.discoveryTimeout = defaultDiscoveryTimeout
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
		Roots:            roots,
		AutoDiscover:     runtime.GOOS == "darwin" || runtime.GOOS == "windows",
		RootsTTL:         defaultDiscoveredRootsTTL,
		DiscoveryTimeout: defaultDiscoveryTimeout,
		DiscoverRoots: func(ctx context.Context) []string {
			switch runtime.GOOS {
			case "darwin":
				return discoverMountedCertificateRoots(ctx, "/Volumes")
			case "windows":
				return discoverWindowsDriveNPKIRoots(ctx)
			default:
				return nil
			}
		},
	}
}

func (s *Service) ListCertificates(context.Context) ([]CertificateMetadata, error) {
	bundles, err := s.discoverBundles()
	if err != nil {
		return nil, err
	}

	items := make([]CertificateMetadata, 0, len(bundles))
	for _, bundle := range bundles {
		metadata, err := readMetadata(bundle)
		if err != nil {
			continue
		}
		items = append(items, metadata)
	}
	return items, nil
}

func (s *Service) Sign(_ context.Context, req SignRequest) (SignResult, error) {
	bundle, err := s.resolveBundle(req.CertificateID)
	if err != nil {
		return SignResult{}, err
	}
	password := append([]byte(nil), req.PasswordBytes...)
	if len(password) == 0 {
		password = []byte(req.Password)
	}
	defer zeroBytes(password)

	switch bundle.kind {
	case CertificateSourceKindBundle:
		files, metadata, err := loadBundle(bundle.path)
		if err != nil {
			return SignResult{}, err
		}
		if err := validatePassword(files.signPriKey, password); err != nil {
			return SignResult{}, err
		}
		challengeSignatureBase64, err := signChallenge(files.signPriKey, password, req.Challenge)
		if err != nil {
			return SignResult{}, err
		}

		businessNo := metadata.BusinessNo
		if businessNo == "" {
			businessNo = digitsOnly(req.BusinessNo)
		}

		return SignResult{
			CertificateID:            bundle.id,
			Challenge:                req.Challenge,
			ChallengeSignatureBase64: challengeSignatureBase64,
			Bundle: ProofBundle{
				BusinessNo:        businessNo,
				CommonName:        metadata.CommonName,
				SignCertDerBase64: base64.StdEncoding.EncodeToString(files.signCertDER),
				SignPriKeyBase64:  base64.StdEncoding.EncodeToString(files.signPriKey),
				KMCertDerBase64:   base64.StdEncoding.EncodeToString(files.kmCertDER),
				KMPriKeyBase64:    base64.StdEncoding.EncodeToString(files.kmPriKey),
			},
		}, nil
	case CertificateSourceKindPKCS12:
		files, metadata, err := loadPKCS12(bundle.path, string(password))
		if err != nil {
			return SignResult{}, err
		}
		challengeSignatureBase64, err := signChallengePKCS12(files.signPrivateKey, req.Challenge)
		if err != nil {
			return SignResult{}, err
		}

		businessNo := metadata.BusinessNo
		if businessNo == "" {
			businessNo = digitsOnly(req.BusinessNo)
		}

		return SignResult{
			CertificateID:            bundle.id,
			Challenge:                req.Challenge,
			ChallengeSignatureBase64: challengeSignatureBase64,
			Bundle: ProofBundle{
				BusinessNo:        businessNo,
				CommonName:        metadata.CommonName,
				SignCertDerBase64: base64.StdEncoding.EncodeToString(files.signCertDER),
				SignPriKeyBase64:  base64.StdEncoding.EncodeToString(files.signPriKeyDER),
			},
		}, nil
	default:
		return SignResult{}, ErrCertificateNotFound
	}
}

type bundleFiles struct {
	signCertDER []byte
	signPriKey  []byte
	kmCertDER   []byte
	kmPriKey    []byte
}

type pkcs12Files struct {
	signCertDER    []byte
	signPriKeyDER  []byte
	signPrivateKey crypto.Signer
}

type bundleEntry struct {
	id        string
	kind      CertificateSourceKind
	path      string
	storage   CertificateStorageKind
	removable bool
	aliases   []string
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

	metadata, err := metadataFromCertificate(newBundleEntry(dir), signCertDER)
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

func readMetadata(bundle bundleEntry) (CertificateMetadata, error) {
	switch bundle.kind {
	case CertificateSourceKindBundle:
		signCertDER, err := os.ReadFile(filepath.Join(bundle.path, "signCert.der"))
		if err != nil {
			return CertificateMetadata{}, err
		}
		return metadataFromCertificate(bundle, signCertDER)
	case CertificateSourceKindPKCS12:
		return metadataFromPKCS12(bundle)
	default:
		return CertificateMetadata{}, ErrCertificateNotFound
	}
}

func metadataFromCertificate(bundle bundleEntry, signCertDER []byte) (CertificateMetadata, error) {
	cert, err := x509.ParseCertificate(signCertDER)
	if err != nil {
		return CertificateMetadata{}, fmt.Errorf("parse signCert.der: %w", err)
	}

	return CertificateMetadata{
		ID:            bundle.id,
		CommonName:    strings.TrimSpace(cert.Subject.CommonName),
		BusinessNo:    extractBusinessNo(cert),
		NotAfter:      cert.NotAfter.UTC(),
		SourceKind:    CertificateSourceKindBundle,
		StorageKind:   bundle.storage,
		DisplayPath:   bundleDisplayPath(bundle.path),
		IssuerName:    strings.TrimSpace(cert.Issuer.CommonName),
		HasKmMaterial: true,
		Removable:     bundle.removable,
	}, nil
}

func metadataFromPKCS12(bundle bundleEntry) (CertificateMetadata, error) {
	files, cert, _, err := loadPKCS12Source(bundle.path, "")
	if err != nil {
		if errors.Is(err, ErrInvalidPassword) {
			return fallbackPKCS12Metadata(bundle), nil
		}
		return CertificateMetadata{}, err
	}
	_ = files
	return CertificateMetadata{
		ID:            bundle.id,
		CommonName:    strings.TrimSpace(cert.Subject.CommonName),
		BusinessNo:    extractBusinessNo(cert),
		NotAfter:      cert.NotAfter.UTC(),
		SourceKind:    CertificateSourceKindPKCS12,
		StorageKind:   CertificateStorageKindPKCS12,
		DisplayPath:   bundleDisplayPath(bundle.path),
		IssuerName:    strings.TrimSpace(cert.Issuer.CommonName),
		HasKmMaterial: false,
		Removable:     bundle.removable,
	}, nil
}

func fallbackPKCS12Metadata(bundle bundleEntry) CertificateMetadata {
	return CertificateMetadata{
		ID:            bundle.id,
		CommonName:    fallbackPKCS12Label(bundle.path),
		BusinessNo:    "",
		NotAfter:      time.Time{},
		SourceKind:    CertificateSourceKindPKCS12,
		StorageKind:   CertificateStorageKindPKCS12,
		DisplayPath:   bundleDisplayPath(bundle.path),
		IssuerName:    "",
		HasKmMaterial: false,
		Removable:     bundle.removable,
	}
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

func (s *Service) resolveBundle(id string) (bundleEntry, error) {
	id = strings.TrimSpace(id)
	if id == "" {
		return bundleEntry{}, ErrCertificateNotFound
	}
	bundles, err := s.discoverBundles()
	if err != nil {
		return bundleEntry{}, err
	}
	for _, bundle := range bundles {
		if bundle.id == id || bundle.path == id {
			return bundle, nil
		}
		for _, alias := range bundle.aliases {
			if alias == id {
				return bundle, nil
			}
		}
		if canonicalID, _ := canonicalizeBundlePath(id); canonicalID == bundle.path {
			return bundle, nil
		}
	}
	return bundleEntry{}, ErrCertificateNotFound
}

func (s *Service) discoverBundles() ([]bundleEntry, error) {
	dirs := make([]bundleEntry, 0)
	walked := make(map[string]struct{})
	seenBundles := make(map[string]struct{})

	for _, root := range s.bundleRoots() {
		if err := walkBundleRoot(root, &dirs, walked, seenBundles); err != nil {
			return nil, err
		}
	}

	sort.Slice(dirs, func(i, j int) bool {
		return dirs[i].id < dirs[j].id
	})
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
		go s.refreshDiscoveredRoots()
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

func (s *Service) refreshDiscoveredRoots() {
	ctx, cancel := context.WithTimeout(context.Background(), s.discoveryTimeout)
	defer cancel()

	roots, completed := s.runDiscoveredRoots(ctx)
	expiresAt := s.now().Add(s.rootsTTL)

	s.mu.Lock()
	doneCh := s.refreshDone
	if completed {
		s.cachedRoots = roots
		s.cacheExpires = expiresAt
		s.rootsCached = true
	} else if s.rootsCached {
		s.cacheExpires = expiresAt
	}
	s.refreshing = false
	s.refreshDone = nil
	s.mu.Unlock()
	if doneCh != nil {
		close(doneCh)
	}
}

func (s *Service) runDiscoveredRoots(ctx context.Context) ([]string, bool) {
	type discoveryResult struct {
		roots []string
		ok    bool
	}

	results := make(chan discoveryResult, 1)
	go func() {
		defer func() {
			if recovered := recover(); recovered != nil {
				log.Printf("npki discover roots panic: %v", recovered)
				select {
				case results <- discoveryResult{}:
				case <-ctx.Done():
				}
			}
		}()

		roots := normalizeRoots(s.discoverRoots(ctx))
		select {
		case results <- discoveryResult{roots: roots, ok: true}:
		case <-ctx.Done():
		}
	}()

	select {
	case result := <-results:
		return result.roots, result.ok
	case <-ctx.Done():
		log.Printf("npki discover roots timed out after %s", s.discoveryTimeout)
		return nil, false
	}
}

func walkBundleRoot(root string, dirs *[]bundleEntry, walked map[string]struct{}, seenBundles map[string]struct{}) error {
	info, err := os.Stat(root)
	if err != nil {
		return nil
	}
	if !info.IsDir() {
		if isPKCS12FileName(root) {
			entry := newPKCS12Entry(root)
			if _, ok := seenBundles[entry.path]; !ok {
				seenBundles[entry.path] = struct{}{}
				*dirs = append(*dirs, entry)
			}
		}
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
				if err == nil && isPKCS12FileName(resolved) {
					entry := newPKCS12Entry(path)
					if _, ok := seenBundles[entry.path]; !ok {
						seenBundles[entry.path] = struct{}{}
						*dirs = append(*dirs, entry)
					}
				}
				return nil
			}
			if isBundleDirectory(resolved) {
				if _, ok := seenBundles[resolved]; !ok {
					seenBundles[resolved] = struct{}{}
					*dirs = append(*dirs, newBundleEntry(path))
				}
				return filepath.SkipDir
			}
			if err := walkBundleRoot(path, dirs, walked, seenBundles); err != nil {
				return err
			}
			return filepath.SkipDir
		}
		if !d.IsDir() {
			if isPKCS12FileName(path) {
				entry := newPKCS12Entry(path)
				if _, ok := seenBundles[entry.path]; !ok {
					seenBundles[entry.path] = struct{}{}
					*dirs = append(*dirs, entry)
				}
			}
			return nil
		}
		if isBundleDirectory(path) {
			bundleKey := path
			if resolved, err := filepath.EvalSymlinks(path); err == nil {
				bundleKey = resolved
			}
			if _, ok := seenBundles[bundleKey]; !ok {
				seenBundles[bundleKey] = struct{}{}
				*dirs = append(*dirs, newBundleEntry(path))
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

func newBundleEntry(dir string) bundleEntry {
	cleanDir, alias := canonicalizeBundlePath(dir)
	storage := storageKindForBundleDir(cleanDir)
	aliases := make([]string, 0, 1)
	if alias != "" {
		aliases = append(aliases, alias)
	}
	return bundleEntry{
		id:        string(CertificateSourceKindBundle) + ":" + cleanDir,
		kind:      CertificateSourceKindBundle,
		path:      cleanDir,
		storage:   storage,
		removable: pathIsOnRemovableMedia(cleanDir),
		aliases:   aliases,
	}
}

func newPKCS12Entry(path string) bundleEntry {
	cleanPath, alias := canonicalizeBundlePath(path)
	aliases := make([]string, 0, 1)
	if alias != "" {
		aliases = append(aliases, alias)
	}
	return bundleEntry{
		id:        string(CertificateSourceKindPKCS12) + ":" + cleanPath,
		kind:      CertificateSourceKindPKCS12,
		path:      cleanPath,
		storage:   CertificateStorageKindPKCS12,
		removable: pathIsOnRemovableMedia(cleanPath),
		aliases:   aliases,
	}
}

func canonicalizeBundlePath(path string) (canonical string, alias string) {
	cleanPath := filepath.Clean(path)
	canonical = cleanPath
	if resolved, err := filepath.EvalSymlinks(cleanPath); err == nil {
		canonical = filepath.Clean(resolved)
	}
	if canonical != cleanPath {
		alias = cleanPath
	}
	return canonical, alias
}

func storageKindForBundleDir(dir string) CertificateStorageKind {
	for _, part := range splitPathParts(filepath.Clean(dir)) {
		switch strings.ToUpper(part) {
		case "NPKI":
			return CertificateStorageKindNPKI
		case "GPKI":
			return CertificateStorageKindGPKI
		}
	}
	return CertificateStorageKindUnknown
}

func bundleDisplayPath(dir string) string {
	cleanDir := filepath.Clean(dir)
	parts := splitPathParts(cleanDir)
	if len(parts) == 0 {
		return filepath.Base(cleanDir)
	}
	for i, part := range parts {
		switch strings.ToUpper(part) {
		case "NPKI", "GPKI":
			return strings.Join(parts[i:], "/")
		}
	}
	return filepath.Base(cleanDir)
}

func splitPathParts(path string) []string {
	parts := make([]string, 0)
	for _, part := range strings.Split(filepath.Clean(path), string(os.PathSeparator)) {
		if part == "" {
			continue
		}
		parts = append(parts, part)
	}
	return parts
}

func pathIsOnRemovableMedia(path string) bool {
	cleanPath := strings.ToLower(filepath.Clean(path))
	if strings.HasPrefix(cleanPath, "/volumes/") {
		return true
	}
	return windowsPathIsOnRemovableMedia(path)
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

func discoverDefaultRoots(ctx context.Context) []string {
	if raw := strings.TrimSpace(os.Getenv("DESKTOP_AGENT_NPKI_DIRS")); raw != "" {
		return splitRootList(raw)
	}

	home, err := os.UserHomeDir()
	if err != nil {
		return nil
	}

	return discoverDefaultRootsFor(ctx, runtime.GOOS, home, "/Volumes")
}

func discoverDefaultRootsFor(ctx context.Context, goos string, home string, volumesRoot string) []string {
	roots := defaultLocalRootsFor(goos, home)
	switch goos {
	case "darwin":
		roots = append(roots, discoverMountedCertificateRoots(ctx, volumesRoot)...)
	case "windows":
		roots = append(roots, discoverWindowsDriveNPKIRoots(ctx)...)
	}
	return normalizeRoots(roots)
}

func defaultLocalRootsFor(goos string, home string) []string {
	switch goos {
	case "windows":
		roots := []string{
			filepath.Join(home, "NPKI"),
			filepath.Join(home, "GPKI"),
			filepath.Join(home, "Documents", "NPKI"),
			filepath.Join(home, "Documents", "GPKI"),
			filepath.Join(home, "AppData", "LocalLow", "NPKI"),
			filepath.Join(home, "AppData", "LocalLow", "GPKI"),
		}
		if pf := os.Getenv("ProgramFiles"); pf != "" {
			roots = append(roots, filepath.Join(pf, "NPKI"))
			roots = append(roots, filepath.Join(pf, "GPKI"))
		}
		if pfx86 := os.Getenv("ProgramFiles(x86)"); pfx86 != "" {
			roots = append(roots, filepath.Join(pfx86, "NPKI"))
			roots = append(roots, filepath.Join(pfx86, "GPKI"))
		}
		return roots
	case "darwin":
		return []string{
			filepath.Join(home, "NPKI"),
			filepath.Join(home, "GPKI"),
			filepath.Join(home, "Documents", "NPKI"),
			filepath.Join(home, "Documents", "GPKI"),
			filepath.Join(home, "Library", "Preferences", "NPKI"),
			filepath.Join(home, "Library", "Preferences", "GPKI"),
		}
	default:
		return []string{
			filepath.Join(home, "NPKI"),
			filepath.Join(home, "GPKI"),
			filepath.Join(home, "Documents", "NPKI"),
			filepath.Join(home, "Documents", "GPKI"),
		}
	}
}

func discoverMountedCertificateRoots(ctx context.Context, volumesRoot string) []string {
	if strings.TrimSpace(volumesRoot) == "" {
		return nil
	}
	if err := ctx.Err(); err != nil {
		return nil
	}

	entries, err := os.ReadDir(volumesRoot)
	if err != nil {
		return nil
	}

	roots := make([]string, 0, len(entries))
	for _, entry := range entries {
		if err := ctx.Err(); err != nil {
			return roots
		}
		mountRoot := filepath.Join(volumesRoot, entry.Name())
		info, err := os.Stat(mountRoot)
		if err != nil || !info.IsDir() {
			continue
		}
		if mountContainsCertificateSources(mountRoot) {
			roots = append(roots, mountRoot)
		}
	}
	return roots
}

func mountContainsCertificateSources(mountRoot string) bool {
	for _, storageRootName := range []string{"NPKI", "GPKI"} {
		childRoot := filepath.Join(mountRoot, storageRootName)
		if info, err := os.Stat(childRoot); err == nil && info.IsDir() {
			return true
		}
	}

	entries, err := readDir(mountRoot)
	if err != nil {
		return false
	}

	for _, entry := range entries {
		if entry.IsDir() {
			continue
		}
		if isPKCS12FileName(entry.Name()) {
			return true
		}
	}
	return false
}

func isPKCS12FileName(name string) bool {
	switch strings.ToLower(filepath.Ext(name)) {
	case ".p12", ".pfx":
		return true
	default:
		return false
	}
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

func withDecryptedSignerKey(signPriKey []byte, password []byte, fn func(*rsa.PrivateKey) error) error {
	encrypted, err := parseEncryptedPrivateKey(signPriKey)
	if err != nil {
		return err
	}
	key, iv, err := deriveDecryptionKey(
		encrypted.Meta.Algorithm,
		encrypted.Meta.Params.Salt,
		encrypted.Meta.Params.Iteration,
		password,
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
	defer zeroRSAPrivateKey(privateKey)
	return fn(privateKey)
}

func validatePassword(signPriKey []byte, password []byte) error {
	return withDecryptedSignerKey(signPriKey, password, func(*rsa.PrivateKey) error {
		return nil
	})
}

func signChallenge(signPriKey []byte, password []byte, challenge string) (string, error) {
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

func loadPKCS12Source(path string, password string) (pkcs12Files, *x509.Certificate, []*x509.Certificate, error) {
	data, err := os.ReadFile(path)
	if err != nil {
		return pkcs12Files{}, nil, nil, err
	}
	privateKey, cert, chain, err := pkcs12.DecodeChain(data, password)
	if err != nil {
		if errors.Is(err, pkcs12.ErrIncorrectPassword) {
			return pkcs12Files{}, nil, nil, ErrInvalidPassword
		}
		return pkcs12Files{}, nil, nil, fmt.Errorf("decode pkcs12: %w", err)
	}
	if cert == nil {
		return pkcs12Files{}, nil, nil, fmt.Errorf("decode pkcs12: certificate missing")
	}
	if privateKey == nil {
		return pkcs12Files{}, nil, nil, fmt.Errorf("decode pkcs12: private key missing")
	}
	signer, err := supportedPKCS12Signer(privateKey)
	if err != nil {
		return pkcs12Files{}, nil, nil, err
	}
	signPriKeyDER, err := x509.MarshalPKCS8PrivateKey(privateKey)
	if err != nil {
		return pkcs12Files{}, nil, nil, fmt.Errorf("marshal pkcs12 private key as PKCS#8: %w", err)
	}
	return pkcs12Files{
		signCertDER:    append([]byte(nil), cert.Raw...),
		signPriKeyDER:  signPriKeyDER,
		signPrivateKey: signer,
	}, cert, chain, nil
}

func supportedPKCS12Signer(privateKey any) (crypto.Signer, error) {
	switch key := privateKey.(type) {
	case *rsa.PrivateKey:
		return key, nil
	case *ecdsa.PrivateKey:
		return key, nil
	default:
		return nil, fmt.Errorf("decode pkcs12: unsupported private key type %T", privateKey)
	}
}

func loadPKCS12(path string, password string) (pkcs12Files, CertificateMetadata, error) {
	files, cert, _, err := loadPKCS12Source(path, password)
	if err != nil {
		return pkcs12Files{}, CertificateMetadata{}, err
	}
	metadata := CertificateMetadata{
		ID:            string(CertificateSourceKindPKCS12) + ":" + canonicalPath(path),
		CommonName:    strings.TrimSpace(cert.Subject.CommonName),
		BusinessNo:    extractBusinessNo(cert),
		NotAfter:      cert.NotAfter.UTC(),
		SourceKind:    CertificateSourceKindPKCS12,
		StorageKind:   CertificateStorageKindPKCS12,
		DisplayPath:   bundleDisplayPath(path),
		IssuerName:    strings.TrimSpace(cert.Issuer.CommonName),
		HasKmMaterial: false,
	}
	return files, metadata, nil
}

func signChallengePKCS12(privateKey crypto.Signer, challenge string) (string, error) {
	digest := sha256.Sum256([]byte(challenge))
	signature, err := privateKey.Sign(rand.Reader, digest[:], crypto.SHA256)
	if err != nil {
		return "", fmt.Errorf("sign challenge: %w", err)
	}
	return base64.StdEncoding.EncodeToString(signature), nil
}

func canonicalPath(path string) string {
	canonical, _ := canonicalizeBundlePath(path)
	return canonical
}

func fallbackPKCS12Label(path string) string {
	base := filepath.Base(path)
	if ext := filepath.Ext(base); ext != "" {
		base = strings.TrimSuffix(base, ext)
	}
	base = strings.TrimSpace(base)
	if base == "" || base == "." || base == string(os.PathSeparator) {
		return filepath.Base(path)
	}
	return base
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

func zeroRSAPrivateKey(privateKey *rsa.PrivateKey) {
	if privateKey == nil {
		return
	}
	zeroBigInt(privateKey.D)
	zeroBigInt(privateKey.N)
	for i := range privateKey.Primes {
		zeroBigInt(privateKey.Primes[i])
	}
	zeroBigInt(privateKey.Precomputed.Dp)
	zeroBigInt(privateKey.Precomputed.Dq)
	zeroBigInt(privateKey.Precomputed.Qinv)
	for i := range privateKey.Precomputed.CRTValues {
		zeroBigInt(privateKey.Precomputed.CRTValues[i].Exp)
		zeroBigInt(privateKey.Precomputed.CRTValues[i].Coeff)
		zeroBigInt(privateKey.Precomputed.CRTValues[i].R)
	}
}

func zeroBigInt(value *big.Int) {
	if value == nil {
		return
	}
	// Best-effort wipe: this relies on math/big's current Bits() representation
	// exposing the backing words. The runtime may still hold copies elsewhere.
	words := value.Bits()
	for i := range words {
		words[i] = 0
	}
	value.SetBits(words[:0])
	runtime.KeepAlive(words)
	runtime.KeepAlive(value)
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
