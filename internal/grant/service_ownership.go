package grant

import (
	"context"
	"crypto/sha256"
	"encoding/hex"
	"encoding/json"
)

// OrganizationOwnerSnapshot is the owner relation and its opaque concurrency token.
// Membership storage metadata participates in the token but is not public API data.
type OrganizationOwnerSnapshot struct {
	OrganizationID string `json:"organizationId"`
	OwnerUserID    string `json:"ownerUserId"`
	ETag           string `json:"-"`
	membershipID   string
	updatedAt      string
}

func (s *Service) organizationOwnerSnapshot(ctx context.Context, org string) (OrganizationOwnerSnapshot, error) {
	rows, err := s.store.Query(ctx, ReadStrong, "SELECT id,user_id,updated_at FROM organization_memberships WHERE organization_id=? AND membership_role='owner'", org)
	if err != nil {
		return OrganizationOwnerSnapshot{}, err
	}
	if len(rows) != 1 {
		return OrganizationOwnerSnapshot{}, serviceError(409, "organization must have exactly one owner before transfer")
	}
	v := OrganizationOwnerSnapshot{OrganizationID: org, OwnerUserID: rowString(rows[0], "user_id"), membershipID: rowString(rows[0], "id"), updatedAt: rowString(rows[0], "updated_at")}
	encoded, _ := json.Marshal([]string{"grant.organization-owner.v1", org, v.OwnerUserID, v.membershipID, v.updatedAt})
	digest := sha256.Sum256(encoded)
	v.ETag = `"` + hex.EncodeToString(digest[:]) + `"`
	return v, nil
}

func (s *Service) GetOrganizationOwnerAuthorized(ctx context.Context, user, org string) (OrganizationOwnerSnapshot, error) {
	if err := s.requireOrganizationAction(ctx, user, org, org, "organization.profile.read", "read organization owner", ReadStrong); err != nil {
		return OrganizationOwnerSnapshot{}, err
	}
	return s.organizationOwnerSnapshot(ctx, org)
}

