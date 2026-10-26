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
	OrganizationID string              `json:"organizationId"`
	Owners         []OrganizationOwner `json:"owners"`
	ETag           string              `json:"-"`
	OwnerUserID    string              `json:"-"`
	membershipID   string
	updatedAt      string
	ownerStates    []ownerMembershipState
}

type ownerMembershipState struct{ userID, membershipID, updatedAt string }

type OrganizationOwnerTransferSnapshot struct {
	OrganizationID          string `json:"organizationId"`
	OwnerUserID             string `json:"ownerUserId"`
	ETag                    string `json:"-"`
	membershipID, updatedAt string
}

type OrganizationOwner struct {
	UserID string `json:"userId"`
}

func (s *Service) organizationOwnerSnapshot(ctx context.Context, org string) (OrganizationOwnerSnapshot, error) {
	rows, err := s.store.Query(ctx, ReadStrong, "SELECT m.id,m.user_id,m.updated_at FROM organization_memberships m JOIN users u ON u.id=m.user_id JOIN organizations o ON o.id=m.organization_id WHERE m.organization_id=? AND m.membership_role='owner' AND m.status='active' AND u.active=1 AND o.inactivated_at IS NULL ORDER BY m.user_id", org)
	if err != nil {
		return OrganizationOwnerSnapshot{}, err
	}
	if len(rows) == 0 {
		return OrganizationOwnerSnapshot{}, serviceError(409, "organization must have an active owner")
	}
	v := OrganizationOwnerSnapshot{OrganizationID: org, Owners: make([]OrganizationOwner, 0, len(rows))}
	state := []string{"grant.organization-owners.v1", org}
	for _, row := range rows {
		id, userID, updatedAt := rowString(row, "id"), rowString(row, "user_id"), rowString(row, "updated_at")
		v.Owners = append(v.Owners, OrganizationOwner{UserID: userID})
		state = append(state, userID, id, updatedAt)
		v.ownerStates = append(v.ownerStates, ownerMembershipState{userID, id, updatedAt})
		if len(rows) == 1 {
			v.OwnerUserID, v.membershipID, v.updatedAt = userID, id, updatedAt
		}
	}
	encoded, _ := json.Marshal(state)
	digest := sha256.Sum256(encoded)
	v.ETag = `"` + hex.EncodeToString(digest[:]) + `"`
	return v, nil
}

func (s *Service) ListOrganizationOwnersAuthorized(ctx context.Context, user, org string) (OrganizationOwnerSnapshot, error) {
	if !s.isActiveOrganizationOwner(ctx, user, org) {
		return OrganizationOwnerSnapshot{}, serviceError(403, "only an active organization owner can read organization owners")
	}
	if err := s.requireOrganizationAction(ctx, user, org, org, "organization.profile.read", "read organization owners", ReadStrong); err != nil {
		return OrganizationOwnerSnapshot{}, err
	}
	return s.organizationOwnerSnapshot(ctx, org)
}

