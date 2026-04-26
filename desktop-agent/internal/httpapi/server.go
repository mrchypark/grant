package httpapi

import (
	"bytes"
	"context"
	"crypto/hmac"
	"crypto/rand"
	"crypto/sha256"
	"encoding/base64"
	"encoding/json"
	"errors"
	"fmt"
	"io"
	"net"
	"net/http"
	"net/url"
	"os/exec"
	"strings"
	"time"

	"desktop-agent/internal/npki"
)

type npkiService interface {
	ListCertificates(context.Context) ([]npki.CertificateMetadata, error)
	Sign(context.Context, npki.SignRequest) (npki.SignResult, error)
}

type approvalPrompter interface {
	PromptApproval(context.Context, ApprovalPrompt) (ApprovalDecision, error)
}

type gatewayExecutor interface {
	CreateChallenge(context.Context, GatewayChallengeRequest) (challengeResponse, error)
	RegisterStored(context.Context, GatewayStoredRegistrationRequest) (map[string]any, error)
}

type localExecutor interface {
	RunEphemeral(context.Context, LocalEphemeralRequest) (map[string]any, error)
}

type Config struct {
	AllowedOrigins []string
	GatewayBaseURL string
	SessionTTL     time.Duration
	SessionSecret  []byte
}

var ErrApprovalRejected = errors.New("approval rejected")
var ErrLocalExecutorUnavailable = errors.New("local executor is not configured")
var errMethodNotAllowed = errors.New("method not allowed")

const maxJSONBodyBytes int64 = 1024 * 1024
const maxLocalExecutorBytes = 64 * 1024 * 1024
const maxLocalExecutorStderr = 64 * 1024
const gatewayRequestTimeout = 5 * time.Minute
const localExecutorTimeout = 5 * time.Minute

type httpStatusError struct {
	status  int
	message string
}

func (e *httpStatusError) Error() string {
	return e.message
}

func newHTTPStatusError(status int, message string) error {
	return &httpStatusError{status: status, message: message}
}

func zeroBytes(buf []byte) {
	clear(buf)
}

type ApprovalPrompt struct {
	Origin         string
	Action         string
	OrganizationID string
	BusinessNo     string
	Certificates   []npki.CertificateMetadata
}

type ApprovalDecision struct {
	CertificateID string
	Password      []byte
}

type ApprovalActionRequest struct {
	GatewayBaseURL       string `json:"gatewayBaseUrl"`
	OrganizationID       string `json:"organizationId"`
	BusinessNo           string `json:"businessNo,omitempty"`
	BearerToken          string `json:"bearerToken,omitempty"`
	ActiveOrganizationID string `json:"activeOrganizationId,omitempty"`
	FacilityCode         string `json:"facilityCode,omitempty"`
	StartMonth           string `json:"startMonth,omitempty"`
	EndMonth             string `json:"endMonth,omitempty"`
	Mode                 string `json:"mode,omitempty"`
}

type GatewayStoredRegistrationRequest struct {
	BaseURL                  string
	OrganizationID           string
	BusinessNo               string
	Password                 string
	BearerToken              string
	ActiveOrganizationID     string
	ChallengeID              string
	ChallengePayload         string
	ChallengeSignatureBase64 string
	Bundle                   npki.ProofBundle
}

type LocalEphemeralRequest struct {
	BaseURL                  string           `json:"gatewayBaseUrl"`
	OrganizationID           string           `json:"organizationId"`
	BusinessNo               string           `json:"businessNo,omitempty"`
	Password                 string           `json:"password"`
	BearerToken              string           `json:"bearerToken,omitempty"`
	ActiveOrganizationID     string           `json:"activeOrganizationId,omitempty"`
	ChallengeID              string           `json:"challengeId"`
	ChallengePayload         string           `json:"challengePayload"`
	ChallengeSignatureBase64 string           `json:"challengeSignatureBase64"`
	FacilityCode             string           `json:"facilityCode,omitempty"`
	StartMonth               string           `json:"startMonth,omitempty"`
	EndMonth                 string           `json:"endMonth,omitempty"`
	Mode                     string           `json:"mode,omitempty"`
	Bundle                   npki.ProofBundle `json:"bundle"`
}

type GatewayChallengeRequest struct {
	BaseURL              string
	OrganizationID       string
	BearerToken          string
	ActiveOrganizationID string
	Mode                 string
	SyncMode             string
	Origin               string
	BusinessNo           string
	FacilityCode         string
	StartMonth           string
	EndMonth             string
	SignCertSHA256Hex    string
}

