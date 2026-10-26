package grant

import (
	"context"
	"encoding/json"
	"strings"
)

// SelfOrganizationInput never accepts an owner supplied by the caller.
type SelfOrganizationInput struct {
	ID   string `json:"id"`
	Name string `json:"name"`
}

func (v *SelfOrganizationInput) UnmarshalJSON(data []byte) error {
	type plain SelfOrganizationInput
	return decodeContractPatch(data, (*plain)(v), "id", "name")
}
func (s *Service) CreateOwnOrganization(ctx context.Context, token string, in SelfOrganizationInput) (Organization, error) {
	view, err := s.CurrentUser(ctx, token, "", ReadStrong)
	if err != nil {
		return Organization{}, err
	}
	id, err := required("id", in.ID)
	if err != nil {
		return Organization{}, err
	}
	return s.createOrganization(WithManagementActor(ctx, view.User.ID), CreateOrganizationInput{ID: id, Name: in.Name, OwnerUserID: view.User.ID}, managementOwnerBindingStatements(id, view.User.ID, view.User.ID))
}

func (s *Service) LinkGoauthyIdentityAdmin(ctx context.Context, in GoauthyIdentityLinkInput) error {
	if s.cfg.Auth.Goauthy == nil {
		return serviceError(503, "Goauthy is not configured")
	}
	issuer, err := canonicalGoauthyOrigin(in.Issuer)
	if err != nil || issuer != s.cfg.Auth.Goauthy.Issuer || in.Issuer != issuer {
		return serviceError(422, "Goauthy issuer does not match configured issuer")
	}
	return s.store.LinkGoauthyIdentityToExistingUser(ctx, in)
}

// This protected system role declares tenant-management actions for active
// owner memberships; the evaluator applies it only to the owner's organization.
func managementOwnerBindingStatements(org, user, actor string) []Statement {
	ids := []string{}
	for _, r := range managementRoutes {
		if !r.Platform && strings.HasPrefix(r.Path, "/api/v1/orgs/{organizationId}") {
			ids = append(ids, "grant-management."+r.ActionCode)
		}
	}
	j, _ := jsonString(ids)
	now := nowString()
	return []Statement{
		assertion("SELECT 1 WHERE NOT EXISTS(SELECT 1 FROM json_each(?) j WHERE NOT EXISTS(SELECT 1 FROM actions a JOIN service_providers p ON p.id=a.provider_id AND p.active=1 WHERE a.id=j.value AND a.provider_id='grant-management'))", j),
		{SQL: "INSERT INTO roles(id,organization_id,code,name,created_at,updated_at) VALUES('role-system-management-owner',NULL,'management-owner','Organization management owner',?,?) ON CONFLICT(id) DO NOTHING", Args: []any{now, now}},
		assertion("SELECT 1 FROM roles WHERE id='role-system-management-owner' AND organization_id IS NULL"),
		assertion("SELECT 1 WHERE NOT EXISTS(SELECT 1 FROM role_actions WHERE role_id='role-system-management-owner' AND action_id NOT IN(SELECT value FROM json_each(?)))", j),
		{SQL: "INSERT INTO role_actions(role_id,action_id) SELECT 'role-system-management-owner',value FROM json_each(?) WHERE 1 ON CONFLICT(role_id,action_id) DO NOTHING", Args: []any{j}},
	}
}

func managementOrganizationCapability(provider string, cap HttpCapability) bool {
	r, ok := LookupManagementRoute(cap.Method, cap.PathTemplate)
	return provider == "grant-management" && ok && !r.Platform && strings.HasPrefix(r.Path, "/api/v1/orgs/{organizationId}") && cap.ActionID != nil && *cap.ActionID == "grant-management."+r.ActionCode && cap.AccessMode == "authorized" && cap.Request.Authorization == "forward-user" && cap.Resource != nil && cap.Resource.ResourceType == "organization" && cap.Resource.ID.Kind == "pathSegment" && cap.Resource.ID.Name == "organizationId" && cap.RecordScope == nil && cap.Batch == nil && cap.ResponseAuthorization == nil
}

