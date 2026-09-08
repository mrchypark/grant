package grant

import (
	"context"
	"fmt"
	"strings"
)

func (s *Service) effectiveConsistency(c ReadConsistency) ReadConsistency {
	if c != "" {
		return c
	}
	return s.cfg.Database.DefaultReadConsistency
}
func (s *Service) localIdentity(ctx context.Context, identity ExternalIdentity, c ReadConsistency) (User, error) {
	c = s.effectiveConsistency(c)
	if c == ReadStrong || c == ReadPreferStrong {
		var u User
		var err error
		if identity.Provider == "patch-data" {
			u, err = s.store.MaterializeExternalIdentity(ctx, ExternalIdentityMaterialization{Provider: identity.Provider, ExternalUserID: identity.ExternalUserID, Email: identity.Email, DisplayName: identity.DisplayName})
		} else {
			u, err = s.store.ResolveOrCreateUserIdentityWithConsistency(ctx, identity.Provider, identity.ExternalUserID, identity.Email, identity.DisplayName, c)
		}
		if err != nil && ErrorStatus(err) != 401 && ErrorStatus(err) != 403 {
			return User{}, serviceError(503, "unavailable: strong consistency requested but quorum-backed identity sync is unavailable")
		}
		return u, err
	}
	u, err := s.store.ResolveExistingUserIdentityWithConsistency(ctx, identity.Provider, identity.ExternalUserID, c)
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
func (s *Service) AuthenticateRequest(ctx context.Context, token, organization string, c ReadConsistency) (AuthenticatedPrincipal, error) {
	return s.Authenticate(ctx, token, organization, c)
}
func (s *Service) CurrentUser(ctx context.Context, token, organization string, c ReadConsistency) (CurrentUserView, error) {
	identity, err := s.resolveExternalIdentity(ctx, token, c)
	if err != nil {
		return CurrentUserView{}, err
	}
	user, err := s.localIdentity(ctx, identity, c)
	if err != nil {
		return CurrentUserView{}, err
	}
	if !user.Active {
		return CurrentUserView{}, serviceError(403, "forbidden: local user is inactive")
	}
	orgs, err := s.store.ListUserOrganizationsWithConsistency(ctx, user.ID, s.effectiveConsistency(c))
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
	return CurrentUserView{Provider: identity.Provider, ExternalUserID: identity.ExternalUserID, User: user, Organizations: orgs, ActiveOrganizationID: active}, nil
}
func (s *Service) MaterializeCurrentUser(ctx context.Context, token, org string) (MaterializedUserView, error) {
	if org == "" {
		return MaterializedUserView{}, serviceError(403, "forbidden: organization is required")
	}
	identity, err := s.resolveExternalIdentity(ctx, token, ReadStrong)
	if err != nil {
		return MaterializedUserView{}, err
	}
	user, err := s.store.MaterializeExternalIdentity(ctx, ExternalIdentityMaterialization{Provider: identity.Provider, ExternalUserID: identity.ExternalUserID, Email: identity.Email, DisplayName: identity.DisplayName})
	if err != nil {
		return MaterializedUserView{}, serviceError(403, "forbidden: identity materialization failed")
	}
	if !user.Active {
		return MaterializedUserView{}, serviceError(403, "forbidden: local user is inactive")
	}
	rows, err := s.store.Query(ctx, ReadStrong, "SELECT status FROM organization_memberships WHERE user_id=? AND organization_id=?", user.ID, org)
	if err != nil {
		return MaterializedUserView{}, err
	}
	status := "none"
	if len(rows) > 0 {
		status = rowString(rows[0], "status")
	}
	return MaterializedUserView{UserID: user.ID, OrganizationID: org, MembershipStatus: status}, nil
}
func (s *Service) SetUserActiveAdmin(ctx context.Context, id string, active bool) (User, error) {
	return s.store.SetUserActive(ctx, id, active)
}
func (s *Service) HealthStatus(ctx context.Context) HealthStatus { return s.store.Health(ctx) }
func (s *Service) IsReady(ctx context.Context) bool              { h := s.store.Health(ctx); return h.ReadReady }
func (s *Service) Bootstrap(ctx context.Context) error           { return s.store.EnsureBootstrapData(ctx) }
func (s *Service) Initialize(ctx context.Context) error {
	if err := s.MigrateSchema(ctx); err != nil {
		return err
	}
	return s.Bootstrap(ctx)
}
