package grant

import (
	"context"
	"encoding/json"
	"strings"
)

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
	stmts := []Statement{assertion("SELECT 1 FROM applications WHERE id=? AND organization_id=? AND NOT(first_party=1 AND consent_policy='auto_approve') AND (security_mutation_token IS NULL OR security_mutation_expires_at<=?)", app, org, nowString())}
	if present {
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
