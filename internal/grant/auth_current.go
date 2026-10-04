package grant

import (
	"context"
	"fmt"
	"strings"
	"time"
)

func (s *Service) effectiveConsistency(c ReadConsistency) ReadConsistency {
	if c != "" {
		return c
	}
	return s.cfg.Database.DefaultReadConsistency
}
func (s *Service) localIdentity(ctx context.Context, identity ExternalIdentity, c ReadConsistency) (User, error) {
	c = s.effectiveConsistency(c)
	if readScope(ctx) != nil {
		c = ReadLocal
	}
	u, err := s.store.resolveScopedIdentity(ctx, ExternalIdentityMaterialization{Provider: identity.Provider, Issuer: identity.Issuer, CollectionID: identity.CollectionID, ExternalUserID: identity.ExternalUserID}, c)
	if err != nil {
		return User{}, err
	}
	if u == nil {
		return User{}, serviceError(403, fmt.Sprintf("forbidden: no local user identity available for %s:%s", identity.Provider, identity.ExternalUserID))
	}
	return *u, nil
}
func (s *Service) Authenticate(ctx context.Context, token, organization string, c ReadConsistency) (AuthenticatedPrincipal, error) {
	organization = strings.TrimSpace(organization)
	if organization == "" {
		return AuthenticatedPrincipal{}, serviceError(401, "unauthorized: missing x-organization-id header")
	}
	identity, err := s.resolveExternalIdentity(ctx, token, c)
	if err != nil {
		return AuthenticatedPrincipal{}, err
	}
	user, err := s.localIdentity(ctx, identity, c)
	if err != nil {
		return AuthenticatedPrincipal{}, err
	}
	if !user.Active {
		return AuthenticatedPrincipal{}, serviceError(403, fmt.Sprintf("forbidden: user %s is inactive", user.ID))
	}
	ok, err := s.store.HasActiveOrganizationMembershipWithConsistency(ctx, user.ID, organization, s.effectiveConsistency(c))
	if err != nil {
		return AuthenticatedPrincipal{}, err
	}
	if !ok {
		return AuthenticatedPrincipal{}, serviceError(403, fmt.Sprintf("forbidden: user %s is not an active member of organization %s", user.ID, organization))
	}
	return AuthenticatedPrincipal{UserID: user.ID, OrganizationID: organization}, nil
}
func (s *Service) CurrentUser(ctx context.Context, token, organization string, c ReadConsistency) (CurrentUserView, error) {
	c = s.effectiveConsistency(c)
	if readScope(ctx) != nil {
		c = ReadLocal
	}
	identity, err := s.resolveExternalIdentity(ctx, token, c)
	if err != nil {
		return CurrentUserView{}, err
	}
	user, err := s.store.resolveScopedIdentity(ctx, ExternalIdentityMaterialization{Provider: identity.Provider, Issuer: identity.Issuer, CollectionID: identity.CollectionID, ExternalUserID: identity.ExternalUserID}, c)
	if err != nil {
		return CurrentUserView{}, err
	}
	if user == nil {
		return CurrentUserView{}, serviceErrorCode(404, "local user is not provisioned", "LOCAL_USER_NOT_PROVISIONED")
	}
	if !user.Active {
		return CurrentUserView{}, serviceError(403, "forbidden: local user is inactive")
	}
	orgs, err := s.store.ListUserOrganizationsWithConsistency(ctx, user.ID, c)
	if err != nil {
		return CurrentUserView{}, err
	}
	var active *string
	organization = strings.TrimSpace(organization)
	if organization != "" {
		for _, org := range orgs {
			if org.ID == organization {
				active = &organization
				break
			}
		}
		if active == nil {
			return CurrentUserView{}, serviceError(403, fmt.Sprintf("forbidden: rauthy user %s is not an active member of organization %s", identity.ExternalUserID, organization))
		}
	}
	return CurrentUserView{Provider: identity.Provider, Issuer: identity.Issuer, CollectionID: identity.CollectionID, ExternalUserID: identity.ExternalUserID, User: *user, Organizations: orgs, ActiveOrganizationID: active}, nil
}
func (s *Service) EnsureCurrentUser(ctx context.Context, token string) (EnsuredUserView, error) {
	identity, err := s.resolveExternalIdentity(ctx, token, ReadStrong)
	if err != nil {
		return EnsuredUserView{}, err
	}
	user, err := s.store.EnsureExternalIdentity(ctx, ExternalIdentityMaterialization{Provider: identity.Provider, Issuer: identity.Issuer, CollectionID: identity.CollectionID, ExternalUserID: identity.ExternalUserID, Email: identity.Email, DisplayName: identity.DisplayName})
	if err != nil {
		return EnsuredUserView{}, serviceError(403, "forbidden: identity ensure failed")
	}
	if !user.Active {
		return EnsuredUserView{}, serviceError(403, "forbidden: local user is inactive")
	}
	return EnsuredUserView{UserID: user.ID}, nil
}
func (s *Service) SetUserActiveAdmin(ctx context.Context, id string, active bool) (User, error) {
	return s.store.SetUserActive(ctx, id, active)
}
func (s *Service) Readiness(ctx context.Context) (HealthStatus, bool) {
	if s.cfg.Database.LocalNodeRole == "learner" {
		ctx, cancel := context.WithTimeout(ctx, 2*time.Second)
		defer cancel()
		_, e := localReplicaRead(ctx, s, func(ctx context.Context) (bool, error) { return true, s.store.verifySchemaAt(ctx, ReadLocal) })
		status := "ok"
		if e != nil {
			status = "down"
		}
		return HealthStatus{Status: status, ReadReady: e == nil, WriteReady: false, RuntimeMode: s.cfg.Database.RuntimeMode, DefaultReadConsistency: ReadLocal}, e == nil
	}
	h := s.store.Health(ctx)
	switch h.DefaultReadConsistency {
	case ReadStrong:
		return h, h.WriteReady
	case ReadLocal:
		return h, h.ReadReady
	case ReadPreferStrong:
		return h, h.ReadReady || h.WriteReady
	default:
		return h, false
	}
}
func (s *Service) Bootstrap(ctx context.Context) error { return s.store.EnsureBootstrapData(ctx) }
