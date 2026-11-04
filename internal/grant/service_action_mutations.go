package grant

import (
	"context"
	"crypto/sha256"
	"encoding/hex"
	"encoding/json"
	"regexp"
	"strings"
)

var customActionCode = regexp.MustCompile(`^custom\.org-[0-9a-f]{64}\.[a-z0-9](?:[a-z0-9-]*[a-z0-9])?\.[a-z0-9](?:[a-z0-9-]*[a-z0-9])?\.[a-z][a-z0-9-]*$`)

func operableCustomNamespace(org string) string {
	return organizationCustomNamespace(org)
}

// The provider column and provider-prefixed ID scope this reserved code namespace.
func organizationCustomNamespace(org string) string {
	digest := sha256.Sum256([]byte(org))
	return "custom.org-" + hex.EncodeToString(digest[:]) + "."
}

type CustomActionInput struct {
	Code        string  `json:"code"`
	Name        string  `json:"name"`
	Description *string `json:"description"`
	RiskLevel   string  `json:"riskLevel"`
}

type CustomActionPatch struct {
	Name                    *string   `json:"name"`
	Description             PatchText `json:"description"`
	RiskLevel               *string   `json:"riskLevel"`
	GrantableToApplications *bool     `json:"grantableToApplications"`
}

// Owners may also discover their own custom actions that are not yet eligible.
func (s *Service) ProviderActionsSnapshotAuthorized(ctx context.Context, user, org, provider string) (ActionSnapshot, error) {
	if s.isActiveOrganizationOwner(ctx, user, org) {
		err := s.requireOrganizationAction(ctx, user, org, org, "organization.permission.manage", "manage custom actions", ReadStrong)
		if err == nil {
			return s.ownerCustomActionsSnapshot(ctx, org, provider)
		}
		if ErrorStatus(err) != 403 {
			return ActionSnapshot{}, err
		}
	}
	if _, err := s.ListProviderActionsAuthorized(ctx, user, org, provider); err != nil {
		return ActionSnapshot{}, err
	}
	return s.store.actionSnapshot(ctx, actionArraySQL("a.provider_id=? AND a.grantable_to_applications=1 AND "+applicationActionNamespaceSQL), provider, organizationCustomNamespace(org)+"*")
}

func (s *Service) ownerCustomActionsSnapshot(ctx context.Context, org, provider string) (ActionSnapshot, error) {
	if _, err := serviceRow[ServiceProvider](ctx, s, ReadStrong, "SELECT * FROM service_providers WHERE id=? AND active=1", provider); err != nil {
		return ActionSnapshot{}, err
	}
	return s.store.actionSnapshot(ctx, actionArraySQL("a.provider_id=? AND (a.grantable_to_applications=1 OR a.code GLOB ?) AND "+applicationActionNamespaceSQL), provider, organizationCustomNamespace(org)+"*", organizationCustomNamespace(org)+"*")
}

func (s *Service) PatchOperableCustomActionConditional(ctx context.Context, user, org, id string, in CustomActionPatch, match string) (Action, error) {
	return s.PatchCustomActionConditional(ctx, user, org, "operable", id, in, match)
}

func (s *Service) PatchCustomActionConditional(ctx context.Context, user, org, provider, id string, in CustomActionPatch, match string) (Action, error) {
	if in.GrantableToApplications != nil {
		if _, err := s.ownerActionMutationGuard(ctx, user, org, provider); err != nil {
			return Action{}, err
		}
		snapshot, err := s.ownerCustomActionsSnapshot(ctx, org, provider)
		if err != nil {
			return Action{}, err
		}
		ctx, err = checkedActionContext(ctx, snapshot, match, "")
		if err != nil {
			return Action{}, err
		}
	}
	return s.PatchCustomActionAuthorized(ctx, user, org, provider, id, in)
}

