package grant

import (
	"context"
	_ "embed"
	"encoding/json"
	"fmt"
	"strings"
	"time"
)

//go:embed store_authz.sql
var authorizationSQLTemplate string

//go:embed store_lookup_resources.sql
var lookupResourcesSQLTemplate string

var authorizationSQL = strings.Replace(authorizationSQLTemplate, "{agreement_bounds}", agreementBoundsSQL, 1)
var lookupResourcesSQL = strings.Replace(lookupResourcesSQLTemplate, "{agreement_bounds}", agreementBoundsSQL, 1)

type LookupResourcesInput struct {
	UserID         string
	OrganizationID string
	ProviderID     string
	ResourceType   string
	ActionID       string
	ApplicationID  string
	Limit          int
	Cursor         string
	Consistency    ReadConsistency
}

type LookupResourcesResult struct {
	ResourceIDs []string `json:"resource_ids"`
	NextCursor  string   `json:"next_cursor,omitempty"`
}

type UserLookupResourcesResult struct {
	ResourceIDs       []string `json:"resource_ids"`
	NextCursor        string   `json:"next_cursor,omitempty"`
	PEPPolicyRevision int64    `json:"pepPolicyRevision"`
}

func (s *Store) LookupResources(ctx context.Context, in LookupResourcesInput) (LookupResourcesResult, error) {
	limit := in.Limit
	if limit <= 0 {
		limit = 100
	}
	if limit > 1000 {
		return LookupResourcesResult{}, serviceError(400, "limit must be between 1 and 1000")
	}
	rows, err := s.Query(
		ctx,
		in.Consistency,
		lookupResourcesSQL,
		in.UserID,
		in.OrganizationID,
		in.ProviderID,
		in.ResourceType,
		in.ActionID,
		in.ApplicationID,
		time.Now().UTC().Format(agreementTimeLayout),
		in.Cursor,
		int64(limit+1),
		int64(limit+1),
		int64(limit+1),
		int64(limit+1),
	)
	if err != nil {
		return LookupResourcesResult{}, err
	}
	ids := make([]string, 0, min(len(rows), limit))
	for i, row := range rows {
		if i == limit {
			break
		}
		ids = append(ids, rowString(row, "resource_id"))
	}
	var next string
	if len(rows) > limit && len(ids) > 0 {
		next = ids[len(ids)-1]
	}
	return LookupResourcesResult{ResourceIDs: ids, NextCursor: next}, nil
}

func (s *Store) EvaluateUserActions(ctx context.Context, checks []UserActionCheck) ([]UserActionDecision, error) {
	return s.evaluateUserActions(ctx, checks, false)
}

const authzBatchChunkSize = 90