func managementOnboardingCapability(provider string, cap HttpCapability) bool {
	if provider != "grant-management" || cap.ActionID == nil || cap.Resource == nil || cap.AccessMode != "authorized" || cap.Request.Authorization != "forward-user" || cap.Resource.ResourceType != "service" || cap.Resource.ID.Kind != "constant" || cap.Resource.ID.Value != "grant" || cap.RecordScope != nil || cap.Batch != nil || cap.ResponseAuthorization != nil {
		return false
	}
	action, body := "", "none"
	switch cap.Method + " " + cap.PathTemplate {
	case "POST /api/v1/me/ensure":
		action = "user.materialize"
	case "GET /api/v1/me":
		action = "user.read"
	case "GET /api/v1/orgs":
		action = "organization.list"
	case "POST /api/v1/orgs":
		action, body = "organization.create", "json"
	default:
		return false
	}
	return *cap.ActionID == "grant-management."+action && cap.Request.Body == body
}

// This read-only resolver allows an external identity before its first local
// materialization. It is restricted to the deployment's trusted onboarding app.
func (s *Service) resolveOnboardingContext(ctx context.Context, token, action string) (applicationAuthorizationContext, error) {
	identity, err := s.resolveExternalIdentity(ctx, token, ReadStrong)
	if err != nil {
		return applicationAuthorizationContext{}, err
	}
	app, err := s.store.GetApplicationWithConsistency(ctx, s.cfg.Security.ManagementApplicationID, ReadStrong)
	if err != nil {
		return applicationAuthorizationContext{}, serviceError(403, "onboarding application unavailable")
	}
	if !app.Active || app.ProvisioningStatus != "ready" || !app.FirstParty || app.ConsentPolicy != "auto_approve" {
		return applicationAuthorizationContext{}, serviceError(403, "trusted onboarding application required")
	}
	if identity.Provider == "pocketbase" {
		if s.cfg.Auth.PocketBase == nil || s.cfg.Auth.PocketBase.ApplicationID != app.ID || identity.ClientID != app.ID {
			return applicationAuthorizationContext{}, serviceError(403, "onboarding application mismatch")
		}
	} else if identity.Provider == "goauthy" {
		if s.cfg.Auth.Goauthy == nil || s.cfg.Auth.Goauthy.ApplicationID != app.ID || identity.ClientID != app.ID {
			return applicationAuthorizationContext{}, serviceError(403, "onboarding application mismatch")
		}
	} else if identity.ClientID == "" || identity.ClientID != ptrValue(app.RauthyClientID) {
		return applicationAuthorizationContext{}, serviceError(403, "onboarding application mismatch")
	}
	rows, err := s.store.Query(ctx, ReadStrong, `SELECT 1 FROM application_actions aa JOIN actions a ON a.id=aa.action_id AND a.provider_id='grant-management' AND a.grantable_to_applications=1 JOIN service_providers p ON p.id=a.provider_id AND p.active=1 JOIN organizations o ON o.id=? AND o.inactivated_at IS NULL WHERE aa.application_id=? AND aa.action_id=?`, app.OrganizationID, app.ID, action)
	if err != nil {
		return applicationAuthorizationContext{}, err
	}
	if len(rows) != 1 {
		return applicationAuthorizationContext{}, serviceError(403, "onboarding action not admitted")
	}
	user, err := s.store.resolveScopedIdentity(ctx, ExternalIdentityMaterialization{Provider: identity.Provider, Issuer: identity.Issuer, CollectionID: identity.CollectionID, ExternalUserID: identity.ExternalUserID}, ReadStrong)
	if err != nil {
		return applicationAuthorizationContext{}, err
	}
	if user != nil && !user.Active {
		return applicationAuthorizationContext{}, serviceError(403, "local user is inactive")
	}
	subject := identity.ExternalUserID
	if identity.Provider == "pocketbase" {
		subject = pocketBaseSubject(identity.Issuer, identity.CollectionID, identity.ExternalUserID)
	} else if identity.Provider == "goauthy" {
		subject = goauthySubject(identity.Issuer, identity.ExternalUserID)
	}
	// Namespace the verified external identity separately from local user IDs.
	key, _ := json.Marshal([]string{identity.Provider, identity.Issuer, identity.CollectionID, identity.ExternalUserID})
	audience := identity.ClientID
	if identity.Provider == "goauthy" {
		audience = app.ID
	}
	return applicationAuthorizationContext{UserID: "onboarding:" + sha256Text(string(key)), OrganizationID: app.OrganizationID, ApplicationID: app.ID, ExternalSubject: subject, ProviderAudience: audience}, nil
}
