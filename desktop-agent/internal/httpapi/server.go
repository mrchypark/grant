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
	"net/http"
	"strings"
	"time"

	"github.com/cypark/account/desktop-agent/internal/npki"
)

type npkiService interface {
	ListCertificates(context.Context) ([]npki.CertificateMetadata, error)
	Sign(context.Context, npki.SignRequest) (npki.SignResult, error)
}

type approvalPrompter interface {
	PromptApproval(context.Context, ApprovalPrompt) (ApprovalDecision, error)
}

type gatewayExecutor interface {
	RunEphemeral(context.Context, GatewayEphemeralRequest) (map[string]any, error)
	RegisterStored(context.Context, GatewayStoredRegistrationRequest) (map[string]any, error)
}

type Config struct {
	AllowedOrigins []string
	SessionTTL     time.Duration
	SessionSecret  []byte
}

var ErrApprovalRejected = errors.New("approval rejected")
var errMethodNotAllowed = errors.New("method not allowed")

type ApprovalPrompt struct {
	Origin         string
	Action         string
	OrganizationID string
	BusinessNo     string
	Certificates   []npki.CertificateMetadata
}

type ApprovalDecision struct {
	CertificateID string
	Password      string
}

type ApprovalActionRequest struct {
	GatewayBaseURL       string `json:"gatewayBaseUrl"`
	OrganizationID       string `json:"organizationId"`
	BusinessNo           string `json:"businessNo"`
	BearerToken          string `json:"bearerToken,omitempty"`
	ActiveOrganizationID string `json:"activeOrganizationId,omitempty"`
}

type GatewayEphemeralRequest struct {
	BaseURL              string
	OrganizationID       string
	BusinessNo           string
	Password             string
	BearerToken          string
	ActiveOrganizationID string
	Bundle               npki.ProofBundle
}

type GatewayStoredRegistrationRequest struct {
	BaseURL              string
	OrganizationID       string
	BusinessNo           string
	Password             string
	BearerToken          string
	ActiveOrganizationID string
	Bundle               npki.ProofBundle
}

