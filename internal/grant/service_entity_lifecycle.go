package grant

import (
	"context"
	"strings"
	"time"
)

// MetadataPatch changes only supplied display fields. An empty description clears it.
// Identifiers, ownership, client bindings and security policies are immutable here.
type MetadataPatch struct {
	Name        *string   `json:"name"`
	Description PatchText `json:"description"`
}

type ApplicationPatch struct {
	Name        *string   `json:"name"`
	Description PatchText `json:"description"`
	RedirectURI PatchText `json:"redirectUri"`
	Active      *bool     `json:"active"`
}

// ApplicationMetadataPatch is intentionally narrower than application create:
// redirectUri may be set before identity binding, but identity and security
// metadata remain immutable through this user route.
type ApplicationMetadataPatch struct {
	Name        *string   `json:"name"`
	RedirectURI PatchText `json:"redirectUri"`
}

func lifecycleActorGuard(user, org, action string) Statement {
	return lifecycleActorTargetGuard(user, org, org, action, false)
}

func lifecycleActorTargetGuard(user, org, target, action string, inactive bool) Statement {
	q := strings.Replace(authorizationSQL, "{values}", "(?, ?, ?, ?, ?, ?, ?, ?, ?, ?)", 1)
	return assertion("SELECT 1 FROM ("+q+") WHERE allowed=1", int64(0), user, org, "grant", "organization", target, action, nil, time.Now().UTC().Format(agreementTimeLayout), inactive)
}

func lifecycleError(err error) error {
	if err != nil && strings.Contains(err.Error(), "transaction rejected") {
		return serviceError(409, "lifecycle preconditions changed")
	}
	return err
}

func metadataStatements(table, id, org string, in MetadataPatch) ([]Statement, error) {
	// table is a server-owned constant at every call site.
	var out []Statement
	if in.Name != nil {
		name, err := required("name", *in.Name)
		if err != nil {
			return nil, err
		}
		out = append(out, Statement{SQL: "UPDATE " + table + " SET name=?,updated_at=? WHERE id=? AND organization_id=?", Args: []any{name, nowString(), id, org}, ExpectedRowsAffected: requireOne()})
	}
	if in.Description.Present {
		out = append(out, Statement{SQL: "UPDATE " + table + " SET description=?,updated_at=? WHERE id=? AND organization_id=?", Args: []any{optionalText(in.Description.Value), nowString(), id, org}, ExpectedRowsAffected: requireOne()})
	}
	return out, nil
}

func (s *Service) patchApplicationLifecycle(ctx context.Context, user, org, id string, in ApplicationPatch) (Application, error) {
	a, err := s.GetApplicationAuthorized(ctx, user, org, id, ReadStrong)
	if err != nil {
		return Application{}, err
	}
	if in.Name == nil && !in.Description.Present && !in.RedirectURI.Present && in.Active == nil {
		return Application{}, serviceError(400, "at least one patch field is required")
	}
	var redirectURI *string
	var previousRedirectURI *string
	if in.RedirectURI.Present {
		if in.RedirectURI.Value == nil {
			return Application{}, serviceError(400, "redirectUri must not be null")
		}
		canonical, canonicalErr := CanonicalRedirectURI(*in.RedirectURI.Value)
		if canonicalErr != nil {
			return Application{}, canonicalErr
		}
		redirectURI = &canonical
		if a.FirstParty || a.ProvisioningStatus != "unprovisioned" || a.RauthyClientID != nil {
			return Application{}, serviceError(409, "redirectUri can only change before identity binding")
		}
		previousRedirectURI, err = s.applicationRedirect(ctx, id, ReadStrong)
		if err != nil {
			return Application{}, err
		}
	}
	if in.Active != nil && a.FirstParty {
		return Application{}, serviceError(403, "platform-managed applications cannot change lifecycle through organization APIs")
	}
	if err = s.applicationMutationAvailable(ctx, id); err != nil {
		return Application{}, err
	}
	ss, err := metadataStatements("applications", id, org, MetadataPatch{Name: in.Name, Description: in.Description})
	if err != nil {
		return Application{}, err
	}
	guards := []Statement{lifecycleActorGuard(user, org, "organization.application.manage"), assertion("SELECT 1 FROM applications WHERE id=? AND organization_id=? AND (security_mutation_token IS NULL OR security_mutation_expires_at<=?)", id, org, nowString())}
	if in.RedirectURI.Present {
		uriCount := 0
		var previousURI any
		if previousRedirectURI != nil {
			uriCount = 1
			previousURI = *previousRedirectURI
		}
		now := nowString()
		guards = append(guards,
			assertion("SELECT 1 FROM applications WHERE id=? AND organization_id=? AND first_party=0 AND provisioning_status='unprovisioned' AND rauthy_client_id IS NULL AND (security_mutation_token IS NULL OR security_mutation_expires_at<=?)", id, org, now),
			assertion("SELECT 1 WHERE (SELECT COUNT(*) FROM application_redirect_uris WHERE application_id=?)=? AND NOT EXISTS(SELECT 1 FROM application_redirect_uris WHERE application_id=? AND redirect_uri IS NOT ?)", id, uriCount, id, previousURI),
		)
		ss = append(ss,
			Statement{SQL: "UPDATE applications SET updated_at=? WHERE id=? AND organization_id=? AND first_party=0 AND provisioning_status='unprovisioned' AND rauthy_client_id IS NULL AND (security_mutation_token IS NULL OR security_mutation_expires_at<=?)", Args: []any{now, id, org, now}, ExpectedRowsAffected: requireOne()},
			Statement{SQL: "DELETE FROM application_redirect_uris WHERE application_id=?", Args: []any{id}},
			Statement{SQL: "INSERT INTO application_redirect_uris(application_id,redirect_uri,created_at) VALUES(?,?,?)", Args: []any{id, *redirectURI, now}},
		)
	}
	if in.Active != nil {
		guards = append(guards, assertion("SELECT 1 FROM applications WHERE id=? AND first_party=0", id))
		ss = append(ss, Statement{SQL: "UPDATE applications SET active=?,updated_at=? WHERE id=? AND organization_id=?", Args: []any{*in.Active, nowString(), id, org}, ExpectedRowsAffected: requireOne()})
		if !*in.Active {
			ss = append(ss, Statement{SQL: "UPDATE application_grants SET status='revoked',revoked_at=?,revoked_by_user_id=?,updated_at=? WHERE application_id=? AND status='active'", Args: []any{nowString(), user, nowString(), id}})
		}
	}
	if err = s.store.Transaction(ctx, append(guards, ss...)); err != nil {
		return Application{}, lifecycleError(err)
	}
	return s.store.GetApplicationWithConsistency(ctx, id, ReadStrong)
}