func (s *Service) ownerActionMutationGuard(ctx context.Context, user, org, provider string) ([]Statement, error) {
	if err := s.requireOrganizationAction(ctx, user, org, org, "organization.permission.manage", "manage custom actions", ReadStrong); err != nil {
		return nil, err
	}
	if !s.isActiveOrganizationOwner(ctx, user, org) {
		return nil, serviceError(403, "only an active organization owner can manage custom actions")
	}
	if _, err := serviceRow[ServiceProvider](ctx, s, ReadStrong, "SELECT * FROM service_providers WHERE id=? AND active=1", provider); err != nil {
		return nil, err
	}
	return []Statement{
		lifecycleActorGuard(user, org, "organization.permission.manage"),
		assertion("SELECT 1 FROM organization_memberships m JOIN users u ON u.id=m.user_id JOIN organizations o ON o.id=m.organization_id WHERE m.organization_id=? AND m.user_id=? AND m.membership_role='owner' AND m.status='active' AND u.active=1 AND o.inactivated_at IS NULL", org, user),
		assertion("SELECT 1 FROM service_providers WHERE id=? AND active=1", provider),
	}, nil
}

func (s *Service) CreateOperableCustomActionAuthorized(ctx context.Context, user, org string, in CustomActionInput) (Action, error) {
	return s.CreateCustomActionAuthorized(ctx, user, org, "operable", in)
}

func (s *Service) CreateCustomActionAuthorized(ctx context.Context, user, org, provider string, in CustomActionInput) (Action, error) {
	guard, err := s.ownerActionMutationGuard(ctx, user, org, provider)
	if err != nil {
		return Action{}, err
	}
	if !customActionCode.MatchString(in.Code) || !strings.HasPrefix(in.Code, organizationCustomNamespace(org)) {
		return Action{}, serviceError(400, "code must be custom.org-<sha256-of-organization-id>.<feature>.<definition-id>.<operation> for this organization")
	}
	in.Name, err = required("name", in.Name)
	if err != nil {
		return Action{}, err
	}
	if !enumValue(in.RiskLevel, "low", "medium", "high", "critical") {
		return Action{}, serviceError(400, "invalid riskLevel")
	}
	in.Description = optionalText(in.Description)
	id := provider + "." + in.Code
	now := nowString()
	stmts := append(guard, assertion("SELECT 1 WHERE (SELECT COUNT(*) FROM actions WHERE provider_id=?)<999", provider), Statement{SQL: "INSERT INTO actions(id,provider_id,code,name,description,risk_level,visibility_granting,grantable_to_applications,created_at,updated_at) VALUES(?,?,?,?,?,?,0,0,?,?)", Args: []any{id, provider, in.Code, in.Name, in.Description, in.RiskLevel, now, now}, ExpectedRowsAffected: requireOne()})
	if err = s.store.Transaction(ctx, stmts); err != nil {
		return Action{}, capabilityWriteError(lifecycleError(err))
	}
	return serviceRow[Action](ctx, s, ReadStrong, "SELECT * FROM actions WHERE id=? AND provider_id=?", id, provider)
}

func (s *Service) PatchOperableCustomActionAuthorized(ctx context.Context, user, org, id string, in CustomActionPatch) (Action, error) {
	return s.PatchCustomActionAuthorized(ctx, user, org, "operable", id, in)
}

func (s *Service) PatchCustomActionAuthorized(ctx context.Context, user, org, provider, id string, in CustomActionPatch) (Action, error) {
	guard, err := s.ownerActionMutationGuard(ctx, user, org, provider)
	if err != nil {
		return Action{}, err
	}
	if in.Name == nil && !in.Description.Present && in.RiskLevel == nil && in.GrantableToApplications == nil {
		return Action{}, serviceError(400, "no fields to patch")
	}
	var sets []string
	var args []any
	if in.GrantableToApplications != nil {
		if _, ok := ctx.Value(actionPreconditionKey{}).(ActionSnapshot); !ok {
			return Action{}, serviceError(428, "If-Match is required")
		}
		sets = append(sets, "grantable_to_applications=?")
		args = append(args, *in.GrantableToApplications)
		if !*in.GrantableToApplications {
			for _, table := range []string{"application_actions", "application_grant_actions"} {
				guard = append(guard, assertion("SELECT 1 WHERE NOT EXISTS(SELECT 1 FROM "+table+" WHERE action_id=?)", id))
			}
		}
	}
	if in.Name != nil {
		value, e := required("name", *in.Name)
		if e != nil {
			return Action{}, e
		}
		sets = append(sets, "name=?")
		args = append(args, value)
	}
	if in.Description.Present {
		sets = append(sets, "description=?")
		args = append(args, optionalText(in.Description.Value))
	}
	if in.RiskLevel != nil {
		if !enumValue(*in.RiskLevel, "low", "medium", "high", "critical") {
			return Action{}, serviceError(400, "invalid riskLevel")
		}
		sets = append(sets, "risk_level=?")
		args = append(args, *in.RiskLevel)
	}
	sets = append(sets, "updated_at=?")
	args = append(args, nowString(), id)
	namespace := organizationCustomNamespace(org)
	guard = append(guard, assertion("SELECT 1 FROM actions WHERE id=? AND provider_id=? AND code GLOB ? AND id=provider_id||'.'||code", id, provider, namespace+"*"))
	args = append(args, provider)
	guard = append(guard, Statement{SQL: "UPDATE actions SET " + strings.Join(sets, ",") + " WHERE id=? AND provider_id=? AND code GLOB ?", Args: append(args, namespace+"*"), ExpectedRowsAffected: requireOne()})
	if err = s.store.actionTransaction(ctx, guard); err != nil {
		return Action{}, capabilityWriteError(lifecycleError(err))
	}
	return serviceRow[Action](ctx, s, ReadStrong, "SELECT * FROM actions WHERE id=? AND provider_id=?", id, provider)
}

