package grant

import (
	"context"
	"unicode/utf8"
)

// OrglessUserBinding is an operator grant, independent of tenant memberships.
type OrglessUserBinding struct {
	SubjectID    string `json:"subjectId"`
	ProviderID   string `json:"providerId"`
	ResourceType string `json:"resourceType"`
	ResourceID   string `json:"resourceId"`
	ActionID     string `json:"actionId"`
	State        string `json:"state"`
}

// The HTTP caller must authenticate with the master key, never tenant authority.
func (s *Service) SetOrglessUserBindingAdmin(ctx context.Context, id string, in OrglessUserBinding) (RoleBinding, error) {
	for _, value := range []string{id, in.SubjectID, in.ProviderID, in.ResourceType, in.ActionID} {
		if !capabilityToken(value) {
			return RoleBinding{}, serviceError(422, "invalid orgless binding identity")
		}
	}
	if !utf8.ValidString(in.ResourceID) || strictToken(in.ResourceID, 512) != nil {
		return RoleBinding{}, serviceError(422, "resourceId must be a bounded exact resource value")
	}
	if in.State != "active" && in.State != "disabled" {
		return RoleBinding{}, serviceError(422, "invalid orgless binding state")
	}
	role := "role-system-orgless-" + sha256Text(in.ActionID)[7:]
	now := nowString()
	// Revocation must remain available after any admission dependency disappears.
	if in.State == "disabled" {
		err := s.store.Transaction(ctx, []Statement{{SQL: `UPDATE role_bindings SET state='disabled',updated_at=?
 WHERE id=? AND subject_type='user' AND subject_organization_id='' AND agreement_id IS NULL
 AND subject_id=? AND provider_id=? AND resource_type=? AND resource_id=? AND role_id=?`, Args: []any{now, id, in.SubjectID, in.ProviderID, in.ResourceType, in.ResourceID, role}, ExpectedRowsAffected: requireOne()}})
		if err != nil {
			return RoleBinding{}, lifecycleError(err)
		}
		return serviceRow[RoleBinding](ctx, s, ReadStrong, "SELECT * FROM role_bindings WHERE id=?", id)
	}
	err := s.store.Transaction(ctx, []Statement{
		assertion(`SELECT 1 FROM users u,actions a,service_providers p
 WHERE u.id=? AND u.active=1 AND a.id=? AND a.provider_id=p.id AND a.grantable_to_applications=1 AND p.id=? AND p.active=1
 AND NOT EXISTS(SELECT 1 FROM role_bindings WHERE id=? AND (subject_type<>'user' OR subject_organization_id<>'' OR agreement_id IS NOT NULL))`, in.SubjectID, in.ActionID, in.ProviderID, id),
		{SQL: "INSERT INTO roles(id,organization_id,code,name,created_at,updated_at) VALUES(?,NULL,?,'Operator orgless action grant',?,?) ON CONFLICT(id) DO NOTHING", Args: []any{role, role, now, now}},
		assertion("SELECT 1 FROM roles WHERE id=? AND organization_id IS NULL AND NOT EXISTS(SELECT 1 FROM role_actions WHERE role_id=? AND action_id<>?)", role, role, in.ActionID),
		{SQL: "INSERT INTO role_actions(role_id,action_id) VALUES(?,?) ON CONFLICT DO NOTHING", Args: []any{role, in.ActionID}},
		{SQL: `INSERT INTO role_bindings(id,subject_type,subject_id,subject_organization_id,provider_id,resource_type,resource_id,role_id,agreement_id,state,created_by_user_id,created_at,updated_at)
 VALUES(?,'user',?,'',?,?,?,?,NULL,?,?,?,?) ON CONFLICT(id) DO UPDATE SET subject_id=excluded.subject_id,provider_id=excluded.provider_id,resource_type=excluded.resource_type,resource_id=excluded.resource_id,role_id=excluded.role_id,state=excluded.state,updated_at=excluded.updated_at`, Args: []any{id, in.SubjectID, in.ProviderID, in.ResourceType, in.ResourceID, role, in.State, ManagementActorFromContext(ctx), now, now}},
	})
	if err != nil {
		return RoleBinding{}, lifecycleError(err)
	}
	return serviceRow[RoleBinding](ctx, s, ReadStrong, "SELECT * FROM role_bindings WHERE id=?", id)
}