func (s *Service) DeleteApplicationAuthorized(ctx context.Context, user, org, id string) error {
	a, err := s.GetApplicationAuthorized(ctx, user, org, id, ReadStrong)
	if err != nil {
		return err
	}
	if a.FirstParty {
		return serviceError(403, "platform-managed applications cannot be deleted through organization APIs")
	}
	if err = s.applicationMutationAvailable(ctx, id); err != nil {
		return err
	}
	return lifecycleError(s.store.Transaction(ctx, []Statement{
		lifecycleActorGuard(user, org, "organization.application.manage"),
		assertion("SELECT 1 FROM applications WHERE id=? AND organization_id=? AND first_party=0 AND (security_mutation_token IS NULL OR security_mutation_expires_at<=?)", id, org, nowString()),
		{SQL: "DELETE FROM application_grant_actions WHERE application_grant_id IN(SELECT id FROM application_grants WHERE application_id=?)", Args: []any{id}},
		{SQL: "DELETE FROM application_grants WHERE application_id=?", Args: []any{id}},
		{SQL: "DELETE FROM application_actions WHERE application_id=?", Args: []any{id}},
		{SQL: "DELETE FROM application_redirect_uris WHERE application_id=?", Args: []any{id}},
		{SQL: "DELETE FROM applications WHERE id=? AND organization_id=?", Args: []any{id, org}, ExpectedRowsAffected: requireOne()},
	}))
}

func (s *Service) PatchRoleAuthorized(ctx context.Context, user, org, id string, in MetadataPatch) (Role, error) {
	if err := s.requireOrganizationAction(ctx, user, org, org, "organization.permission.manage", "manage organization roles", ReadStrong); err != nil {
		return Role{}, err
	}
	if _, err := serviceRow[Role](ctx, s, ReadStrong, "SELECT * FROM roles WHERE id=? AND organization_id=?", id, org); err != nil {
		return Role{}, err
	}
	if in.Name == nil && !in.Description.Present {
		return Role{}, serviceError(400, "at least one patch field is required")
	}
	ss, err := metadataStatements("roles", id, org, in)
	if err != nil {
		return Role{}, err
	}
	if err = s.store.Transaction(ctx, append([]Statement{lifecycleActorGuard(user, org, "organization.permission.manage")}, ss...)); err != nil {
		return Role{}, lifecycleError(err)
	}
	return serviceRow[Role](ctx, s, ReadStrong, "SELECT * FROM roles WHERE id=? AND organization_id=?", id, org)
}

