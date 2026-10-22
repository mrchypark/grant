package grant

import (
	"context"
	"fmt"
	"strings"
)

func (s *Service) ListRolesAuthorized(ctx context.Context, user, org string, c ReadConsistency) ([]Role, error) {
	if err := s.requireOrganizationAction(ctx, user, org, org, "organization.permission.manage", "manage organization roles", c); err != nil {
		return nil, err
	}
	return serviceRows[Role](ctx, s, c, "SELECT * FROM roles WHERE organization_id IS NULL OR organization_id=? ORDER BY organization_id IS NOT NULL DESC,name,id", org)
}
func (s *Service) CreateRoleAuthorized(ctx context.Context, user, org string, in CreateRoleInput) (Role, error) {
	if err := s.requireOrganizationAction(ctx, user, org, org, "organization.permission.manage", "manage organization roles", ReadStrong); err != nil {
		return Role{}, err
	}
	id, err := required("id", in.ID)
	if err != nil {
		return Role{}, err
	}
	if strings.HasPrefix(id, "role-system-") {
		return Role{}, serviceError(400, "role-system- identifiers are reserved")
	}
	code, err := required("code", in.Code)
	if err != nil {
		return Role{}, err
	}
	name, err := required("name", in.Name)
	if err != nil {
		return Role{}, err
	}
	r := Role{ID: id, OrganizationID: &org, Code: code, Name: name, Description: optionalText(in.Description)}
	_, err = s.store.Exec(ctx, "INSERT INTO roles(id,organization_id,code,name,description,created_at,updated_at) VALUES(?,?,?,?,?,?,?)", id, org, code, name, r.Description, nowString(), nowString())
	return r, err
}
func (s *Service) ReplaceRoleActionsAuthorized(ctx context.Context, user, org, id string, actions []string) ([]Action, error) {
	if err := s.requireOrganizationAction(ctx, user, org, org, "organization.permission.manage", "manage organization roles", ReadStrong); err != nil {
		return nil, err
	}
	if len(actions) > 999 {
		return nil, serviceError(400, "actionIds must contain at most 999 items")
	}
	if _, err := serviceRow[Role](ctx, s, ReadStrong, "SELECT * FROM roles WHERE id=? AND organization_id=?", id, org); err != nil {
		return nil, err
	}
	dedup := make([]string, 0, len(actions))
	seen := map[string]bool{}
	for _, a := range actions {
		a, err := required("role action ids", a)
		if err != nil {
			return nil, err
		}
		if !seen[a] {
			seen[a] = true
			dedup = append(dedup, a)
		}
	}
	payload, err := jsonString(dedup)
	if err != nil {
		return nil, err
	}
	err = s.store.actionTransaction(ctx, []Statement{assertion("SELECT 1 FROM roles WHERE id=? AND organization_id=?", id, org), assertion("SELECT 1 WHERE NOT EXISTS(SELECT 1 FROM json_each(?) j LEFT JOIN actions a ON a.id=j.value WHERE a.id IS NULL)", payload), {SQL: "DELETE FROM role_actions WHERE role_id=?", Args: []any{id}}, {SQL: "INSERT INTO role_actions(role_id,action_id) SELECT ?,value FROM json_each(?)", Args: []any{id, payload}}})
	if err != nil {
		return nil, err
	}
	return serviceRows[Action](ctx, s, ReadStrong, "SELECT a.* FROM actions a JOIN role_actions ra ON ra.action_id=a.id WHERE ra.role_id=? ORDER BY a.code", id)
}
func (s *Service) ListRoleBindingsAuthorized(ctx context.Context, user, org string, c ReadConsistency) ([]RoleBinding, error) {
	if err := s.requireOrganizationAction(ctx, user, org, org, "organization.permission.manage", "manage organization role bindings", c); err != nil {
		return nil, err
	}
	return serviceRows[RoleBinding](ctx, s, c, "SELECT * FROM role_bindings WHERE subject_organization_id=? ORDER BY created_at DESC,id", org)
}
func (s *Service) CreateRoleBindingAuthorized(ctx context.Context, user, org string, in CreateRoleBindingInput) (RoleBinding, error) {
	if !enumValue(in.SubjectType, "organization", "group", "user") || !enumValue(in.State, "active", "disabled") {
		return RoleBinding{}, serviceError(400, "invalid role binding subject type or state")
	}
	if err := s.requireOrganizationAction(ctx, user, org, org, "organization.permission.manage", "manage organization role bindings", ReadStrong); err != nil {
		return RoleBinding{}, err
	}
	if in.SubjectOrganizationID != org {
		return RoleBinding{}, serviceError(403, fmt.Sprintf("forbidden: role binding subject organization %s does not match path organization %s", in.SubjectOrganizationID, org))
	}
	vals := []string{in.ID, in.SubjectID, in.ProviderID, in.ResourceType, in.ResourceID, in.RoleID}
	labels := []string{"id", "subjectId", "providerId", "resourceType", "resourceId", "roleId"}
	for i := range vals {
		v, err := required(labels[i], vals[i])
		if err != nil {
			return RoleBinding{}, err
		}
		vals[i] = v
	}
	b := RoleBinding{ID: vals[0], SubjectType: in.SubjectType, SubjectID: vals[1], SubjectOrganizationID: org, ProviderID: vals[2], ResourceType: vals[3], ResourceID: vals[4], RoleID: vals[5], AgreementID: optionalText(in.AgreementID), State: in.State, CreatedByUserID: user, CreatedAt: nowString(), UpdatedAt: nowString()}
	if err := validateRoleBinding(b); err != nil {
		return RoleBinding{}, err
	}
	if err := lifecycleError(s.store.Transaction(ctx, []Statement{lifecycleActorGuard(user, org, "organization.permission.manage"), roleBindingStatement(b)})); err != nil {
		return RoleBinding{}, err
	}
	return b, nil
}
func (s *Service) DeleteRoleBindingAuthorized(ctx context.Context, user, org, id string) error {
	if err := s.requireOrganizationAction(ctx, user, org, org, "organization.permission.manage", "manage organization role bindings", ReadStrong); err != nil {
		return err
	}
	if _, err := serviceRow[RoleBinding](ctx, s, ReadStrong, "SELECT * FROM role_bindings WHERE id=? AND subject_organization_id=?", id, org); err != nil {
		return err
	}
	return lifecycleError(s.store.Transaction(ctx, []Statement{
		lifecycleActorGuard(user, org, "organization.permission.manage"),
		assertion("SELECT 1 FROM role_bindings WHERE id=? AND subject_organization_id=?", id, org),
		{SQL: "DELETE FROM role_bindings WHERE id=? AND subject_organization_id=?", Args: []any{id, org}, ExpectedRowsAffected: requireOne()},
	}))
}