func (s *Service) DeleteOperableCustomActionAuthorized(ctx context.Context, user, org, id string) error {
	return s.DeleteCustomActionAuthorized(ctx, user, org, "operable", id)
}

func (s *Service) DeleteCustomActionAuthorized(ctx context.Context, user, org, provider, id string) error {
	guard, err := s.ownerActionMutationGuard(ctx, user, org, provider)
	if err != nil {
		return err
	}
	const refs = `SELECT 1 FROM provider_http_capability_sets c JOIN json_tree(c.canonical_json) ref ON ref.key='actionId' WHERE c.provider_id=? AND ref.value=?`
	guard = append(guard, assertion("SELECT 1 FROM actions WHERE id=? AND provider_id=? AND code GLOB ? AND id=provider_id||'.'||code", id, provider, organizationCustomNamespace(org)+"*"), assertion("SELECT 1 WHERE NOT EXISTS("+refs+")", provider, id))
	for _, table := range []string{"application_grant_actions", "application_actions", "role_actions"} {
		guard = append(guard, assertion("SELECT 1 WHERE NOT EXISTS(SELECT 1 FROM "+table+" WHERE action_id=?)", id))
	}
	guard = append(guard, Statement{SQL: "DELETE FROM actions WHERE id=? AND provider_id=? AND code GLOB ?", Args: []any{id, provider, organizationCustomNamespace(org) + "*"}, ExpectedRowsAffected: requireOne()})
	if err = s.store.Transaction(ctx, guard); err != nil {
		return capabilityWriteError(lifecycleError(err))
	}
	return nil
}

func (s *Service) SetRoleAction(ctx context.Context, user, org, role, action string, present bool) error {
	if _, err := s.RoleActionsSnapshot(ctx, user, org, role); err != nil {
		return err
	}
	if _, err := required("actionId", action); err != nil {
		return err
	}
	stmts := []Statement{assertion("SELECT 1 FROM roles WHERE id=? AND organization_id=?", role, org)}
	if present {
		stmts = append(stmts, assertion("SELECT 1 FROM actions WHERE id=?", action), assertion("SELECT 1 WHERE (SELECT COUNT(*) FROM role_actions WHERE role_id=?)<999 OR EXISTS(SELECT 1 FROM role_actions WHERE role_id=? AND action_id=?)", role, role, action), Statement{SQL: "INSERT INTO role_actions(role_id,action_id) VALUES(?,?) ON CONFLICT DO NOTHING", Args: []any{role, action}})
	} else {
		stmts = append(stmts, Statement{SQL: "DELETE FROM role_actions WHERE role_id=? AND action_id=?", Args: []any{role, action}})
	}
	if err := s.store.Transaction(ctx, stmts); err != nil {
		return capabilityWriteError(err)
	}
	return nil
}

