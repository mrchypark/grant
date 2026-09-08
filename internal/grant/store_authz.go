package grant

import (
	"context"
	_ "embed"
	"encoding/json"
	"fmt"
	"strings"
)

//go:embed store_authz.sql
var authorizationSQL string

func (s *Store) EvaluateUserActions(ctx context.Context, checks []UserActionCheck) ([]UserActionDecision, error) {
	return s.evaluateUserActions(ctx, checks, false)
}
func (s *Store) evaluateUserActions(ctx context.Context, checks []UserActionCheck, inactive bool) ([]UserActionDecision, error) {
	out := make([]UserActionDecision, 0, len(checks))
	for _, c := range checks {
		q := strings.Replace(authorizationSQL, "{values}", "(?, ?, ?, ?, ?, ?, ?, ?, ?, ?)", 1)
		rows, err := s.Query(ctx, c.Consistency, q, int64(0), c.UserID, c.OrganizationID, c.ProviderID, c.ResourceType, c.ResourceID, c.ActionID, c.ApplicationID, nowString(), inactive)
		if err != nil {
			return nil, err
		}
		if len(rows) != 1 {
			return nil, fmt.Errorf("authorization query did not return exactly one decision")
		}
		out = append(out, UserActionDecision{Allowed: rowInt(rows[0], "allowed") != 0, RBACAllowed: rowInt(rows[0], "rbac_allowed") != 0, TrustedApplication: rowInt(rows[0], "trusted_application") != 0})
	}
	return out, nil
}
func decodeRow[T any](row map[string]any) (T, error) {
	var out T
	normalized := make(map[string]any, len(row))
	for k, v := range row {
		parts := strings.Split(k, "_")
		for i := 1; i < len(parts); i++ {
			if parts[i] != "" {
				parts[i] = strings.ToUpper(parts[i][:1]) + parts[i][1:]
			}
		}
		if k == "active" || k == "first_party" || k == "visibility_granting" || k == "grantable_to_applications" {
			v = rowInt(row, k) != 0
		}
		normalized[strings.Join(parts, "")] = v
	}
	b, e := json.Marshal(normalized)
	if e != nil {
		return out, e
	}
	e = json.Unmarshal(b, &out)
	return out, e
}
func decodeRows[T any](rows []map[string]any) ([]T, error) {
	out := make([]T, 0, len(rows))
	for _, r := range rows {
		v, e := decodeRow[T](r)
		if e != nil {
			return nil, e
		}
		out = append(out, v)
	}
	return out, nil
}
func (s *Store) InsertRoleBinding(ctx context.Context, b RoleBinding) error {
	if b.SubjectType != "user" && b.SubjectType != "group" && b.SubjectType != "organization" {
		return fmt.Errorf("invalid binding subject")
	}
	if b.State != "active" && b.State != "disabled" {
		return fmt.Errorf("invalid binding state")
	}
	if b.SubjectType == "organization" && b.SubjectID != b.SubjectOrganizationID {
		return fmt.Errorf("organization subject mismatch")
	}
	cross := b.ProviderID == "grant" && b.ResourceType == "organization" && b.ResourceID != b.SubjectOrganizationID
	if cross && (b.SubjectType != "organization" || b.AgreementID == nil) {
		return fmt.Errorf("cross-organization binding requires organization subject and agreement")
	}
	_, e := s.Execute(ctx, requestID(), Statement{SQL: `INSERT INTO role_bindings(id,subject_type,subject_id,subject_organization_id,provider_id,resource_type,resource_id,role_id,agreement_id,state,created_by_user_id,created_at,updated_at)
 SELECT $1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13
 WHERE EXISTS(SELECT 1 FROM service_providers WHERE id=$5 AND active=1)
 AND EXISTS(SELECT 1 FROM organizations WHERE id=$4 AND inactivated_at IS NULL)
 AND EXISTS(SELECT 1 FROM roles WHERE id=$8 AND (organization_id IS NULL OR organization_id=$4))
 AND EXISTS(SELECT 1 FROM users WHERE id=$11)
 AND ($2<>'group' OR EXISTS(SELECT 1 FROM groups WHERE id=$3 AND organization_id=$4))
 AND ($2<>'user' OR EXISTS(SELECT 1 FROM organization_memberships WHERE user_id=$3 AND organization_id=$4))
 AND ($5<>'grant' OR $6<>'organization' OR EXISTS(SELECT 1 FROM organizations WHERE id=$7 AND inactivated_at IS NULL))
 AND ($9 IS NULL OR EXISTS(SELECT 1 FROM agreements WHERE id=$9 AND status='active' AND (starts_at IS NULL OR starts_at<=$14) AND (ends_at IS NULL OR ends_at>=$14) AND ($5<>'grant' OR $6<>'organization' OR $7=$4 OR (grantor_organization_id=$7 AND grantee_organization_id=$4))))`, Args: []any{b.ID, b.SubjectType, b.SubjectID, b.SubjectOrganizationID, b.ProviderID, b.ResourceType, b.ResourceID, b.RoleID, b.AgreementID, b.State, b.CreatedByUserID, b.CreatedAt, b.UpdatedAt, nowString()}, ExpectedRowsAffected: requireOne()})
	return e
}