type Server struct {
	npki           npkiService
	approver       approvalPrompter
	gateway        gatewayExecutor
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

func NewServer(service npkiService, approver approvalPrompter, gateway gatewayExecutor, cfg Config) *Server {
	server := &Server{
		npki:           service,
		approver:       approver,
		gateway:        gateway,
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

	var req npki.SignRequest
	if err := json.NewDecoder(r.Body).Decode(&req); err != nil {
		writeError(w, http.StatusBadRequest, "invalid json body")
		return
	}
	req.CertificateID = strings.TrimSpace(req.CertificateID)
	req.Challenge = strings.TrimSpace(req.Challenge)

	switch {
	case req.Challenge == "":
		writeError(w, http.StatusBadRequest, "challenge must not be empty")
		return
	case strings.TrimSpace(req.Password) == "":
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
	writeJSON(w, http.StatusOK, result)
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
	result, status, err := s.handleApprovedAction(r, "ephemeral")
	if err != nil {
		writeApprovalError(w, err)
		return
	}
	writeJSON(w, status, result)
}

func (s *Server) handleApproveStoredRegistration(w http.ResponseWriter, r *http.Request) {
	result, status, err := s.handleApprovedAction(r, "stored")
	if err != nil {
		writeApprovalError(w, err)
		return
	}
	writeJSON(w, status, result)
}

func (s *Server) handleApprovedAction(r *http.Request, action string) (map[string]any, int, error) {
	if r.Method != http.MethodPost {
		return nil, 0, errMethodNotAllowed
	}
	if err := s.validateSession(r); err != nil {
		return nil, 0, err
	}
	defer r.Body.Close()

	var req ApprovalActionRequest
	if err := json.NewDecoder(r.Body).Decode(&req); err != nil {
		return nil, 0, errors.New("invalid json body")
	}
	req.GatewayBaseURL = strings.TrimSpace(req.GatewayBaseURL)
	req.OrganizationID = strings.TrimSpace(req.OrganizationID)
	req.BusinessNo = strings.TrimSpace(req.BusinessNo)
	req.BearerToken = strings.TrimSpace(req.BearerToken)
	req.ActiveOrganizationID = strings.TrimSpace(req.ActiveOrganizationID)

	if req.GatewayBaseURL == "" {
		return nil, 0, errors.New("gatewayBaseUrl must not be empty")
	}
	if req.OrganizationID == "" {
		return nil, 0, errors.New("organizationId must not be empty")
	}
	if req.BusinessNo == "" {
		return nil, 0, errors.New("businessNo must not be empty")
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
	signResult, err := s.npki.Sign(r.Context(), npki.SignRequest{
		CertificateID: decision.CertificateID,
		Challenge:     "pending",
		Password:      decision.Password,
		BusinessNo:    req.BusinessNo,
	})
	if err != nil {
		return nil, 0, err
	}

	switch action {
	case "ephemeral":
		body, err := s.gateway.RunEphemeral(r.Context(), GatewayEphemeralRequest{
			BaseURL:              req.GatewayBaseURL,
			OrganizationID:       req.OrganizationID,
			BusinessNo:           req.BusinessNo,
			Password:             decision.Password,
			BearerToken:          req.BearerToken,
			ActiveOrganizationID: req.ActiveOrganizationID,
			Bundle:               signResult.Bundle,
		})
		return body, http.StatusAccepted, err
	case "stored":
		body, err := s.gateway.RegisterStored(r.Context(), GatewayStoredRegistrationRequest{
			BaseURL:              req.GatewayBaseURL,
			OrganizationID:       req.OrganizationID,
			BusinessNo:           req.BusinessNo,
			Password:             decision.Password,
			BearerToken:          req.BearerToken,
			ActiveOrganizationID: req.ActiveOrganizationID,
			Bundle:               signResult.Bundle,
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

func writeApprovalError(w http.ResponseWriter, err error) {
	status := http.StatusInternalServerError
	switch {
	case errors.Is(err, errMethodNotAllowed):
		status = http.StatusMethodNotAllowed
	case errors.Is(err, ErrApprovalRejected):
		status = http.StatusForbidden
	case errors.Is(err, npki.ErrCertificateNotFound):
		status = http.StatusNotFound
	case errors.Is(err, npki.ErrInvalidPassword):
		status = http.StatusUnauthorized
	case strings.Contains(err.Error(), "origin not allowed"):
		status = http.StatusForbidden
	default:
		message := err.Error()
		if strings.Contains(message, "must not be empty") || strings.Contains(message, "invalid json body") {
			status = http.StatusBadRequest
		} else if strings.Contains(message, "agent session") {
			status = http.StatusUnauthorized
		}
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
		return errors.New("missing agent session")
	}
	return s.sessions.validate(origin, sessionToken, time.Now().UTC())
}

func (s *Server) originFromRequest(r *http.Request) (string, error) {
	origin := strings.TrimSpace(r.Header.Get("Origin"))
	if origin == "" || !s.isOriginAllowed(origin) {
		return "", errors.New("origin not allowed")
	}
	return origin, nil
}

func (s *Server) isOriginAllowed(origin string) bool {
	_, ok := s.allowedOrigins[origin]
	return ok
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
		return errors.New("invalid agent session")
	}
	expected := signPayload(m.secret, parts[0])
	actual, err := base64.RawURLEncoding.DecodeString(parts[1])
	if err != nil || !hmac.Equal(expected, actual) {
		return errors.New("invalid agent session")
	}
	payload, err := decodePayload(parts[0])
	if err != nil {
		return errors.New("invalid agent session")
	}
	if payload.Origin != origin {
		return errors.New("agent session origin mismatch")
	}
	expiry, err := time.Parse(time.RFC3339, payload.Expiry)
	if err != nil {
		return errors.New("invalid agent session")
	}
	if now.After(expiry) {
		return errors.New("agent session expired")
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

type httpGatewayExecutor struct {
	client *http.Client
}

func NewHTTPGatewayExecutor() gatewayExecutor {
	return httpGatewayExecutor{
		client: &http.Client{Timeout: 60 * time.Second},
	}
}

func (h httpGatewayExecutor) RunEphemeral(ctx context.Context, req GatewayEphemeralRequest) (map[string]any, error) {
	challenge, err := h.createChallenge(ctx, req.BaseURL, req.OrganizationID, req.BearerToken, req.ActiveOrganizationID, "ephemeral")
	if err != nil {
		return nil, err
	}
	return h.postJSON(ctx, gatewayRequest{
		BaseURL:              req.BaseURL,
		Path:                 fmt.Sprintf("/api/v1/organizations/%s/cert-gateway/ephemeral-executions", req.OrganizationID),
		BearerToken:          req.BearerToken,
		ActiveOrganizationID: req.ActiveOrganizationID,
		Body: map[string]any{
			"challengeId":      challenge.ChallengeID,
			"challengePayload": challenge.Payload,
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

func (h httpGatewayExecutor) RegisterStored(ctx context.Context, req GatewayStoredRegistrationRequest) (map[string]any, error) {
	challenge, err := h.createChallenge(ctx, req.BaseURL, req.OrganizationID, req.BearerToken, req.ActiveOrganizationID, "stored")
	if err != nil {
		return nil, err
	}
	return h.postJSON(ctx, gatewayRequest{
		BaseURL:              req.BaseURL,
		Path:                 fmt.Sprintf("/api/v1/organizations/%s/certificates", req.OrganizationID),
		BearerToken:          req.BearerToken,
		ActiveOrganizationID: req.ActiveOrganizationID,
		Body: map[string]any{
			"challengeId":      challenge.ChallengeID,
			"challengePayload": challenge.Payload,
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
}

func (h httpGatewayExecutor) createChallenge(ctx context.Context, baseURL string, organizationID string, bearerToken string, activeOrganizationID string, mode string) (challengeResponse, error) {
	var result challengeResponse
	body, err := h.postJSON(ctx, gatewayRequest{
		BaseURL:              baseURL,
		Path:                 fmt.Sprintf("/api/v1/organizations/%s/cert-gateway/challenges", organizationID),
		BearerToken:          bearerToken,
		ActiveOrganizationID: activeOrganizationID,
		Body: map[string]any{
			"mode": mode,
		},
	})
	if err != nil {
		return result, err
	}
	raw, err := json.Marshal(body)
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
	raw, err := io.ReadAll(resp.Body)
	if err != nil {
		return nil, err
	}
	if resp.StatusCode < 200 || resp.StatusCode >= 300 {
		return nil, fmt.Errorf("gateway request failed with status %d", resp.StatusCode)
	}
	var body map[string]any
	if err := json.Unmarshal(raw, &body); err != nil {
		return nil, err
	}
	return body, nil
}

func firstNonEmpty(values ...string) string {
	for _, value := range values {
		if trimmed := strings.TrimSpace(value); trimmed != "" {
			return trimmed
		}
	}
	return ""
}