func (s *Service) ChangeOrganizationOwnerAuthorized(ctx context.Context, user, org, target, match string, add bool) (OrganizationOwnerSnapshot, error) {
	if match == "" {
		return OrganizationOwnerSnapshot{}, serviceError(428, "If-Match is required")
	}
	target, err := required("userId", target)
	if err != nil {
		return OrganizationOwnerSnapshot{}, err
	}
	if err := s.requireOrganizationAction(ctx, user, org, org, "organization.permission.manage", "manage organization owners", ReadStrong); err != nil {
		return OrganizationOwnerSnapshot{}, err
	}
	if !s.isActiveOrganizationOwner(ctx, user, org) {
		return OrganizationOwnerSnapshot{}, serviceError(403, "only an active organization owner can manage owners")
	}
	before, err := s.organizationOwnerSnapshot(ctx, org)
	if err != nil {
		return OrganizationOwnerSnapshot{}, err
	}
	if before.ETag != match {
		return OrganizationOwnerSnapshot{}, serviceError(412, "organization owners changed; read the current ETag")
	}
	now := nowString()
	statements := []Statement{lifecycleActorGuard(user, org, "organization.permission.manage")}
	statements = append(statements,
		assertion("SELECT 1 FROM organization_memberships m JOIN users u ON u.id=m.user_id JOIN organizations o ON o.id=m.organization_id WHERE m.organization_id=? AND m.user_id=? AND m.membership_role='owner' AND m.status='active' AND u.active=1 AND o.inactivated_at IS NULL", org, user),
		assertion("SELECT 1 WHERE (SELECT COUNT(*) FROM organization_memberships m JOIN users u ON u.id=m.user_id JOIN organizations o ON o.id=m.organization_id WHERE m.organization_id=? AND m.membership_role='owner' AND m.status='active' AND u.active=1 AND o.inactivated_at IS NULL)=?", org, int64(len(before.Owners))),
	)
	for _, owner := range before.ownerStates {
		statements = append(statements, assertion("SELECT 1 FROM organization_memberships m JOIN users u ON u.id=m.user_id JOIN organizations o ON o.id=m.organization_id WHERE m.organization_id=? AND m.user_id=? AND m.membership_role='owner' AND m.status='active' AND u.active=1 AND o.inactivated_at IS NULL AND m.updated_at=? AND m.id=?", org, owner.userID, owner.updatedAt, owner.membershipID))
	}
	if add {
		statements = append(statements,
			assertion("SELECT 1 WHERE EXISTS(SELECT 1 FROM users u JOIN organization_memberships m ON m.user_id=u.id JOIN organizations o ON o.id=m.organization_id WHERE u.id=? AND u.active=1 AND m.organization_id=? AND m.status='active' AND o.inactivated_at IS NULL)", target, org),
			Statement{SQL: "UPDATE organization_memberships SET membership_role='owner',updated_at=? WHERE organization_id=? AND user_id=? AND status='active' AND membership_role<>'owner'", Args: []any{now, org, target}},
			assertion("SELECT 1 FROM organization_memberships WHERE organization_id=? AND user_id=? AND status='active' AND membership_role='owner'", org, target),
		)
	} else {
		statements = append(statements,
			assertion("SELECT 1 FROM organization_memberships m JOIN users u ON u.id=m.user_id JOIN organizations o ON o.id=m.organization_id WHERE m.organization_id=? AND m.user_id=? AND m.membership_role='owner' AND m.status='active' AND u.active=1 AND o.inactivated_at IS NULL", org, target),
			assertion("SELECT 1 WHERE (SELECT COUNT(*) FROM organization_memberships m JOIN users u ON u.id=m.user_id JOIN organizations o ON o.id=m.organization_id WHERE m.organization_id=? AND m.membership_role='owner' AND m.status='active' AND u.active=1 AND o.inactivated_at IS NULL)>1", org),
			Statement{SQL: "UPDATE organization_memberships SET membership_role='member',updated_at=? WHERE organization_id=? AND user_id=? AND membership_role='owner' AND status='active'", Args: []any{now, org, target}, ExpectedRowsAffected: requireOne()},
			Statement{SQL: "DELETE FROM role_bindings WHERE subject_type='user' AND subject_id=? AND subject_organization_id=? AND role_id IN('role-system-bootstrap-owner','role-system-management-owner')", Args: []any{target, org}},
		)
	}
	if err = s.store.Transaction(ctx, statements); err != nil {
		current, readErr := s.organizationOwnerSnapshot(ctx, org)
		if readErr == nil {
			if !add && len(current.Owners) == 1 && current.Owners[0].UserID == target {
				return OrganizationOwnerSnapshot{}, serviceError(409, "cannot remove the last active organization owner")
			}
			if current.ETag != before.ETag {
				return OrganizationOwnerSnapshot{}, serviceError(412, "organization owners changed; read the current ETag")
			}
		}
		return OrganizationOwnerSnapshot{}, lifecycleError(err)
	}
	return s.organizationOwnerSnapshot(ctx, org)
}

func (s *Service) isActiveOrganizationOwner(ctx context.Context, user, org string) bool {
	rows, err := s.store.Query(ctx, ReadStrong, "SELECT 1 FROM organization_memberships m JOIN users u ON u.id=m.user_id JOIN organizations o ON o.id=m.organization_id WHERE m.organization_id=? AND m.user_id=? AND m.membership_role='owner' AND m.status='active' AND u.active=1 AND o.inactivated_at IS NULL", org, user)
	return err == nil && len(rows) == 1
}

func (s *Service) GetOrganizationOwnerAuthorized(ctx context.Context, user, org string) (OrganizationOwnerTransferSnapshot, error) {
	if err := s.requireOrganizationAction(ctx, user, org, org, "organization.profile.read", "read organization owner", ReadStrong); err != nil {
		return OrganizationOwnerTransferSnapshot{}, err
	}
	v, err := s.organizationOwnerSnapshot(ctx, org)
	if err != nil {
		return OrganizationOwnerTransferSnapshot{}, err
	}
	if len(v.Owners) != 1 {
		return OrganizationOwnerTransferSnapshot{}, serviceError(409, "legacy owner endpoint is unavailable when an organization has multiple owners")
	}
	return OrganizationOwnerTransferSnapshot{OrganizationID: org, OwnerUserID: v.Owners[0].UserID, ETag: v.ETag, membershipID: v.ownerStates[0].membershipID, updatedAt: v.ownerStates[0].updatedAt}, nil
}