type Server struct {
	npki           npkiService
	approver       approvalPrompter
	gateway        gatewayExecutor
	local          localExecutor
	gatewayBaseURL string
	mux            *http.ServeMux
	allowedOrigins map[string]struct{}
	sessions       sessionManager
}

type sessionManager struct {
	secret []byte
	ttl    time.Duration
}

type sessionPayload struct {
	Origin string `json:"origin"`
	Expiry string `json:"expiry"`
	Nonce  string `json:"nonce"`
}

func NewServer(service npkiService, approver approvalPrompter, gateway gatewayExecutor, local localExecutor, cfg Config) *Server {
	if local == nil {
		local = unavailableLocalExecutor{}
	}
	server := &Server{
		npki:           service,
		approver:       approver,
		gateway:        gateway,
		local:          local,
		gatewayBaseURL: strings.TrimRight(strings.TrimSpace(cfg.GatewayBaseURL), "/"),
		mux:            http.NewServeMux(),
		allowedOrigins: allowlistFromConfig(cfg.AllowedOrigins),
		sessions:       newSessionManager(cfg.SessionSecret, cfg.SessionTTL),
	}
	server.routes()
	return server
}

func (s *Server) Handler() http.Handler {
	return withCORS(s, s.mux)
}

func (s *Server) routes() {
	s.mux.HandleFunc("/healthz", s.handleHealthz)
	s.mux.HandleFunc("/sessions/open", s.handleOpenSession)
	s.mux.HandleFunc("/certificates", s.handleCertificates)
	s.mux.HandleFunc("/sign", s.handleSign)
	s.mux.HandleFunc("/approve-ephemeral-run", s.handleApproveEphemeralRun)
	s.mux.HandleFunc("/approve-stored-registration", s.handleApproveStoredRegistration)
}

func (s *Server) handleHealthz(w http.ResponseWriter, r *http.Request) {
	if r.Method != http.MethodGet {
		writeMethodNotAllowed(w, http.MethodGet)
		return
	}
	w.Header().Set("Content-Type", "text/plain; charset=utf-8")
	w.WriteHeader(http.StatusOK)
	_, _ = w.Write([]byte("ok"))
}

func (s *Server) handleCertificates(w http.ResponseWriter, r *http.Request) {
	if r.Method != http.MethodGet {
		writeMethodNotAllowed(w, http.MethodGet)
		return
	}
	if _, ok := s.requireSession(w, r); !ok {
		return
	}
	certificates, err := s.npki.ListCertificates(r.Context())
	if err != nil {
		writeError(w, http.StatusInternalServerError, err.Error())
		return
	}
	writeJSON(w, http.StatusOK, map[string]any{"items": certificates})
}

func (s *Server) handleSign(w http.ResponseWriter, r *http.Request) {
	if r.Method != http.MethodPost {
		writeMethodNotAllowed(w, http.MethodPost)
		return
	}
	if _, ok := s.requireSession(w, r); !ok {
		return
	}
	defer r.Body.Close()
	r.Body = http.MaxBytesReader(w, r.Body, maxJSONBodyBytes)

	var req npki.SignRequest
	if err := json.NewDecoder(r.Body).Decode(&req); err != nil {
		writeError(w, http.StatusBadRequest, "invalid json body")
		return
	}
	req.CertificateID = strings.TrimSpace(req.CertificateID)
	req.Challenge = strings.TrimSpace(req.Challenge)

	switch {
	case req.CertificateID == "":
		writeError(w, http.StatusBadRequest, "certificateId must not be empty")
		return
	case req.Challenge == "":
		writeError(w, http.StatusBadRequest, "challenge must not be empty")
		return
	case !strings.HasPrefix(req.Challenge, "cert-gateway:"):
		writeError(w, http.StatusBadRequest, "challenge must be a cert-gateway payload")
		return
	case strings.TrimSpace(req.Password) == "":
		if req.Password == "" && strings.HasPrefix(req.CertificateID, "pkcs12:") {
			break
		}
		writeError(w, http.StatusBadRequest, "password must not be empty")
		return
	}

	result, err := s.npki.Sign(r.Context(), req)
	if err != nil {
		status := http.StatusInternalServerError
		switch {
		case errors.Is(err, npki.ErrNotImplemented):
			status = http.StatusNotImplemented
		case errors.Is(err, npki.ErrCertificateNotFound):
			status = http.StatusNotFound
		case errors.Is(err, npki.ErrInvalidPassword):
			status = http.StatusUnauthorized
		}
		writeError(w, status, err.Error())
		return
	}
	writeJSON(w, http.StatusOK, signResponseFromResult(result))
}