func (s *Service) TransferOrganizationOwnerAuthorized(ctx context.Context, user, org, target, match string) (OrganizationOwnerSnapshot, error) {
	if match == "" {
		return OrganizationOwnerSnapshot{}, serviceError(428, "If-Match is required")
	}
	target, err := required("ownerUserId", target)
	if err != nil {
		return OrganizationOwnerSnapshot{}, err
	}
	before, err := s.organizationOwnerSnapshot(ctx, org)
	if err != nil {
		return OrganizationOwnerSnapshot{}, err
	}
	if before.OwnerUserID != user {
		return OrganizationOwnerSnapshot{}, serviceError(403, "only the current organization owner can transfer ownership")
	}
	if err := s.requireOrganizationAction(ctx, user, org, org, "organization.permission.manage", "transfer organization ownership", ReadStrong); err != nil {
		return OrganizationOwnerSnapshot{}, err
	}
	if before.ETag != match {
		return OrganizationOwnerSnapshot{}, serviceError(412, "organization owner changed; read the current ETag")
	}
	if user == target {
		return OrganizationOwnerSnapshot{}, serviceError(400, "new owner must differ from the current owner")
	}
	rows, err := s.store.Query(ctx, ReadStrong, "SELECT m.user_id FROM organization_memberships m JOIN users u ON u.id=m.user_id JOIN organizations o ON o.id=m.organization_id WHERE m.organization_id=? AND m.user_id IN (?,?) AND m.status='active' AND u.active=1 AND o.inactivated_at IS NULL", org, user, target)
	if err != nil {
		return OrganizationOwnerSnapshot{}, err
	}
	if len(rows) != 2 {
		return OrganizationOwnerSnapshot{}, serviceError(400, "both owners must be active users and active members of an active organization")
	}
	now := nowString()
	statements := []Statement{
		lifecycleActorGuard(user, org, "organization.permission.manage"),
		assertion("SELECT 1 FROM organization_memberships WHERE id=? AND organization_id=? AND user_id=? AND membership_role='owner' AND status='active' AND updated_at=?", before.membershipID, org, user, before.updatedAt),
		assertion("SELECT 1 WHERE (SELECT COUNT(*) FROM organization_memberships WHERE organization_id=? AND membership_role='owner')=1", org),
		assertion("SELECT 1 WHERE (SELECT COUNT(*) FROM organization_memberships m JOIN users u ON u.id=m.user_id JOIN organizations o ON o.id=m.organization_id WHERE m.organization_id=? AND m.user_id IN (?,?) AND m.status='active' AND u.active=1 AND o.inactivated_at IS NULL)=2", org, user, target),
		assertion("SELECT 1 FROM roles r JOIN service_providers p ON p.id='grant' AND p.active=1 WHERE r.id='role-system-bootstrap-owner' AND r.organization_id IS NULL"),
		{SQL: "UPDATE organization_memberships SET membership_role='member',updated_at=? WHERE id=? AND membership_role='owner'", Args: []any{now, before.membershipID}, ExpectedRowsAffected: requireOne()},
		{SQL: "UPDATE organization_memberships SET membership_role='owner',updated_at=? WHERE organization_id=? AND user_id=? AND status='active'", Args: []any{now, org, target}, ExpectedRowsAffected: requireOne()},
		// Only onboarding-created management ownership follows the owner. Copy
		// the current state instead of resurrecting a revoked/disabled binding.
		{SQL: "INSERT INTO role_bindings(id,subject_type,subject_id,subject_organization_id,provider_id,resource_type,resource_id,role_id,agreement_id,state,created_by_user_id,created_at,updated_at) SELECT ?,'user',?,?,'grant-management','organization',?,'role-system-management-owner',agreement_id,state,?,?,? FROM role_bindings WHERE subject_type='user' AND subject_id=? AND subject_organization_id=? AND provider_id='grant-management' AND resource_type='organization' AND resource_id=? AND role_id='role-system-management-owner' ON CONFLICT(subject_type,subject_id,subject_organization_id,provider_id,resource_type,resource_id,role_id) DO UPDATE SET state=excluded.state,agreement_id=excluded.agreement_id,updated_at=excluded.updated_at", Args: []any{requestID(), target, org, org, user, now, now, user, org, org}},
		{SQL: "DELETE FROM role_bindings WHERE subject_type='user' AND subject_id=? AND subject_organization_id=? AND provider_id='grant-management' AND resource_type='organization' AND resource_id=? AND role_id='role-system-management-owner'", Args: []any{user, org, org}},
		{SQL: "DELETE FROM role_bindings WHERE subject_type='user' AND subject_id=? AND subject_organization_id=? AND provider_id='grant' AND resource_type='organization' AND resource_id=? AND role_id='role-system-bootstrap-owner'", Args: []any{user, org, org}},
		{SQL: "INSERT INTO role_bindings(id,subject_type,subject_id,subject_organization_id,provider_id,resource_type,resource_id,role_id,state,created_by_user_id,created_at,updated_at) VALUES(?,'user',?,?,'grant','organization',?,'role-system-bootstrap-owner','active',?,?,?) ON CONFLICT(subject_type,subject_id,subject_organization_id,provider_id,resource_type,resource_id,role_id) DO UPDATE SET state='active',agreement_id=NULL,updated_at=excluded.updated_at", Args: []any{requestID(), target, org, org, user, now, now}},
	}
	if err = s.store.Transaction(ctx, statements); err != nil {
		current, readErr := s.organizationOwnerSnapshot(ctx, org)
		if readErr == nil && current.ETag != before.ETag {
			return OrganizationOwnerSnapshot{}, serviceError(412, "organization owner changed; read the current ETag")
		}
		return OrganizationOwnerSnapshot{}, lifecycleError(err)
	}
	return s.organizationOwnerSnapshot(ctx, org)
}