func (s *Service) TransferOrganizationOwnerAuthorized(ctx context.Context, user, org, target, match string) (OrganizationOwnerTransferSnapshot, error) {
	if match == "" {
		return OrganizationOwnerTransferSnapshot{}, serviceError(428, "If-Match is required")
	}
	target, err := required("ownerUserId", target)
	if err != nil {
		return OrganizationOwnerTransferSnapshot{}, err
	}
	before, err := s.organizationOwnerSnapshot(ctx, org)
	if err != nil {
		return OrganizationOwnerTransferSnapshot{}, err
	}
	if len(before.Owners) != 1 {
		return OrganizationOwnerTransferSnapshot{}, serviceError(409, "legacy ownership transfer is unavailable when an organization has multiple owners")
	}
	if before.Owners[0].UserID != user {
		return OrganizationOwnerTransferSnapshot{}, serviceError(403, "only the current organization owner can transfer ownership")
	}
	if err := s.requireOrganizationAction(ctx, user, org, org, "organization.permission.manage", "transfer organization ownership", ReadStrong); err != nil {
		return OrganizationOwnerTransferSnapshot{}, err
	}
	if before.ETag != match {
		return OrganizationOwnerTransferSnapshot{}, serviceError(412, "organization owner changed; read the current ETag")
	}
	if user == target {
		return OrganizationOwnerTransferSnapshot{}, serviceError(400, "new owner must differ from the current owner")
	}
	rows, err := s.store.Query(ctx, ReadStrong, "SELECT m.user_id FROM organization_memberships m JOIN users u ON u.id=m.user_id JOIN organizations o ON o.id=m.organization_id WHERE m.organization_id=? AND m.user_id IN (?,?) AND m.status='active' AND u.active=1 AND o.inactivated_at IS NULL", org, user, target)
	if err != nil {
		return OrganizationOwnerTransferSnapshot{}, err
	}
	if len(rows) != 2 {
		return OrganizationOwnerTransferSnapshot{}, serviceError(400, "both owners must be active users and active members of an active organization")
	}
	now := nowString()
	statements := []Statement{
		lifecycleActorGuard(user, org, "organization.permission.manage"),
		assertion("SELECT 1 FROM organization_memberships WHERE id=? AND organization_id=? AND user_id=? AND membership_role='owner' AND status='active' AND updated_at=?", before.ownerStates[0].membershipID, org, user, before.ownerStates[0].updatedAt),
		assertion("SELECT 1 WHERE (SELECT COUNT(*) FROM organization_memberships m JOIN users u ON u.id=m.user_id JOIN organizations o ON o.id=m.organization_id WHERE m.organization_id=? AND m.membership_role='owner' AND m.status='active' AND u.active=1 AND o.inactivated_at IS NULL)=1", org),
		assertion("SELECT 1 WHERE (SELECT COUNT(*) FROM organization_memberships m JOIN users u ON u.id=m.user_id JOIN organizations o ON o.id=m.organization_id WHERE m.organization_id=? AND m.user_id IN (?,?) AND m.status='active' AND u.active=1 AND o.inactivated_at IS NULL)=2", org, user, target),
		assertion("SELECT 1 FROM roles r JOIN service_providers p ON p.id='grant' AND p.active=1 WHERE r.id='role-system-bootstrap-owner' AND r.organization_id IS NULL"),
		{SQL: "UPDATE organization_memberships SET membership_role='member',updated_at=? WHERE id=? AND membership_role='owner'", Args: []any{now, before.ownerStates[0].membershipID}, ExpectedRowsAffected: requireOne()},
		{SQL: "UPDATE organization_memberships SET membership_role='owner',updated_at=? WHERE organization_id=? AND user_id=? AND status='active'", Args: []any{now, org, target}, ExpectedRowsAffected: requireOne()},
		{SQL: "DELETE FROM role_bindings WHERE subject_type='user' AND subject_id=? AND subject_organization_id=? AND role_id IN('role-system-bootstrap-owner','role-system-management-owner')", Args: []any{user, org}},
		{SQL: "DELETE FROM role_bindings WHERE subject_type='user' AND subject_id=? AND subject_organization_id=? AND role_id IN('role-system-bootstrap-owner','role-system-management-owner')", Args: []any{target, org}},
	}
	if err = s.store.Transaction(ctx, statements); err != nil {
		current, readErr := s.organizationOwnerSnapshot(ctx, org)
		if readErr == nil && current.ETag != before.ETag {
			return OrganizationOwnerTransferSnapshot{}, serviceError(412, "organization owner changed; read the current ETag")
		}
		return OrganizationOwnerTransferSnapshot{}, lifecycleError(err)
	}
	updated, err := s.organizationOwnerSnapshot(ctx, org)
	if err != nil {
		return OrganizationOwnerTransferSnapshot{}, err
	}
	return OrganizationOwnerTransferSnapshot{OrganizationID: org, OwnerUserID: updated.Owners[0].UserID, ETag: updated.ETag, membershipID: updated.ownerStates[0].membershipID, updatedAt: updated.ownerStates[0].updatedAt}, nil
}