func (s *Server) handleOpenSession(w http.ResponseWriter, r *http.Request) {
	if r.Method != http.MethodPost {
		writeMethodNotAllowed(w, http.MethodPost)
		return
	}
	origin, ok := s.requireOrigin(w, r)
	if !ok {
		return
	}
	sessionToken, expiresAt, err := s.sessions.issue(origin, time.Now().UTC())
	if err != nil {
		writeError(w, http.StatusInternalServerError, err.Error())
		return
	}
	writeJSON(w, http.StatusCreated, map[string]string{
		"sessionToken": sessionToken,
		"expiresAt":    expiresAt.Format(time.RFC3339),
	})
}

func (s *Server) handleApproveEphemeralRun(w http.ResponseWriter, r *http.Request) {
	r.Body = http.MaxBytesReader(w, r.Body, maxJSONBodyBytes)
	result, status, err := s.handleApprovedAction(r, "ephemeral")
	if err != nil {
		writeApprovalError(w, err)
		return
	}
	writeJSON(w, status, result)
}

func (s *Server) handleApproveStoredRegistration(w http.ResponseWriter, r *http.Request) {
	r.Body = http.MaxBytesReader(w, r.Body, maxJSONBodyBytes)
	result, status, err := s.handleApprovedAction(r, "stored")
	if err != nil {
		writeApprovalError(w, err)
		return
	}
	writeJSON(w, status, result)
}

func (s *Server) handleApprovedAction(r *http.Request, action string) (map[string]any, int, error) {
	if r.Method != http.MethodPost {
		return nil, 0, newHTTPStatusError(http.StatusMethodNotAllowed, errMethodNotAllowed.Error())
	}
	if err := s.validateSession(r); err != nil {
		return nil, 0, err
	}
	defer r.Body.Close()

	var req ApprovalActionRequest
	if err := json.NewDecoder(r.Body).Decode(&req); err != nil {
		return nil, 0, newHTTPStatusError(http.StatusBadRequest, "invalid json body")
	}
	req.GatewayBaseURL = strings.TrimSpace(req.GatewayBaseURL)
	req.OrganizationID = strings.TrimSpace(req.OrganizationID)
	req.BusinessNo = strings.TrimSpace(req.BusinessNo)
	req.BearerToken = strings.TrimSpace(req.BearerToken)
	req.ActiveOrganizationID = strings.TrimSpace(req.ActiveOrganizationID)
	req.FacilityCode = strings.TrimSpace(req.FacilityCode)
	req.StartMonth = strings.TrimSpace(req.StartMonth)
	req.EndMonth = strings.TrimSpace(req.EndMonth)
	req.Mode = strings.TrimSpace(req.Mode)

	if req.OrganizationID == "" {
		return nil, 0, newHTTPStatusError(http.StatusBadRequest, "organizationId must not be empty")
	}
	gatewayBaseURL, err := s.trustedGatewayBaseURL(req.GatewayBaseURL)
	if err != nil {
		return nil, 0, err
	}
	origin := strings.TrimSpace(r.Header.Get("Origin"))
	certificates, err := s.npki.ListCertificates(r.Context())
	if err != nil {
		return nil, 0, err
	}
	decision, err := s.approver.PromptApproval(r.Context(), ApprovalPrompt{
		Origin:         origin,
		Action:         action,
		OrganizationID: req.OrganizationID,
		BusinessNo:     req.BusinessNo,
		Certificates:   certificates,
	})
	if err != nil {
		return nil, 0, err
	}
	defer zeroBytes(decision.Password)
	selectedCertificate, err := findCertificateByID(certificates, decision.CertificateID)
	if err != nil {
		return nil, 0, err
	}
	if err := validateSignedBusinessNo(req.BusinessNo, selectedCertificate.BusinessNo); err != nil {
		return nil, 0, err
	}
	challenge, err := s.gateway.CreateChallenge(r.Context(), GatewayChallengeRequest{
		BaseURL:              gatewayBaseURL,
		OrganizationID:       req.OrganizationID,
		BearerToken:          req.BearerToken,
		ActiveOrganizationID: req.ActiveOrganizationID,
		Mode:                 action,
		SyncMode:             req.Mode,
		Origin:               origin,
		BusinessNo:           req.BusinessNo,
		FacilityCode:         req.FacilityCode,
		StartMonth:           req.StartMonth,
		EndMonth:             req.EndMonth,
		SignCertSHA256Hex:    selectedCertificate.SignCertSHA256Hex,
	})
	if err != nil {
		return nil, 0, err
	}
	signResult, err := s.npki.Sign(r.Context(), npki.SignRequest{
		CertificateID: decision.CertificateID,
		Challenge:     challenge.Payload,
		PasswordBytes: decision.Password,
		BusinessNo:    req.BusinessNo,
	})
	if err != nil {
		return nil, 0, err
	}
	if err := validateSignedBusinessNo(req.BusinessNo, signResult.Bundle.BusinessNo); err != nil {
		return nil, 0, err
	}

	switch action {
	case "ephemeral":
		localPassword := string(decision.Password)
		syncMode := firstNonEmpty(challenge.SyncMode, req.Mode)
		startMonth := firstNonEmpty(challenge.StartMonth, req.StartMonth)
		endMonth := firstNonEmpty(challenge.EndMonth, req.EndMonth)
		body, err := s.local.RunEphemeral(r.Context(), LocalEphemeralRequest{
			BaseURL:                  gatewayBaseURL,
			OrganizationID:           req.OrganizationID,
			BusinessNo:               req.BusinessNo,
			Password:                 localPassword,
			BearerToken:              req.BearerToken,
			ActiveOrganizationID:     req.ActiveOrganizationID,
			ChallengeID:              challenge.ChallengeID,
			ChallengePayload:         challenge.Payload,
			ChallengeSignatureBase64: signResult.ChallengeSignatureBase64,
			FacilityCode:             req.FacilityCode,
			StartMonth:               startMonth,
			EndMonth:                 endMonth,
			Mode:                     syncMode,
			Bundle:                   signResult.Bundle,
		})
		return body, http.StatusAccepted, err
	case "stored":
		gatewayPassword := string(decision.Password)
		body, err := s.gateway.RegisterStored(r.Context(), GatewayStoredRegistrationRequest{
			BaseURL:                  gatewayBaseURL,
			OrganizationID:           req.OrganizationID,
			BusinessNo:               req.BusinessNo,
			Password:                 gatewayPassword,
			BearerToken:              req.BearerToken,
			ActiveOrganizationID:     req.ActiveOrganizationID,
			ChallengeID:              challenge.ChallengeID,
			ChallengePayload:         challenge.Payload,
			ChallengeSignatureBase64: signResult.ChallengeSignatureBase64,
			Bundle:                   signResult.Bundle,
		})
		return body, http.StatusCreated, err
	default:
		return nil, 0, errors.New("unsupported approval action")
	}
}