func (s *Service) SetApplicationAction(ctx context.Context, user, org, app, action string, present bool) error {
	a, err := s.GetApplicationAuthorized(ctx, user, org, app, ReadStrong)
	if err != nil {
		return err
	}
	if a.FirstParty && a.ConsentPolicy == "auto_approve" {
		return serviceError(403, "forbidden: trusted application actions cannot be changed")
	}
	if err = s.applicationMutationAvailable(ctx, app); err != nil {
		return err
	}
	if _, err = required("actionId", action); err != nil {
		return err
	}
	stmts := []Statement{lifecycleActorGuard(user, org, "organization.application.manage"), assertion("SELECT 1 FROM applications WHERE id=? AND organization_id=? AND NOT(first_party=1 AND consent_policy='auto_approve') AND (security_mutation_token IS NULL OR security_mutation_expires_at<=?)", app, org, nowString())}
	if present {
		stmts = append(stmts, assertion("SELECT 1 FROM actions a WHERE a.id=? AND "+applicationActionNamespaceSQL, action, organizationCustomNamespace(org)+"*"))
		stmts = append(stmts, assertion("SELECT 1 FROM actions a JOIN service_providers p ON p.id=a.provider_id WHERE a.id=? AND a.grantable_to_applications=1 AND p.active=1", action), assertion("SELECT 1 WHERE (SELECT COUNT(*) FROM application_actions WHERE application_id=?)<999 OR EXISTS(SELECT 1 FROM application_actions WHERE application_id=? AND action_id=?)", app, app, action), Statement{SQL: "INSERT INTO application_actions(application_id,action_id) VALUES(?,?) ON CONFLICT DO NOTHING", Args: []any{app, action}})
	} else {
		stmts = append(stmts, Statement{SQL: "DELETE FROM application_grant_actions WHERE action_id=? AND application_grant_id IN(SELECT id FROM application_grants WHERE application_id=?)", Args: []any{action, app}}, Statement{SQL: "DELETE FROM application_actions WHERE application_id=? AND action_id=?", Args: []any{app, action}})
	}
	return applicationMutationError(s.store.Transaction(ctx, stmts))
}

func (s *Service) CreateProviderAction(ctx context.Context, provider string, a Action) (Action, error) {
	if _, err := s.ListProviderActionsAdmin(ctx, provider); err != nil {
		return Action{}, err
	}
	code, err := required("action.code", a.Code)
	if err != nil {
		return Action{}, err
	}
	id := code
	if provider != "grant" && !strings.HasPrefix(code, provider+".") {
		id = provider + "." + code
	}
	if a.ID != code && a.ID != id {
		return Action{}, serviceError(400, "action.id must match normalized action id "+id)
	}
	if a.ProviderID != provider {
		return Action{}, serviceError(400, "providerId must match path")
	}
	if a.VisibilityGranting {
		return Action{}, serviceError(400, "visibilityGranting=true is not supported")
	}
	if !enumValue(a.RiskLevel, "low", "medium", "high", "critical") {
		return Action{}, serviceError(400, "invalid riskLevel")
	}
	a.Name, err = required("action.name", a.Name)
	if err != nil {
		return Action{}, err
	}
	a.ID, a.Code = id, code
	a.Description, a.ConsentLabel, a.ConsentDescription = optionalText(a.Description), optionalText(a.ConsentLabel), optionalText(a.ConsentDescription)
	now := nowString()
	err = s.store.Transaction(ctx, []Statement{assertion("SELECT 1 FROM service_providers WHERE id=?", provider), assertion("SELECT 1 WHERE (SELECT COUNT(*) FROM actions WHERE provider_id=?)<999", provider), {SQL: "INSERT INTO actions(id,provider_id,code,name,description,risk_level,visibility_granting,grantable_to_applications,consent_label,consent_description,created_at,updated_at) VALUES(?,?,?,?,?,?,0,?,?,?,?,?)", Args: []any{a.ID, provider, a.Code, a.Name, a.Description, a.RiskLevel, a.GrantableToApplications, a.ConsentLabel, a.ConsentDescription, now, now}}})
	if err != nil {
		return Action{}, capabilityWriteError(err)
	}
	return serviceRow[Action](ctx, s, ReadStrong, "SELECT * FROM actions WHERE id=? AND provider_id=?", id, provider)
}

