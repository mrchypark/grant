package grant

import (
	"context"
	"fmt"
)

func (s *Service) AuthenticateRequestForInactiveOrganization(ctx context.Context, token, org string, c ReadConsistency) (AuthenticatedPrincipal, error) {
	org, err := required("organizationId", org)
	if err != nil {
		return AuthenticatedPrincipal{}, err
	}
	i, err := s.resolveExternalIdentity(ctx, token, c)
	if err != nil {
		return AuthenticatedPrincipal{}, err
	}
	if i.Provider == "pocketbase" {
		c = ReadStrong
	}
	u, err := s.localIdentity(ctx, i, c)
	if err != nil {
		return AuthenticatedPrincipal{}, err
	}
	if !u.Active {
		return AuthenticatedPrincipal{}, serviceError(403, "forbidden: local user is inactive")
	}
	rows, err := s.store.Query(ctx, s.effectiveConsistency(c), "SELECT id FROM organization_memberships WHERE user_id=? AND organization_id=? AND status='active'", u.ID, org)
	if err != nil {
		return AuthenticatedPrincipal{}, err
	}
	if len(rows) == 0 {
		return AuthenticatedPrincipal{}, serviceError(403, fmt.Sprintf("forbidden: user %s is not an active member of organization %s", u.ID, org))
	}
	return AuthenticatedPrincipal{UserID: u.ID, OrganizationID: org}, nil
}
func (s *Service) DeactivateOrganization(ctx context.Context, user, org, target string) (Organization, error) {
	if err := s.requireOrganizationAction(ctx, user, org, target, "organization.permission.manage", "deactivate organization", ReadStrong); err != nil {
		return Organization{}, err
	}
	err := lifecycleError(s.store.Transaction(ctx, []Statement{
		lifecycleActorTargetGuard(user, org, target, "organization.permission.manage", false),
		{SQL: "UPDATE organizations SET inactivated_at=COALESCE(inactivated_at,?),updated_at=? WHERE id=?", Args: []any{nowString(), nowString(), target}, ExpectedRowsAffected: requireOne()},
	}))
	if err != nil {
		return Organization{}, err
	}
	return serviceRow[Organization](ctx, s, ReadStrong, "SELECT * FROM organizations WHERE id=?", target)
}
func (s *Service) ReactivateOrganization(ctx context.Context, user, org, target string) (Organization, error) {
	if org != target {
		return Organization{}, serviceError(403, "forbidden: organization scope mismatch")
	}
	ds, err := s.store.evaluateUserActions(ctx, []UserActionCheck{{UserID: user, OrganizationID: org, ProviderID: "grant", ResourceType: "organization", ResourceID: target, ActionID: "organization.permission.manage", Consistency: ReadStrong}}, true)
	if err != nil {
		return Organization{}, err
	}
	if len(ds) != 1 || !ds[0].Allowed {
		return Organization{}, serviceError(403, fmt.Sprintf("forbidden: user %s cannot reactivate organization %s", user, target))
	}
	err = lifecycleError(s.store.Transaction(ctx, []Statement{
		lifecycleActorTargetGuard(user, org, target, "organization.permission.manage", true),
		{SQL: "UPDATE organizations SET inactivated_at=NULL,updated_at=? WHERE id=?", Args: []any{nowString(), target}, ExpectedRowsAffected: requireOne()},
	}))
	if err != nil {
		return Organization{}, err
	}
	return serviceRow[Organization](ctx, s, ReadStrong, "SELECT * FROM organizations WHERE id=?", target)
}