func writeMethodNotAllowed(w http.ResponseWriter, method string) {
	w.Header().Set("Allow", method)
	writeError(w, http.StatusMethodNotAllowed, "method not allowed")
}

func writeJSON(w http.ResponseWriter, status int, value any) {
	w.Header().Set("Content-Type", "application/json")
	w.WriteHeader(status)
	_ = json.NewEncoder(w).Encode(value)
}

func writeError(w http.ResponseWriter, status int, message string) {
	writeJSON(w, status, map[string]string{"error": message})
}

func signResponseFromResult(result npki.SignResult) map[string]any {
	return map[string]any{
		"certificateId":            result.CertificateID,
		"challenge":                result.Challenge,
		"challengeSignatureBase64": result.ChallengeSignatureBase64,
		"certificate": map[string]any{
			"businessNo":        result.Bundle.BusinessNo,
			"commonName":        result.Bundle.CommonName,
			"signCertDerBase64": result.Bundle.SignCertDerBase64,
		},
	}
}

func writeApprovalError(w http.ResponseWriter, err error) {
	var statusErr *httpStatusError
	if errors.As(err, &statusErr) {
		writeError(w, statusErr.status, statusErr.message)
		return
	}
	status := http.StatusInternalServerError
	switch {
	case errors.Is(err, ErrApprovalRejected):
		status = http.StatusForbidden
	case errors.Is(err, ErrLocalExecutorUnavailable):
		status = http.StatusNotImplemented
	case errors.Is(err, npki.ErrCertificateNotFound):
		status = http.StatusNotFound
	case errors.Is(err, npki.ErrInvalidPassword):
		status = http.StatusUnauthorized
	}
	writeError(w, status, err.Error())
}

