package grant

import (
	"context"
	"fmt"
)

func (s *Store) GetApplicationWithConsistency(ctx context.Context, id string, c ReadConsistency) (Application, error) {
	r, e := s.Query(ctx, c, "SELECT * FROM applications WHERE id=?", id)
	if e != nil {
		return Application{}, e
	}
	if len(r) == 0 {
		return Application{}, fmt.Errorf("application not found: %s", id)
	}
	return decodeRow[Application](r[0])
}
func (s *Store) FindApplicationForOrganizationWithConsistency(ctx context.Context, id, org string, c ReadConsistency) (*Application, error) {
	r, e := s.Query(ctx, c, "SELECT * FROM applications WHERE id=? AND organization_id=?", id, org)
	if e != nil {
		return nil, e
	}
	if len(r) == 0 {
		return nil, nil
	}
	a, e := decodeRow[Application](r[0])
	return &a, e
}
func (s *Store) ListApplicationsForOrganizationWithConsistency(ctx context.Context, org string, c ReadConsistency) ([]Application, error) {
	r, e := s.Query(ctx, c, "SELECT * FROM applications WHERE organization_id=? ORDER BY name,id", org)
	if e != nil {
		return nil, e
	}
	return decodeRows[Application](r)
}
func (s *Store) InsertApplicationWithRedirectURI(ctx context.Context, a Application, uri *string) error {
	now := nowString()
	ss := []Statement{{SQL: "INSERT INTO applications(id,organization_id,slug,name,description,application_type,app_type,first_party,consent_policy,active,provisioning_status,rauthy_client_id,created_by_user_id,created_at,updated_at) SELECT ?,?,?,?,?,?,?,?,?,?,?,?,?,?,? FROM organizations o JOIN users u ON u.id=? JOIN organization_memberships m ON m.organization_id=o.id AND m.user_id=u.id WHERE o.id=? AND o.inactivated_at IS NULL AND u.active=1 AND m.status='active'", Args: []any{a.ID, a.OrganizationID, a.Slug, a.Name, a.Description, a.ApplicationType, a.AppType, a.FirstParty, a.ConsentPolicy, a.Active, a.ProvisioningStatus, a.RauthyClientID, a.CreatedByUserID, now, now, a.CreatedByUserID, a.OrganizationID}, ExpectedRowsAffected: requireOne()}}
	if uri != nil {
		ss = append(ss, Statement{SQL: "INSERT INTO application_redirect_uris(application_id,redirect_uri,created_at) VALUES(?,?,?)", Args: []any{a.ID, *uri, now}})
	}
	return s.Transaction(ctx, ss)
}
func (s *Store) ListApplicationActionsWithConsistency(ctx context.Context, id string, c ReadConsistency) ([]Action, error) {
	r, e := s.Query(ctx, c, "SELECT a.* FROM actions a JOIN application_actions aa ON aa.action_id=a.id WHERE aa.application_id=? ORDER BY a.id", id)
	if e != nil {
		return nil, e
	}
	return decodeRows[Action](r)
}

type applicationActionTarget struct {
	ID             string `json:"id"`
	OrganizationID string `json:"organizationId"`
	CreatedAt      string `json:"createdAt"`
}

// ReplaceApplicationActions is the internal store entry point. HTTP callers use
// the service-authorized path, which also supplies a transactional actor guard.
func (s *Store) ReplaceApplicationActions(ctx context.Context, id string, ids []string) ([]Action, error) {
	rows, err := s.Query(ctx, ReadStrong, "SELECT id,organization_id,created_at FROM applications WHERE id=?", id)
	if err != nil {
		return nil, err
	}
	if len(rows) != 1 {
		return nil, serviceError(404, "application not found")
	}
	target, err := decodeRow[applicationActionTarget](rows[0])
	if err != nil {
		return nil, err
	}
	return s.replaceApplicationActions(ctx, target, "", ids)
}

