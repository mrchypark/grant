package grant

import (
	"context"
	"fmt"
	"strings"
)

func optionalText(v *string) *string {
	if v == nil {
		return nil
	}
	s := strings.TrimSpace(*v)
	if s == "" {
		return nil
	}
	return &s
}
func serviceRows[T any](ctx context.Context, s *Service, c ReadConsistency, query string, args ...any) ([]T, error) {
	rows, err := s.store.Query(ctx, s.effectiveConsistency(c), query, args...)
	if err != nil {
		return nil, err
	}
	return decodeRows[T](rows)
}
func serviceRow[T any](ctx context.Context, s *Service, c ReadConsistency, query string, args ...any) (T, error) {
	var zero T
	rows, err := serviceRows[T](ctx, s, c, query, args...)
	if err != nil {
		return zero, err
	}
	if len(rows) == 0 {
		return zero, serviceError(404, "not found")
	}
	return rows[0], nil
}
func (s *Service) ListGroupsAuthorized(ctx context.Context, user, org string, c ReadConsistency) ([]Group, error) {
	if err := s.requireOrganizationAction(ctx, user, org, org, "organization.group.manage", "manage organization groups", c); err != nil {
		return nil, err
	}
	return serviceRows[Group](ctx, s, c, "SELECT * FROM groups WHERE organization_id=? ORDER BY name,id", org)
}
func (s *Service) GetGroupAuthorized(ctx context.Context, user, org, id string, c ReadConsistency) (Group, error) {
	if err := s.requireOrganizationAction(ctx, user, org, org, "organization.group.manage", "manage organization groups", c); err != nil {
		return Group{}, err
	}
	return serviceRow[Group](ctx, s, c, "SELECT * FROM groups WHERE id=? AND organization_id=?", id, org)
}
func (s *Service) CreateGroupAuthorized(ctx context.Context, user, org string, in UpsertGroupInput) (Group, error) {
	if err := s.requireOrganizationAction(ctx, user, org, org, "organization.group.manage", "manage organization groups", ReadStrong); err != nil {
		return Group{}, err
	}
	id, err := required("id", in.ID)
	if err != nil {
		return Group{}, err
	}
	name, err := required("name", in.Name)
	if err != nil {
		return Group{}, err
	}
	g := Group{ID: id, OrganizationID: org, Name: name, Description: optionalText(in.Description)}
	err = s.store.Transaction(ctx, []Statement{assertion("SELECT 1 FROM organizations WHERE id=?", org), {SQL: "INSERT INTO groups(id,organization_id,name,description,created_at,updated_at) VALUES(?,?,?,?,?,?)", Args: []any{g.ID, org, g.Name, g.Description, nowString(), nowString()}}})
	return g, err
}
func (s *Service) UpdateGroupAuthorized(ctx context.Context, user, org, id string, in UpsertGroupInput) (Group, error) {
	g, err := s.GetGroupAuthorized(ctx, user, org, id, ReadStrong)
	if err != nil {
		return Group{}, err
	}
	g.Name, err = required("name", in.Name)
	if err != nil {
		return Group{}, err
	}
	g.Description = optionalText(in.Description)
	err = s.store.Transaction(ctx, []Statement{{SQL: "UPDATE groups SET name=?,description=?,updated_at=? WHERE id=? AND organization_id=?", Args: []any{g.Name, g.Description, nowString(), id, org}, ExpectedRowsAffected: requireOne()}})
	return g, err
}
func (s *Service) DeleteGroupAuthorized(ctx context.Context, user, org, id string) error {
	if _, err := s.GetGroupAuthorized(ctx, user, org, id, ReadStrong); err != nil {
		return err
	}
	rows, err := s.store.Query(ctx, ReadStrong, "SELECT id FROM role_bindings WHERE subject_type='group' AND subject_id=?", id)
	if err != nil {
		return err
	}
	if len(rows) > 0 {
		return serviceError(409, "group cannot be deleted while role bindings reference it")
	}
	return s.store.Transaction(ctx, []Statement{assertion("SELECT 1 WHERE NOT EXISTS(SELECT 1 FROM role_bindings WHERE subject_type='group' AND subject_id=?)", id), {SQL: "DELETE FROM group_memberships WHERE group_id=?", Args: []any{id}}, {SQL: "DELETE FROM groups WHERE id=? AND organization_id=?", Args: []any{id, org}, ExpectedRowsAffected: requireOne()}})
}
func (s *Service) ListGroupMembersAuthorized(ctx context.Context, user, org, id string, c ReadConsistency) ([]GroupMember, error) {
	if _, err := s.GetGroupAuthorized(ctx, user, org, id, c); err != nil {
		return nil, err
	}
	return serviceRows[GroupMember](ctx, s, c, "SELECT gm.group_id,gm.organization_membership_id,om.user_id FROM group_memberships gm JOIN organization_memberships om ON om.id=gm.organization_membership_id WHERE gm.group_id=? AND om.organization_id=? ORDER BY om.user_id", id, org)
}
func (s *Service) AddGroupMemberAuthorized(ctx context.Context, user, org, id, target string) (GroupMember, error) {
	if _, err := s.GetGroupAuthorized(ctx, user, org, id, ReadStrong); err != nil {
		return GroupMember{}, err
	}
	target, err := required("userId", target)
	if err != nil {
		return GroupMember{}, err
	}
	rows, err := s.store.Query(ctx, ReadStrong, "SELECT om.id FROM organization_memberships om JOIN users u ON u.id=om.user_id JOIN organizations o ON o.id=om.organization_id WHERE om.user_id=? AND om.organization_id=? AND om.status='active' AND u.active=1 AND o.inactivated_at IS NULL", target, org)
	if err != nil {
		return GroupMember{}, err
	}
	if len(rows) != 1 {
		return GroupMember{}, serviceError(403, fmt.Sprintf("forbidden: user %s is not an active member of organization %s", target, org))
	}
	membership := rowString(rows[0], "id")
	err = s.store.Transaction(ctx, []Statement{assertion("SELECT 1 FROM organization_memberships om JOIN users u ON u.id=om.user_id JOIN organizations o ON o.id=om.organization_id JOIN groups g ON g.organization_id=o.id WHERE om.id=? AND om.status='active' AND u.active=1 AND o.inactivated_at IS NULL AND g.id=?", membership, id), {SQL: "INSERT INTO group_memberships(group_id,organization_membership_id) VALUES(?,?) ON CONFLICT DO NOTHING", Args: []any{id, membership}}})
	return GroupMember{GroupID: id, OrganizationMembershipID: membership, UserID: target}, err
}
func (s *Service) RemoveGroupMemberAuthorized(ctx context.Context, user, org, id, target string) error {
	if _, err := s.GetGroupAuthorized(ctx, user, org, id, ReadStrong); err != nil {
		return err
	}
	_, err := s.store.Exec(ctx, "DELETE FROM group_memberships WHERE group_id=? AND organization_membership_id IN(SELECT id FROM organization_memberships WHERE organization_id=? AND user_id=?)", id, org, target)
	return err
}
func (s *Service) EnrollOrganizationMemberAuthorized(ctx context.Context, user, org, target string) (OrganizationMembership, error) {
	if err := s.requireOrganizationAction(ctx, user, org, org, "organization.member.manage", "manage organization members", ReadStrong); err != nil {
		return OrganizationMembership{}, err
	}
	target, err := required("userId", target)
	if err != nil {
		return OrganizationMembership{}, err
	}
	return s.store.EnrollActiveMember(ctx, org, target)
}