func withCORS(server *Server, next http.Handler) http.Handler {
	return http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		origin := strings.TrimSpace(r.Header.Get("Origin"))
		if origin != "" && server.isOriginAllowed(origin) {
			w.Header().Set("Access-Control-Allow-Origin", origin)
			w.Header().Set("Vary", "Origin")
			w.Header().Set("Access-Control-Allow-Methods", "GET, POST, OPTIONS")
			w.Header().Set("Access-Control-Allow-Headers", "Content-Type, Authorization, X-Agent-Session")
		}
		if r.Method == http.MethodOptions {
			if origin != "" && !server.isOriginAllowed(origin) {
				writeError(w, http.StatusForbidden, "origin not allowed")
				return
			}
			w.WriteHeader(http.StatusNoContent)
			return
		}
		next.ServeHTTP(w, r)
	})
}

func (s *Server) requireSession(w http.ResponseWriter, r *http.Request) (string, bool) {
	origin, err := s.originFromRequest(r)
	if err != nil {
		writeError(w, http.StatusForbidden, err.Error())
		return "", false
	}
	if err := s.validateSession(r); err != nil {
		writeError(w, http.StatusUnauthorized, err.Error())
		return "", false
	}
	return origin, true
}

func (s *Server) requireOrigin(w http.ResponseWriter, r *http.Request) (string, bool) {
	origin, err := s.originFromRequest(r)
	if err != nil {
		writeError(w, http.StatusForbidden, err.Error())
		return "", false
	}
	return origin, true
}

func (s *Server) validateSession(r *http.Request) error {
	origin, err := s.originFromRequest(r)
	if err != nil {
		return err
	}
	sessionToken := strings.TrimSpace(r.Header.Get("X-Agent-Session"))
	if sessionToken == "" {
		return newHTTPStatusError(http.StatusUnauthorized, "missing agent session")
	}
	return s.sessions.validate(origin, sessionToken, time.Now().UTC())
}

func (s *Server) originFromRequest(r *http.Request) (string, error) {
	origin := strings.TrimSpace(r.Header.Get("Origin"))
	if origin == "" || !s.isOriginAllowed(origin) {
		return "", newHTTPStatusError(http.StatusForbidden, "origin not allowed")
	}
	return origin, nil
}

func (s *Server) isOriginAllowed(origin string) bool {
	_, ok := s.allowedOrigins[origin]
	return ok
}

func (s *Server) trustedGatewayBaseURL(requestValue string) (string, error) {
	if s.gatewayBaseURL == "" {
		return "", newHTTPStatusError(http.StatusBadRequest, "gateway backend is not configured")
	}
	configured, err := normalizeTrustedGatewayBaseURL(s.gatewayBaseURL)
	if err != nil {
		return "", err
	}
	requested := strings.TrimSpace(requestValue)
	if requested != "" {
		normalizedRequested, err := normalizeTrustedGatewayBaseURL(requested)
		if err != nil {
			return "", err
		}
		if normalizedRequested != configured {
			return "", newHTTPStatusError(
				http.StatusBadRequest,
				"gatewayBaseUrl does not match configured gateway backend",
			)
		}
	}
	return configured, nil
}

func normalizeTrustedGatewayBaseURL(value string) (string, error) {
	parsed, err := url.Parse(strings.TrimSpace(value))
	if err != nil || parsed.Scheme == "" || parsed.Host == "" {
		return "", newHTTPStatusError(http.StatusBadRequest, "gatewayBaseUrl must be an absolute URL")
	}
	if parsed.User != nil {
		return "", newHTTPStatusError(http.StatusBadRequest, "gatewayBaseUrl must not include userinfo")
	}
	if parsed.RawQuery != "" || parsed.Fragment != "" {
		return "", newHTTPStatusError(http.StatusBadRequest, "gatewayBaseUrl must not include query or fragment")
	}
	switch parsed.Scheme {
	case "https":
	case "http":
		if !isLoopbackHost(parsed.Hostname()) {
			return "", newHTTPStatusError(http.StatusBadRequest, "gatewayBaseUrl must use https except for loopback http")
		}
	default:
		return "", newHTTPStatusError(
			http.StatusBadRequest,
			"gatewayBaseUrl must use https except for loopback http",
		)
	}
	parsed.Path = strings.TrimRight(parsed.Path, "/")
	if parsed.RawPath != "" {
		parsed.RawPath = strings.TrimRight(parsed.RawPath, "/")
	}
	return parsed.String(), nil
}