func (s *Store) replaceApplicationActions(ctx context.Context, target applicationActionTarget, user string, ids []string) ([]Action, error) {
	id := target.ID
	if len(ids) > 999 {
		return nil, fmt.Errorf("actionIds must contain at most 999 items")
	}
	unique := map[string]bool{}
	v := make([]string, 0, len(ids))
	for _, id := range ids {
		if !unique[id] {
			unique[id] = true
			v = append(v, id)
		}
	}
	j, e := jsonString(v)
	if e != nil {
		return nil, e
	}
	n := int64(len(v))
	ss := []Statement{
		assertion("SELECT 1 FROM applications WHERE id=? AND organization_id=? AND created_at=? AND NOT(first_party=1 AND consent_policy='auto_approve') AND (security_mutation_token IS NULL OR security_mutation_expires_at<=?)", id, target.OrganizationID, target.CreatedAt, nowString()),
		{SQL: "DELETE FROM application_actions WHERE application_id=?", Args: []any{id}},
		{SQL: "INSERT INTO application_actions(application_id,action_id) SELECT ?,j.value FROM json_each(?) j JOIN actions a ON a.id=j.value JOIN service_providers p ON p.id=a.provider_id AND p.active=1 WHERE a.grantable_to_applications=1", Args: []any{id, j}, ExpectedRowsAffected: &n},
		{SQL: "DELETE FROM application_grant_actions WHERE application_grant_id IN(SELECT id FROM application_grants WHERE application_id=?) AND action_id NOT IN(SELECT action_id FROM application_actions WHERE application_id=?)", Args: []any{id, id}},
	}
	if user != "" {
		ss = append([]Statement{lifecycleActorGuard(user, target.OrganizationID, "organization.application.manage")}, ss...)
	}
	if e = s.actionTransaction(ctx, ss); e != nil {
		return nil, e
	}
	return s.ListApplicationActionsWithConsistency(ctx, id, ReadStrong)
}
func (s *Store) ListApplicationRedirectURIsWithConsistency(ctx context.Context, id string, c ReadConsistency) ([]ApplicationRedirectUri, error) {
	r, e := s.Query(ctx, c, "SELECT * FROM application_redirect_uris WHERE application_id=? ORDER BY redirect_uri", id)
	if e != nil {
		return nil, e
	}
	return decodeRows[ApplicationRedirectUri](r)
}
func (s *Store) MarkApplicationProvisioningReady(ctx context.Context, id, org string) (Application, error) {
	_, e := s.Execute(ctx, requestID(), Statement{SQL: "UPDATE applications SET provisioning_status='ready',updated_at=? WHERE id=? AND organization_id=? AND active=1", Args: []any{nowString(), id, org}, ExpectedRowsAffected: requireOne()})
	if e != nil {
		return Application{}, e
	}
	return s.GetApplicationWithConsistency(ctx, id, ReadStrong)
}
func (s *Store) SetApplicationTrust(ctx context.Context, id, org string, trusted bool) (Application, error) {
	now := nowString()
	policy := "explicit"
	if trusted {
		policy = "auto_approve"
	}
	ss := []Statement{{SQL: "UPDATE applications SET first_party=?,consent_policy=?,updated_at=? WHERE id=? AND organization_id=? AND (security_mutation_token IS NULL OR security_mutation_expires_at<=?)", Args: []any{trusted, policy, now, id, org, now}, ExpectedRowsAffected: requireOne()}}
	if trusted {
		ss = append(ss, Statement{SQL: "UPDATE application_grants SET status='revoked',revoked_at=?,updated_at=? WHERE application_id=? AND status='active'", Args: []any{now, now, id}})
	}
	if e := s.Transaction(ctx, ss); e != nil {
		return Application{}, e
	}
	return s.GetApplicationWithConsistency(ctx, id, ReadStrong)
}

// BindApplicationIdentity commits only the exact application manifest verified
// by the IdP adapter. The legacy column name is retained for existing databases.
func (s *Store) BindApplicationIdentity(ctx context.Context, a Application, clientID string, uri *string) (Application, error) {
	now := nowString()
	uriCount := 0
	if uri != nil {
		uriCount = 1
	}
	ss := []Statement{
		assertion("SELECT 1 WHERE NOT EXISTS(SELECT 1 FROM applications WHERE rauthy_client_id=? AND id<>?)", clientID, a.ID),
		assertion("SELECT 1 WHERE (SELECT COUNT(*) FROM application_redirect_uris WHERE application_id=?)=? AND NOT EXISTS(SELECT 1 FROM application_redirect_uris WHERE application_id=? AND redirect_uri<>?)", a.ID, uriCount, a.ID, ptrValue(uri)),
		{SQL: "UPDATE applications SET rauthy_client_id=?,provisioning_status='ready',updated_at=? WHERE id=? AND organization_id=? AND active=1 AND name=? AND application_type=? AND app_type=? AND provisioning_status=? AND rauthy_client_id IS ? AND (security_mutation_token IS NULL OR security_mutation_expires_at<=?) AND EXISTS(SELECT 1 FROM organizations WHERE id=? AND inactivated_at IS NULL)", Args: []any{clientID, now, a.ID, a.OrganizationID, a.Name, a.ApplicationType, a.AppType, a.ProvisioningStatus, a.RauthyClientID, now, a.OrganizationID}, ExpectedRowsAffected: requireOne()},
	}
	if err := s.Transaction(ctx, ss); err != nil {
		return Application{}, err
	}
	return s.GetApplicationWithConsistency(ctx, a.ID, ReadStrong)
}
