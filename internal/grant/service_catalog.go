package grant

import (
	"context"
	"fmt"
	"strings"
)

func (s *Service) ListServiceProviders(ctx context.Context, c ReadConsistency) ([]ServiceProvider, error) {
	return serviceRows[ServiceProvider](ctx, s, c, "SELECT * FROM service_providers ORDER BY name,id")
}
func (s *Service) UpsertServiceProviderAdmin(ctx context.Context, p ServiceProvider) (ServiceProvider, error) {
	var err error
	p.ID, err = required("id", p.ID)
	if err != nil {
		return p, err
	}
	p.Name, err = required("name", p.Name)
	if err != nil {
		return p, err
	}
	p.Description = optionalText(p.Description)
	_, err = s.store.Exec(ctx, "INSERT INTO service_providers(id,name,description,active,created_at,updated_at) VALUES(?,?,?,?,?,?) ON CONFLICT(id) DO UPDATE SET name=excluded.name,description=excluded.description,active=excluded.active,updated_at=excluded.updated_at", p.ID, p.Name, p.Description, p.Active, nowString(), nowString())
	return p, err
}
func (s *Service) ReplaceProviderActionsAdmin(ctx context.Context, provider string, actions []Action) ([]Action, error) {
	provider, err := required("providerId", provider)
	if err != nil {
		return nil, err
	}
	if len(actions) > 999 {
		return nil, serviceError(400, "actions must contain at most 999 items")
	}
	seen := map[string]bool{}
	normalized := make([]Action, 0, len(actions))
	for _, a := range actions {
		if a.VisibilityGranting {
			return nil, serviceError(400, "action.visibilityGranting=true is unavailable in the coarse deployment baseline")
		}
		code, e := required("action.code", a.Code)
		if e != nil {
			return nil, e
		}
		id := code
		if provider != "grant" && !strings.HasPrefix(code, provider+".") {
			id = provider + "." + code
		}
		if a.ID != code && a.ID != id {
			return nil, serviceError(400, "action.id must match normalized action id "+id)
		}
		if a.ProviderID != provider {
			return nil, serviceError(400, fmt.Sprintf("action %s providerId must match path providerId", id))
		}
		if seen[code] {
			return nil, serviceError(400, "duplicate action code in provider manifest: "+code)
		}
		seen[code] = true
		a.ID = id
		a.Code = code
		a.Name, e = required("action.name", a.Name)
		if e != nil {
			return nil, e
		}
		a.Description = optionalText(a.Description)
		a.ConsentLabel = optionalText(a.ConsentLabel)
		a.ConsentDescription = optionalText(a.ConsentDescription)
		normalized = append(normalized, a)
	}
	payload, err := jsonString(normalized)
	if err != nil {
		return nil, err
	}
	stmts := []Statement{assertion("SELECT 1 FROM service_providers WHERE id=?", provider)}
	for _, table := range []string{"application_grant_actions", "application_actions", "role_actions"} {
		stmts = append(stmts, Statement{SQL: "DELETE FROM " + table + " WHERE action_id IN(SELECT id FROM actions WHERE provider_id=? AND id NOT IN(SELECT json_extract(value,'$.id') FROM json_each(?)))", Args: []any{provider, payload}})
	}
	stmts = append(stmts, Statement{SQL: "DELETE FROM actions WHERE provider_id=? AND id NOT IN(SELECT json_extract(value,'$.id') FROM json_each(?))", Args: []any{provider, payload}}, Statement{SQL: "INSERT INTO actions(id,provider_id,code,name,description,risk_level,visibility_granting,grantable_to_applications,consent_label,consent_description,created_at,updated_at) SELECT json_extract(value,'$.id'),?,json_extract(value,'$.code'),json_extract(value,'$.name'),json_extract(value,'$.description'),json_extract(value,'$.riskLevel'),0,json_extract(value,'$.grantableToApplications'),json_extract(value,'$.consentLabel'),json_extract(value,'$.consentDescription'),?,? FROM json_each(?) WHERE 1 ON CONFLICT(provider_id,code) DO UPDATE SET name=excluded.name,description=excluded.description,risk_level=excluded.risk_level,grantable_to_applications=excluded.grantable_to_applications,consent_label=excluded.consent_label,consent_description=excluded.consent_description,updated_at=excluded.updated_at", Args: []any{provider, nowString(), nowString(), payload}})
	if err = s.store.Transaction(ctx, stmts); err != nil {
		return nil, err
	}
	return serviceRows[Action](ctx, s, ReadStrong, "SELECT * FROM actions WHERE provider_id=? ORDER BY code", provider)
}
