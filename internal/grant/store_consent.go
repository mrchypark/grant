package grant

import (
	"context"
	"fmt"
)

func (s *Store) FindActiveApplicationConsentWithConsistency(ctx context.Context, user, app, org string, c ReadConsistency) (*ApplicationGrant, error) {
	r, e := s.Query(ctx, c, "SELECT * FROM application_grants WHERE user_id=? AND application_id=? AND organization_id=? AND status='active'", user, app, org)
	if e != nil {
		return nil, e
	}
	if len(r) == 0 {
		return nil, nil
	}
	g, e := decodeRow[ApplicationGrant](r[0])
	return &g, e
}
func (s *Store) ReplaceActiveApplicationConsent(ctx context.Context, g ApplicationGrant, actions []string) (ApplicationGrant, error) {
	if len(actions) > 999 {
		return ApplicationGrant{}, fmt.Errorf("actionIds must contain at most 999 items")
	}
	set := map[string]bool{}
	ids := make([]string, 0, len(actions))
	for _, id := range actions {
		if !set[id] {
			set[id] = true
			ids = append(ids, id)
		}
	}
	j, e := jsonString(ids)
	if e != nil {
		return ApplicationGrant{}, e
	}
	stmts := []Statement{{SQL: "INSERT INTO application_grants(id,user_id,application_id,organization_id,status,consented_at,revoked_at,revoked_by_user_id,created_at,updated_at) SELECT ?,?,?,?,'active',?,NULL,NULL,?,? FROM applications a JOIN organization_memberships m ON m.organization_id=a.organization_id AND m.user_id=? AND m.status='active' JOIN users u ON u.id=m.user_id AND u.active=1 WHERE a.id=? AND a.organization_id=? AND a.active=1 AND a.first_party=0 AND a.consent_policy='explicit' ON CONFLICT(user_id,application_id,organization_id) WHERE status='active' DO UPDATE SET consented_at=excluded.consented_at,revoked_at=NULL,revoked_by_user_id=NULL,updated_at=excluded.updated_at", Args: []any{g.ID, g.UserID, g.ApplicationID, g.OrganizationID, g.ConsentedAt, g.CreatedAt, g.UpdatedAt, g.UserID, g.ApplicationID, g.OrganizationID}, ExpectedRowsAffected: requireOne()},
		{SQL: "DELETE FROM application_grant_actions WHERE application_grant_id=(SELECT id FROM application_grants WHERE user_id=? AND application_id=? AND organization_id=? AND status='active')", Args: []any{g.UserID, g.ApplicationID, g.OrganizationID}},
		{SQL: "INSERT INTO application_grant_actions(application_grant_id,action_id) SELECT g.id,j.value FROM json_each(?) j JOIN application_grants g ON g.user_id=? AND g.application_id=? AND g.organization_id=? AND g.status='active' JOIN application_actions a ON a.application_id=g.application_id AND a.action_id=j.value", Args: []any{j, g.UserID, g.ApplicationID, g.OrganizationID}, ExpectedRowsAffected: func() *int64 { n := int64(len(ids)); return &n }()},
	}
	if e = s.actionTransaction(ctx, stmts); e != nil {
		return ApplicationGrant{}, e
	}
	r, e := s.FindActiveApplicationConsentWithConsistency(ctx, g.UserID, g.ApplicationID, g.OrganizationID, ReadStrong)
	if e != nil {
		return ApplicationGrant{}, e
	}
	if r == nil {
		return ApplicationGrant{}, fmt.Errorf("consent missing after transaction")
	}
	return *r, nil
}
func (s *Store) RevokeActiveApplicationConsent(ctx context.Context, user, app, org string) error {
	_, e := s.Exec(ctx, "UPDATE application_grants SET status='revoked',revoked_at=?,revoked_by_user_id=?,updated_at=? WHERE user_id=? AND application_id=? AND organization_id=? AND status='active'", nowString(), user, nowString(), user, app, org)
	return e
}