func isLoopbackHost(host string) bool {
	if strings.EqualFold(host, "localhost") {
		return true
	}
	ip := net.ParseIP(host)
	return ip != nil && ip.IsLoopback()
}

func allowlistFromConfig(origins []string) map[string]struct{} {
	allowlist := make(map[string]struct{}, len(origins))
	for _, origin := range origins {
		trimmed := strings.TrimSpace(origin)
		if trimmed == "" {
			continue
		}
		allowlist[trimmed] = struct{}{}
	}
	return allowlist
}

func findCertificateByID(certificates []npki.CertificateMetadata, certificateID string) (npki.CertificateMetadata, error) {
	for _, certificate := range certificates {
		if certificate.ID == certificateID {
			return certificate, nil
		}
	}
	return npki.CertificateMetadata{}, npki.ErrCertificateNotFound
}

func newSessionManager(secret []byte, ttl time.Duration) sessionManager {
	secretCopy := append([]byte(nil), secret...)
	if ttl <= 0 {
		ttl = 5 * time.Minute
	}
	return sessionManager{
		secret: secretCopy,
		ttl:    ttl,
	}
}

func (m sessionManager) issue(origin string, now time.Time) (string, time.Time, error) {
	nonce, err := randomNonce()
	if err != nil {
		return "", time.Time{}, fmt.Errorf("generate session nonce: %w", err)
	}
	expiresAt := now.Add(m.ttl)
	payload := sessionPayload{
		Origin: origin,
		Expiry: expiresAt.Format(time.RFC3339),
		Nonce:  nonce,
	}
	encodedPayload, err := encodePayload(payload)
	if err != nil {
		return "", time.Time{}, err
	}
	signature := signPayload(m.secret, encodedPayload)
	token := encodedPayload + "." + base64.RawURLEncoding.EncodeToString(signature)
	return token, expiresAt, nil
}

func (m sessionManager) validate(origin string, token string, now time.Time) error {
	parts := strings.Split(token, ".")
	if len(parts) != 2 {
		return newHTTPStatusError(http.StatusUnauthorized, "invalid agent session")
	}
	expected := signPayload(m.secret, parts[0])
	actual, err := base64.RawURLEncoding.DecodeString(parts[1])
	if err != nil || !hmac.Equal(expected, actual) {
		return newHTTPStatusError(http.StatusUnauthorized, "invalid agent session")
	}
	payload, err := decodePayload(parts[0])
	if err != nil {
		return newHTTPStatusError(http.StatusUnauthorized, "invalid agent session")
	}
	if payload.Origin != origin {
		return newHTTPStatusError(http.StatusUnauthorized, "agent session origin mismatch")
	}
	expiry, err := time.Parse(time.RFC3339, payload.Expiry)
	if err != nil {
		return newHTTPStatusError(http.StatusUnauthorized, "invalid agent session")
	}
	if now.After(expiry) {
		return newHTTPStatusError(http.StatusUnauthorized, "agent session expired")
	}
	return nil
}

func signPayload(secret []byte, payload string) []byte {
	sum := hmac.New(sha256.New, secret)
	_, _ = sum.Write([]byte(payload))
	return sum.Sum(nil)
}

func encodePayload(payload sessionPayload) (string, error) {
	raw, err := json.Marshal(payload)
	if err != nil {
		return "", fmt.Errorf("encode session payload: %w", err)
	}
	return base64.RawURLEncoding.EncodeToString(raw), nil
}

func decodePayload(encoded string) (sessionPayload, error) {
	raw, err := base64.RawURLEncoding.DecodeString(encoded)
	if err != nil {
		return sessionPayload{}, err
	}
	var payload sessionPayload
	if err := json.Unmarshal(raw, &payload); err != nil {
		return sessionPayload{}, err
	}
	return payload, nil
}

func randomNonce() (string, error) {
	var buf [16]byte
	if _, err := rand.Read(buf[:]); err != nil {
		return "", err
	}
	return base64.RawURLEncoding.EncodeToString(buf[:]), nil
}

type unavailableLocalExecutor struct{}

func (unavailableLocalExecutor) RunEphemeral(context.Context, LocalEphemeralRequest) (map[string]any, error) {
	return nil, ErrLocalExecutorUnavailable
}

type commandLocalExecutor struct {
	path string
}

func NewCommandLocalExecutor(path string) localExecutor {
	path = strings.TrimSpace(path)
	if path == "" {
		return unavailableLocalExecutor{}
	}
	return commandLocalExecutor{path: path}
}