// Organization ownership admits an application; it never supplies request context.
func (s *Service) resolveOrglessApplicationContext(ctx context.Context, token string) (applicationAuthorizationContext, error) {
	identity, err := s.resolveExternalIdentity(ctx, token, ReadStrong)
	if err != nil {
		return applicationAuthorizationContext{}, err
	}
	u, err := s.localIdentity(ctx, identity, ReadStrong)
	if err != nil {
		return applicationAuthorizationContext{}, err
	}
	if !u.Active {
		return applicationAuthorizationContext{}, serviceError(403, "local user is inactive")
	}
	var appID string
	switch s.AuthProviderMode() {
	case "pocketbase":
		appID = s.cfg.Auth.PocketBase.ApplicationID
	case "goauthy":
		appID = s.cfg.Auth.Goauthy.ApplicationID
	default:
		apps, e := serviceRows[Application](ctx, s, ReadStrong, "SELECT * FROM applications WHERE rauthy_client_id=? AND active=1 AND provisioning_status='ready'", identity.ClientID)
		if e != nil {
			return applicationAuthorizationContext{}, e
		}
		if len(apps) != 1 {
			return applicationAuthorizationContext{}, serviceError(403, "token application is not uniquely active")
		}
		appID = apps[0].ID
	}
	app, err := s.store.GetApplicationWithConsistency(ctx, appID, ReadStrong)
	if err != nil || !app.Active || app.ProvisioningStatus != "ready" {
		return applicationAuthorizationContext{}, serviceError(403, "application is not ready")
	}
	orgs, err := s.store.Query(ctx, ReadStrong, "SELECT 1 FROM organizations WHERE id=? AND inactivated_at IS NULL", app.OrganizationID)
	if err != nil {
		return applicationAuthorizationContext{}, err
	}
	if len(orgs) != 1 {
		return applicationAuthorizationContext{}, serviceError(403, "application owner organization is inactive")
	}
	subject, audience := identity.ExternalUserID, identity.ClientID
	if identity.Provider == "pocketbase" {
		subject = pocketBaseSubject(identity.Issuer, identity.CollectionID, identity.ExternalUserID)
	}
	if identity.Provider == "goauthy" {
		subject, audience = goauthySubject(identity.Issuer, identity.ExternalUserID), app.ID
	}
	return applicationAuthorizationContext{UserID: u.ID, ApplicationID: app.ID, ExternalSubject: subject, ProviderAudience: audience}, nil
}

// Empty subject_organization_id is an explicit operator-owned orgless grant.
// It cannot inherit tenant membership, hierarchy, agreement, group, or wildcard rights.
func (s *Service) orglessActionAllowed(ctx context.Context, c applicationAuthorizationContext, provider, action, resourceType, resourceID string) (bool, error) {
	rows, err := s.store.Query(ctx, ReadStrong, `SELECT 1
 FROM role_bindings b
 JOIN roles r ON r.id=b.role_id AND r.organization_id IS NULL
 JOIN role_actions ra ON ra.role_id=r.id AND ra.action_id=?
 JOIN actions a ON a.id=ra.action_id AND a.provider_id=b.provider_id AND a.grantable_to_applications=1
 JOIN service_providers p ON p.id=a.provider_id AND p.active=1
 JOIN application_actions aa ON aa.action_id=a.id AND aa.application_id=?
 JOIN applications app ON app.id=aa.application_id AND app.active=1 AND app.provisioning_status='ready'
 JOIN organizations o ON o.id=app.organization_id AND o.inactivated_at IS NULL
 JOIN users u ON u.id=b.subject_id AND u.active=1
 WHERE b.subject_type='user' AND b.subject_id=? AND b.subject_organization_id=''
 AND b.provider_id=? AND b.resource_type=? AND b.resource_id=?
 AND b.agreement_id IS NULL AND b.state='active' LIMIT 1`, action, c.ApplicationID, c.UserID, provider, resourceType, resourceID)
	return len(rows) == 1, err
}