func (s *Store) evaluateUserActions(ctx context.Context, checks []UserActionCheck, inactive bool) ([]UserActionDecision, error) {
	if len(checks) == 0 {
		return []UserActionDecision{}, nil
	}
	out := make([]UserActionDecision, 0, len(checks))
	now := time.Now().UTC().Format(agreementTimeLayout)
	for start := 0; start < len(checks); {
		consistency := checks[start].Consistency
		end := start + 1
		for end < len(checks) && end-start < authzBatchChunkSize && checks[end].Consistency == consistency {
			end++
		}
		chunk := checks[start:end]
		placeholders := make([]string, len(chunk))
		args := make([]any, 0, len(chunk)*10)
		for i, c := range chunk {
			placeholders[i] = "(?, ?, ?, ?, ?, ?, ?, ?, ?, ?)"
			args = append(args, int64(i), c.UserID, c.OrganizationID, c.ProviderID, c.ResourceType, c.ResourceID, c.ActionID, c.ApplicationID, now, inactive)
		}
		q := strings.Replace(authorizationSQL, "{values}", strings.Join(placeholders, ", "), 1)
		rows, err := s.Query(ctx, consistency, q, args...)
		if err != nil {
			return nil, err
		}
		if len(rows) != len(chunk) {
			return nil, fmt.Errorf("authorization query returned %d decisions, want %d", len(rows), len(chunk))
		}
		for _, row := range rows {
			out = append(out, UserActionDecision{
				Allowed:            rowInt(row, "allowed") != 0,
				RBACAllowed:        rowInt(row, "rbac_allowed") != 0,
				TrustedApplication: rowInt(row, "trusted_application") != 0,
				ApplicationAllowed: rowInt(row, "application_allowed") != 0,
			})
		}
		start = end
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
	if err := validateRoleBinding(b); err != nil {
		return err
	}
	_, e := s.Execute(ctx, requestID(), roleBindingStatement(b))
	return e
}

func validateRoleBinding(b RoleBinding) error {
	if b.SubjectType != "user" && b.SubjectType != "group" && b.SubjectType != "organization" {
		return serviceError(400, "invalid binding subject")
	}
	if b.State != "active" && b.State != "disabled" {
		return serviceError(400, "invalid binding state")
	}
	if b.SubjectType == "organization" && b.SubjectID != b.SubjectOrganizationID {
		return serviceError(400, "organization subject mismatch")
	}
	cross := b.ProviderID == "grant" && b.ResourceType == "organization" && b.ResourceID != b.SubjectOrganizationID
	if cross && (b.SubjectType != "organization" || b.AgreementID == nil) {
		return serviceError(400, "cross-organization binding requires organization subject and agreement")
	}
	return nil
}

func roleBindingStatement(b RoleBinding) Statement {
	return Statement{SQL: `WITH ` + agreementBoundsSQL + ` INSERT INTO role_bindings(id,subject_type,subject_id,subject_organization_id,provider_id,resource_type,resource_id,role_id,agreement_id,state,created_by_user_id,created_at,updated_at)
 SELECT $1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13
 WHERE EXISTS(SELECT 1 FROM service_providers WHERE id=$5 AND active=1)
 AND EXISTS(SELECT 1 FROM organizations WHERE id=$4 AND inactivated_at IS NULL)
 AND EXISTS(SELECT 1 FROM roles WHERE id=$8 AND (organization_id IS NULL OR organization_id=$4))
 AND EXISTS(SELECT 1 FROM users WHERE id=$11)
 AND ($2<>'group' OR EXISTS(SELECT 1 FROM groups WHERE id=$3 AND organization_id=$4))
 AND ($2<>'user' OR EXISTS(SELECT 1 FROM organization_memberships WHERE user_id=$3 AND organization_id=$4))
 AND ($5<>'grant' OR $6<>'organization' OR EXISTS(SELECT 1 FROM organizations WHERE id=$7 AND inactivated_at IS NULL))
 AND ($9 IS NULL OR EXISTS(SELECT 1 FROM valid_agreements WHERE id=$9 AND (starts_at IS NULL OR starts_at<=$14) AND (ends_at IS NULL OR ends_at>=$14) AND ($5<>'grant' OR $6<>'organization' OR $7=$4 OR (grantor_organization_id=$7 AND grantee_organization_id=$4))))`, Args: []any{b.ID, b.SubjectType, b.SubjectID, b.SubjectOrganizationID, b.ProviderID, b.ResourceType, b.ResourceID, b.RoleID, b.AgreementID, b.State, b.CreatedByUserID, b.CreatedAt, b.UpdatedAt, time.Now().UTC().Format(agreementTimeLayout)}, ExpectedRowsAffected: requireOne()}
}

// Shared by authorization reads and replicated mutation guards. Rhiza forbids
// SQLite date functions in writes, so only exact fixed-width UTC bounds compare.
// Noncanonical legacy bounds deny until an authorized grantor normalizes them.
const agreementBoundsSQL = `valid_agreements AS (
 SELECT a.* FROM agreements a WHERE a.status='active' AND NOT EXISTS (
  SELECT 1 FROM (SELECT a.starts_at AS bound UNION ALL SELECT a.ends_at) bounds
  WHERE bound IS NOT NULL AND NOT (
   length(bound)=30 AND bound GLOB '[0-9][0-9][0-9][0-9]-[0-9][0-9]-[0-9][0-9]T[0-9][0-9]:[0-9][0-9]:[0-9][0-9].[0-9][0-9][0-9][0-9][0-9][0-9][0-9][0-9][0-9]Z'
   AND substr(bound,6,2) BETWEEN '01' AND '12'
   AND CAST(substr(bound,9,2) AS INTEGER) BETWEEN 1 AND
    CASE CAST(substr(bound,6,2) AS INTEGER)
     WHEN 2 THEN CASE WHEN CAST(substr(bound,1,4) AS INTEGER)%400=0 OR (CAST(substr(bound,1,4) AS INTEGER)%4=0 AND CAST(substr(bound,1,4) AS INTEGER)%100<>0) THEN 29 ELSE 28 END
     WHEN 4 THEN 30 WHEN 6 THEN 30 WHEN 9 THEN 30 WHEN 11 THEN 30 ELSE 31 END
   AND substr(bound,12,2) BETWEEN '00' AND '23'
   AND substr(bound,15,2) BETWEEN '00' AND '59'
   AND substr(bound,18,2) BETWEEN '00' AND '59'
  )
 )
)`