func (c commandLocalExecutor) RunEphemeral(ctx context.Context, req LocalEphemeralRequest) (map[string]any, error) {
	payload, err := json.Marshal(req)
	if err != nil {
		return nil, err
	}
	ctx, cancel := context.WithTimeout(ctx, localExecutorTimeout)
	defer cancel()
	cmd := exec.CommandContext(ctx, c.path)
	cmd.Stdin = bytes.NewReader(payload)
	var stdout bytes.Buffer
	var stderr bytes.Buffer
	stdoutLimit := &limitedBufferWriter{w: &stdout, limit: maxLocalExecutorBytes}
	stderrLimit := &limitedBufferWriter{w: &stderr, limit: maxLocalExecutorStderr}
	cmd.Stdout = stdoutLimit
	cmd.Stderr = stderrLimit
	if err := cmd.Run(); err != nil {
		detail := strings.TrimSpace(stderr.String())
		if detail != "" {
			return nil, fmt.Errorf("local executor failed: %w: %s", err, detail)
		}
		return nil, fmt.Errorf("local executor failed: %w", err)
	}
	if stdoutLimit.truncated {
		return nil, fmt.Errorf("local executor response exceeds %d bytes", maxLocalExecutorBytes)
	}
	var result map[string]any
	if err := json.Unmarshal(stdout.Bytes(), &result); err != nil {
		return nil, fmt.Errorf("decode local executor response: %w", err)
	}
	return result, nil
}

type limitedBufferWriter struct {
	w         *bytes.Buffer
	limit     int
	truncated bool
}

func (l *limitedBufferWriter) Write(p []byte) (int, error) {
	remaining := l.limit - l.w.Len()
	if remaining <= 0 {
		l.truncated = true
		return len(p), nil
	}
	if len(p) > remaining {
		_, _ = l.w.Write(p[:remaining])
		l.truncated = true
		return len(p), nil
	}
	_, _ = l.w.Write(p)
	return len(p), nil
}

type httpGatewayExecutor struct {
	client *http.Client
}

func NewHTTPGatewayExecutor() gatewayExecutor {
	return httpGatewayExecutor{
		client: &http.Client{Timeout: gatewayRequestTimeout},
	}
}

func (h httpGatewayExecutor) CreateChallenge(ctx context.Context, req GatewayChallengeRequest) (challengeResponse, error) {
	return h.createChallenge(ctx, req)
}

func (h httpGatewayExecutor) RegisterStored(ctx context.Context, req GatewayStoredRegistrationRequest) (map[string]any, error) {
	return h.postJSON(ctx, gatewayRequest{
		BaseURL:              req.BaseURL,
		Path:                 fmt.Sprintf("/api/v1/organizations/%s/certificates", url.PathEscape(req.OrganizationID)),
		BearerToken:          req.BearerToken,
		ActiveOrganizationID: req.ActiveOrganizationID,
		Body: map[string]any{
			"challengeId":              req.ChallengeID,
			"challengePayload":         req.ChallengePayload,
			"challengeSignatureBase64": req.ChallengeSignatureBase64,
			"bundle": map[string]any{
				"businessNo":        firstNonEmpty(req.Bundle.BusinessNo, req.BusinessNo),
				"password":          req.Password,
				"commonName":        req.Bundle.CommonName,
				"signCertDerBase64": req.Bundle.SignCertDerBase64,
				"signPriKeyBase64":  req.Bundle.SignPriKeyBase64,
				"kmCertDerBase64":   req.Bundle.KMCertDerBase64,
				"kmPriKeyBase64":    req.Bundle.KMPriKeyBase64,
			},
		},
	})
}

type gatewayRequest struct {
	BaseURL              string
	Path                 string
	BearerToken          string
	ActiveOrganizationID string
	Body                 any
}

type challengeResponse struct {
	ChallengeID string `json:"challengeId"`
	Payload     string `json:"payload"`
	SyncMode    string `json:"syncMode,omitempty"`
	StartMonth  string `json:"startMonth,omitempty"`
	EndMonth    string `json:"endMonth,omitempty"`
}

