package grant

import (
	"context"
	"crypto/sha256"
	"encoding/binary"
	"encoding/hex"
	"errors"
	"fmt"
	"net/http"
	"strings"
	"sync"
	"time"
)

type Error struct {
	Status  int
	Message string
}

func (e *Error) Error() string { return e.Message }
func ErrorStatus(err error) int {
	var typed *Error
	if errors.As(err, &typed) {
		return typed.Status
	}
	text := err.Error()
	switch {
	case strings.Contains(text, "forbidden:"):
		return 403
	case strings.Contains(text, "unauthorized:"):
		return 401
	case strings.Contains(text, "UNIQUE constraint failed"), strings.Contains(text, "already exists"):
		return 409
	case strings.Contains(text, "unavailable:"):
		return 503
	case strings.Contains(text, "not found"):
		return 404
	case strings.Contains(text, "must not be empty"), strings.Contains(text, "mismatch"), strings.Contains(text, "expired"), strings.Contains(text, "must be included"):
		return 400
	default:
		return 500
	}
}
func serviceError(status int, message string) error { return &Error{status, message} }
func required(field, value string) (string, error) {
	value = strings.TrimSpace(value)
	if value == "" {
		return "", serviceError(400, field+" must not be empty")
	}
	return value, nil
}

type Service struct {
	cfg       Config
	store     *Store
	client    *http.Client
	authMu    sync.Mutex
	authCache map[string]cachedIdentity
}

func NewService(cfg Config, store *Store) *Service {
	return &Service{cfg: cfg, store: store, authCache: make(map[string]cachedIdentity), client: &http.Client{Timeout: 30 * time.Second, CheckRedirect: func(_ *http.Request, _ []*http.Request) error { return http.ErrUseLastResponse }}}
}
func (s *Service) Config() Config                  { return s.cfg }
func (s *Service) RauthyBaseURL() string           { return s.cfg.Auth.Rauthy.BaseURL }
func (s *Service) AuthProviderMode() string        { return s.cfg.Auth.ProviderMode }
func (s *Service) RauthyProxyClient() *http.Client { return s.client }

func (s *Service) CheckAuthorization(ctx context.Context, subject AuthorizationSubject, action string, resource AuthorizationResource, consistency ReadConsistency) (AuthorizationDecision, error) {
	decisions, err := s.CheckAuthorizations(ctx, []AuthorizationCheck{{Subject: subject, Action: action, Resource: resource}}, consistency)
	if err != nil {
		return AuthorizationDecision{}, err
	}
	if len(decisions) != 1 {
		return AuthorizationDecision{}, serviceError(503, "authorization check failed")
	}
	return decisions[0], nil
}
func (s *Service) CheckAuthorizations(ctx context.Context, checks []AuthorizationCheck, consistency ReadConsistency) ([]AuthorizationDecision, error) {
	return s.evaluateAuthorizations(ctx, checks, consistency, nil)
}
func (s *Service) evaluateAuthorizations(ctx context.Context, checks []AuthorizationCheck, consistency ReadConsistency, applicationID *string) ([]AuthorizationDecision, error) {
	dbChecks := make([]UserActionCheck, 0, len(checks))
	for _, check := range checks {
		if check.Subject.SubjectType != "user" {
			return nil, serviceError(400, "only user authz subjects are supported")
		}
		values := []struct{ name, value string }{{"subject.id", check.Subject.ID}, {"subject.orgId", check.Subject.OrganizationID}, {"resource.provider", check.Resource.ProviderID}, {"resource.type", check.Resource.ResourceType}, {"resource.id", check.Resource.ID}, {"action", check.Action}}
		normalized := make([]string, len(values))
		for i, v := range values {
			n, err := required(v.name, v.value)
			if err != nil {
				return nil, err
			}
			normalized[i] = n
		}
		dbChecks = append(dbChecks, UserActionCheck{UserID: normalized[0], OrganizationID: normalized[1], ProviderID: normalized[2], ResourceType: normalized[3], ResourceID: normalized[4], ActionID: normalized[5], ApplicationID: applicationID, Consistency: consistency})
	}
	rows, err := s.store.EvaluateUserActions(ctx, dbChecks)
	if err != nil {
		return nil, err
	}
	if len(rows) != len(checks) {
		return nil, serviceError(503, "authorization check failed")
	}
	out := make([]AuthorizationDecision, len(rows))
	for i, d := range rows {
		reason := "no_matching_binding"
		if d.Allowed {
			if applicationID == nil {
				reason = "role_binding"
			} else if d.TrustedApplication {
				reason = "trusted_application"
			} else {
				reason = "explicit_consent"
			}
		} else if d.RBACAllowed && applicationID != nil {
			reason = "application_consent_required"
		}
		out[i] = AuthorizationDecision{Allowed: d.Allowed, Reason: reason}
	}
	return out, nil
}
func (s *Service) requireOrganizationAction(ctx context.Context, userID, orgID, targetOrgID, action, label string, consistency ReadConsistency) error {
	d, err := s.CheckAuthorization(ctx, AuthorizationSubject{SubjectType: "user", ID: userID, OrganizationID: orgID}, action, AuthorizationResource{ProviderID: "grant", ResourceType: "organization", ID: targetOrgID}, consistency)
	if err != nil {
		return err
	}
	if !d.Allowed {
		return serviceError(403, fmt.Sprintf("forbidden: user %s in organization %s cannot %s", userID, orgID, label))
	}
	return nil
}
func ExposurePrincipalDigest(userID, organizationID, applicationID string) string {
	h := sha256.New()
	h.Write([]byte("grant.exposure-read.principal.v1"))
	for _, v := range []string{userID, organizationID, applicationID} {
		var size [8]byte
		binary.BigEndian.PutUint64(size[:], uint64(len(v)))
		h.Write(size[:])
		h.Write([]byte(v))
	}
	return hex.EncodeToString(h.Sum(nil))
}
func CredentialDigest(credential string) string {
	h := sha256.New()
	h.Write([]byte("grant.http-capability.provider-credential.v1"))
	var size [8]byte
	binary.BigEndian.PutUint64(size[:], uint64(len(credential)))
	h.Write(size[:])
	h.Write([]byte(credential))
	return "sha256:" + hex.EncodeToString(h.Sum(nil))
}