// PatchProviderAction changes only supplied metadata. IDs and codes are immutable.
// Null clears nullable text; omitted fields remain unchanged.
func (s *Service) PatchProviderAction(ctx context.Context, provider, id string, fields map[string]json.RawMessage) (Action, error) {
	if _, err := serviceRow[Action](ctx, s, ReadStrong, "SELECT * FROM actions WHERE id=? AND provider_id=?", id, provider); err != nil {
		return Action{}, err
	}
	if len(fields) == 0 {
		return Action{}, serviceError(400, "no fields to patch")
	}
	columns := map[string]string{"name": "name", "description": "description", "riskLevel": "risk_level", "visibilityGranting": "visibility_granting", "grantableToApplications": "grantable_to_applications", "consentLabel": "consent_label", "consentDescription": "consent_description"}
	// Fixed order keeps statements deterministic regardless of Go map order.
	for key := range fields {
		if _, ok := columns[key]; !ok {
			return Action{}, serviceError(422, "unknown or immutable action field: "+key)
		}
	}
	var sets []string
	var args []any
	revoke := false
	for _, key := range []string{"name", "description", "riskLevel", "visibilityGranting", "grantableToApplications", "consentLabel", "consentDescription"} {
		raw, ok := fields[key]
		if !ok {
			continue
		}
		var value any
		if key == "visibilityGranting" || key == "grantableToApplications" {
			var b bool
			if string(raw) == "null" || json.Unmarshal(raw, &b) != nil {
				return Action{}, serviceError(422, key+" must be boolean")
			}
			if key == "visibilityGranting" && b {
				return Action{}, serviceError(400, "visibilityGranting=true is not supported")
			}
			value = b
			if key == "grantableToApplications" && !b {
				revoke = true
			}
		} else {
			var text *string
			if json.Unmarshal(raw, &text) != nil {
				return Action{}, serviceError(422, key+" must be text or null")
			}
			if key == "name" || key == "riskLevel" {
				if text == nil {
					return Action{}, serviceError(422, key+" must not be null")
				}
				v, err := required(key, *text)
				if err != nil {
					return Action{}, err
				}
				if key == "riskLevel" && !enumValue(v, "low", "medium", "high", "critical") {
					return Action{}, serviceError(400, "invalid riskLevel")
				}
				value = v
			} else {
				value = optionalText(text)
			}
		}
		sets = append(sets, columns[key]+"=?")
		args = append(args, value)
	}
	sets = append(sets, "updated_at=?")
	args = append(args, nowString(), id, provider)
	stmts := []Statement{{SQL: "UPDATE actions SET " + strings.Join(sets, ",") + " WHERE id=? AND provider_id=?", Args: args, ExpectedRowsAffected: requireOne()}}
	if revoke {
		for _, table := range []string{"application_grant_actions", "application_actions"} {
			stmts = append(stmts, Statement{SQL: "DELETE FROM " + table + " WHERE action_id=?", Args: []any{id}})
		}
	}
	if err := s.store.Transaction(ctx, stmts); err != nil {
		return Action{}, capabilityWriteError(err)
	}
	return serviceRow[Action](ctx, s, ReadStrong, "SELECT * FROM actions WHERE id=? AND provider_id=?", id, provider)
}

func (s *Service) DeleteProviderAction(ctx context.Context, provider, id string) error {
	if _, err := s.ListProviderActionsAdmin(ctx, provider); err != nil {
		return err
	}
	// Repeat the dependency check inside the delete transaction so activation
	// cannot introduce a dangling reference between the check and the write.
	const references = `SELECT 1 FROM provider_http_capability_active ac JOIN provider_http_capability_sets c ON c.provider_id=ac.provider_id AND c.generation=ac.generation JOIN json_tree(c.canonical_json) ref ON ref.key='actionId' WHERE ac.provider_id=? AND ref.value=?`
	stmts := []Statement{assertion("SELECT 1 WHERE NOT EXISTS("+references+")", provider, id)}
	for _, table := range []string{"application_grant_actions", "application_actions", "role_actions"} {
		stmts = append(stmts, Statement{SQL: "DELETE FROM " + table + " WHERE action_id IN(SELECT id FROM actions WHERE id=? AND provider_id=?)", Args: []any{id, provider}})
	}
	stmts = append(stmts, Statement{SQL: "DELETE FROM actions WHERE id=? AND provider_id=?", Args: []any{id, provider}})
	if err := s.store.Transaction(ctx, stmts); err != nil {
		return capabilityWriteError(err)
	}
	return nil
}