func (h httpGatewayExecutor) createChallenge(ctx context.Context, req GatewayChallengeRequest) (challengeResponse, error) {
	var result challengeResponse
	body := map[string]any{
		"mode": req.Mode,
	}
	if req.Origin != "" {
		body["origin"] = req.Origin
	}
	if req.BusinessNo != "" {
		body["businessNo"] = req.BusinessNo
	}
	if req.SyncMode != "" {
		body["syncMode"] = req.SyncMode
	}
	if req.FacilityCode != "" {
		body["facilityCode"] = req.FacilityCode
	}
	if req.StartMonth != "" {
		body["startMonth"] = req.StartMonth
	}
	if req.EndMonth != "" {
		body["endMonth"] = req.EndMonth
	}
	if req.SignCertSHA256Hex != "" {
		body["signCertSha256Hex"] = req.SignCertSHA256Hex
	}
	responseBody, err := h.postJSON(ctx, gatewayRequest{
		BaseURL:              req.BaseURL,
		Path:                 fmt.Sprintf("/api/v1/organizations/%s/cert-gateway/challenges", url.PathEscape(req.OrganizationID)),
		BearerToken:          req.BearerToken,
		ActiveOrganizationID: req.ActiveOrganizationID,
		Body:                 body,
	})
	if err != nil {
		return result, err
	}
	raw, err := json.Marshal(responseBody)
	if err != nil {
		return result, err
	}
	if err := json.Unmarshal(raw, &result); err != nil {
		return result, err
	}
	return result, nil
}

func (h httpGatewayExecutor) postJSON(ctx context.Context, req gatewayRequest) (map[string]any, error) {
	payload, err := json.Marshal(req.Body)
	if err != nil {
		return nil, err
	}
	httpReq, err := http.NewRequestWithContext(ctx, http.MethodPost, strings.TrimRight(req.BaseURL, "/")+req.Path, bytes.NewReader(payload))
	if err != nil {
		return nil, err
	}
	httpReq.Header.Set("Content-Type", "application/json")
	if req.BearerToken != "" {
		httpReq.Header.Set("Authorization", "Bearer "+req.BearerToken)
	}
	if req.ActiveOrganizationID != "" {
		httpReq.Header.Set("X-Organization-Id", req.ActiveOrganizationID)
	}
	resp, err := h.client.Do(httpReq)
	if err != nil {
		return nil, err
	}
	defer resp.Body.Close()
	raw, err := readLimitedBody(resp.Body, maxJSONBodyBytes)
	if err != nil {
		return nil, err
	}
	if resp.StatusCode < 200 || resp.StatusCode >= 300 {
		if detail := summarizeGatewayErrorBody(raw); detail != "" {
			return nil, fmt.Errorf("gateway request failed with status %d: %s", resp.StatusCode, detail)
		}
		return nil, fmt.Errorf("gateway request failed with status %d", resp.StatusCode)
	}
	var body map[string]any
	if err := json.Unmarshal(raw, &body); err != nil {
		return nil, err
	}
	return body, nil
}

func readLimitedBody(reader io.Reader, maxBytes int64) ([]byte, error) {
	raw, err := io.ReadAll(io.LimitReader(reader, maxBytes+1))
	if err != nil {
		return nil, err
	}
	if int64(len(raw)) > maxBytes {
		return nil, fmt.Errorf("gateway response body exceeds %d bytes", maxBytes)
	}
	return raw, nil
}

func firstNonEmpty(values ...string) string {
	for _, value := range values {
		if trimmed := strings.TrimSpace(value); trimmed != "" {
			return trimmed
		}
	}
	return ""
}

func validateSignedBusinessNo(requested string, signed string) error {
	requested = digitsOnly(requested)
	signed = digitsOnly(signed)
	if requested == "" || signed == "" || requested == signed {
		return nil
	}
	return newHTTPStatusError(http.StatusBadRequest, "selected certificate businessNo does not match request")
}

func digitsOnly(value string) string {
	var b strings.Builder
	for _, r := range value {
		if r >= '0' && r <= '9' {
			b.WriteRune(r)
		}
	}
	return b.String()
}

func summarizeGatewayErrorBody(raw []byte) string {
	trimmed := strings.TrimSpace(string(raw))
	if trimmed == "" {
		return ""
	}
	var body map[string]any
	if err := json.Unmarshal(raw, &body); err == nil {
		for _, key := range []string{"message", "error", "detail"} {
			if value, ok := body[key]; ok {
				if text := summarizeGatewayErrorValue(value); text != "" {
					return text
				}
			}
		}
	}
	if len(trimmed) > 200 {
		return trimmed[:200]
	}
	return trimmed
}

func summarizeGatewayErrorValue(value any) string {
	if text, ok := value.(string); ok {
		return strings.TrimSpace(text)
	}
	encoded, err := json.Marshal(value)
	if err != nil {
		return ""
	}
	return strings.TrimSpace(string(encoded))
}