func (s *Service) DeleteRoleAuthorized(ctx context.Context, user, org, id string) error {
	if err := s.requireOrganizationAction(ctx, user, org, org, "organization.permission.manage", "manage organization roles", ReadStrong); err != nil {
		return err
	}
	if _, err := serviceRow[Role](ctx, s, ReadStrong, "SELECT * FROM roles WHERE id=? AND organization_id=?", id, org); err != nil {
		return err
	}
	refs, err := s.store.Query(ctx, ReadStrong, "SELECT id FROM role_bindings WHERE role_id=? LIMIT 1", id)
	if err != nil {
		return err
	}
	if len(refs) > 0 {
		return serviceError(409, "remove role bindings before deleting the role")
	}
	return lifecycleError(s.store.Transaction(ctx, []Statement{lifecycleActorGuard(user, org, "organization.permission.manage"), assertion("SELECT 1 FROM roles WHERE id=? AND organization_id=?", id, org), assertion("SELECT 1 WHERE NOT EXISTS(SELECT 1 FROM role_bindings WHERE role_id=?)", id),
		{SQL: "DELETE FROM role_action_field_views WHERE role_id=?", Args: []any{id}},
		{SQL: "DELETE FROM role_actions WHERE role_id=?", Args: []any{id}},
		{SQL: "DELETE FROM roles WHERE id=? AND organization_id=?", Args: []any{id, org}, ExpectedRowsAffected: requireOne()},
	}))
}

func (s *Service) ListOrganizationMembersAuthorized(ctx context.Context, user, org string, c ReadConsistency) ([]OrganizationMembership, error) {
	if err := s.requireOrganizationAction(ctx, user, org, org, "organization.member.manage", "manage organization members", c); err != nil {
		return nil, err
	}
	return serviceRows[OrganizationMembership](ctx, s, c, "SELECT * FROM organization_memberships WHERE organization_id=? ORDER BY user_id", org)
}

// Removing membership also revokes user-specific access. Re-enrollment never restores it.
// Owners require a separate ownership transfer procedure and cannot be removed here.
func (s *Service) RemoveOrganizationMemberAuthorized(ctx context.Context, user, org, target string) error {
	if err := s.requireOrganizationAction(ctx, user, org, org, "organization.member.manage", "manage organization members", ReadStrong); err != nil {
		return err
	}
	m, err := serviceRow[OrganizationMembership](ctx, s, ReadStrong, "SELECT * FROM organization_memberships WHERE organization_id=? AND user_id=?", org, target)
	if err != nil {
		return err
	}
	if m.MembershipRole == "owner" {
		return serviceError(409, "transfer ownership before removing an owner")
	}
	return lifecycleError(s.store.Transaction(ctx, []Statement{lifecycleActorGuard(user, org, "organization.member.manage"), assertion("SELECT 1 FROM organization_memberships WHERE organization_id=? AND user_id=? AND membership_role<>'owner'", org, target),
		{SQL: "DELETE FROM group_memberships WHERE organization_membership_id IN(SELECT id FROM organization_memberships WHERE organization_id=? AND user_id=?)", Args: []any{org, target}},
		{SQL: "DELETE FROM role_bindings WHERE subject_type='user' AND subject_id=? AND subject_organization_id=?", Args: []any{target, org}},
		{SQL: "UPDATE application_grants SET status='revoked',revoked_at=?,revoked_by_user_id=?,updated_at=? WHERE organization_id=? AND user_id=? AND status='active'", Args: []any{nowString(), user, nowString(), org, target}},
		{SQL: "UPDATE organization_memberships SET membership_role='member',status='inactive',updated_at=? WHERE organization_id=? AND user_id=?", Args: []any{nowString(), org, target}, ExpectedRowsAffected: requireOne()},
	}))
}

func (s *Service) RevokeApplicationGrantAuthorized(ctx context.Context, user, org, id string) error {
	if err := s.requireOrganizationAction(ctx, user, org, org, "organization.application.manage", "manage organization applications", ReadStrong); err != nil {
		return err
	}
	if _, err := serviceRow[ApplicationGrant](ctx, s, ReadStrong, "SELECT * FROM application_grants WHERE id=? AND organization_id=?", id, org); err != nil {
		return err
	}
	return lifecycleError(s.store.Transaction(ctx, []Statement{lifecycleActorGuard(user, org, "organization.application.manage"), assertion("SELECT 1 FROM application_grants WHERE id=? AND organization_id=?", id, org), {SQL: "UPDATE application_grants SET status='revoked',revoked_at=?,revoked_by_user_id=?,updated_at=? WHERE id=? AND organization_id=? AND status='active'", Args: []any{nowString(), user, nowString(), id, org}}}))
}

func (s *Service) PatchApplicationAuthorized(ctx context.Context, user, org, id string, in MetadataPatch) (Application, error) {
	return s.patchApplicationLifecycle(ctx, user, org, id, ApplicationPatch{Name: in.Name, Description: in.Description})
}

func (s *Service) PatchApplicationMetadataAuthorized(ctx context.Context, user, org, id string, in ApplicationMetadataPatch) (Application, error) {
	return s.patchApplicationLifecycle(ctx, user, org, id, ApplicationPatch{Name: in.Name, RedirectURI: in.RedirectURI})
}

func (s *Service) SetApplicationActiveAuthorized(ctx context.Context, user, org, id string, active bool) (Application, error) {
	return s.patchApplicationLifecycle(ctx, user, org, id, ApplicationPatch{Active: &active})
}
