package grant

import (
	"context"
	"crypto/sha256"
	"encoding/hex"
	"encoding/json"
	"fmt"
	"strings"
)

// ActionSnapshot captures the response and its write precondition in one read.
// SQL is selected by the server, never supplied by an HTTP caller.
type ActionSnapshot struct {
	Value json.RawMessage
	ETag  string
	query string
	args  []any
	state string
}

const actionJSON = `json_object('id',a.id,'providerId',a.provider_id,'code',a.code,'name',a.name,'description',a.description,'riskLevel',a.risk_level,'visibilityGranting',json(CASE WHEN a.visibility_granting THEN 'true' ELSE 'false' END),'grantableToApplications',json(CASE WHEN a.grantable_to_applications THEN 'true' ELSE 'false' END),'consentLabel',a.consent_label,'consentDescription',a.consent_description,'createdAt',a.created_at,'updatedAt',a.updated_at)`

func actionArraySQL(filter string) string {
	return `SELECT COALESCE(json_group_array(json(item)),'[]') FROM (SELECT ` + actionJSON + ` AS item FROM actions a WHERE ` + filter + ` ORDER BY a.code,a.id)`
}

func (s *Store) actionSnapshot(ctx context.Context, query string, args ...any) (ActionSnapshot, error) {
	// Chunk one SQL snapshot to respect the replication transport's scalar limit.
	rows, err := s.Query(ctx, ReadStrong, "WITH RECURSIVE snapshot AS (SELECT ("+query+") AS state), chunks(part,remaining,n) AS (SELECT substr(state,1,15000),substr(state,15001),0 FROM snapshot UNION ALL SELECT substr(remaining,1,15000),substr(remaining,15001),n+1 FROM chunks WHERE remaining<>'') SELECT part FROM chunks ORDER BY n", args...)
	if err != nil {
		return ActionSnapshot{}, err
	}
	if len(rows) == 0 {
		return ActionSnapshot{}, fmt.Errorf("action snapshot missing")
	}
	var parts strings.Builder
	for _, row := range rows {
		parts.WriteString(rowString(row, "part"))
	}
	state := parts.String()
	if !json.Valid([]byte(state)) {
		return ActionSnapshot{}, fmt.Errorf("invalid action snapshot")
	}
	// Keep the existing public DTO (including omitted nullable fields) while
	// retaining storage metadata in the atomic comparison and validator.
	var value json.RawMessage
	if state == "null" {
		value = json.RawMessage("null")
	} else if strings.HasPrefix(state, "[") {
		var actions []Action
		if err = json.Unmarshal([]byte(state), &actions); err != nil {
			return ActionSnapshot{}, err
		}
		value, err = json.Marshal(actions)
	} else {
		var envelope struct {
			Application json.RawMessage `json:"application"`
			Actions     []Action        `json:"actions"`
		}
		if err = json.Unmarshal([]byte(state), &envelope); err != nil {
			return ActionSnapshot{}, err
		}
		if envelope.Application != nil {
			value, err = json.Marshal(envelope.Actions)
		} else {
			var consent ApplicationConsent
			if err = json.Unmarshal([]byte(state), &consent); err != nil {
				return ActionSnapshot{}, err
			}
			value, err = json.Marshal(consent)
		}
	}
	if err != nil {
		return ActionSnapshot{}, err
	}
	sum := sha256.Sum256([]byte(state))
	return ActionSnapshot{Value: value, ETag: `"` + hex.EncodeToString(sum[:]) + `"`, query: query, args: args, state: state}, nil
}

func (v ActionSnapshot) guard(stageID string) Statement {
	args := append([]any(nil), v.args...)
	args = append(args, stageID, "action-precondition")
	return assertion("SELECT 1 WHERE ("+v.query+")="+stagedTextSQL(), args...)
}

type actionPreconditionKey struct{}

// Only the four action-set mutation transactions opt into this guard. Staging,
// authentication and unrelated writes must not consume the precondition.
func (s *Store) actionTransaction(ctx context.Context, stmts []Statement) error {
	snapshot, checked := ctx.Value(actionPreconditionKey{}).(ActionSnapshot)
	if checked {
		stageID := requestID()
		if err := s.stageText(ctx, stageID, "action-precondition", snapshot.state); err != nil {
			return err
		}
		defer func() {
			_, _ = s.Exec(ctx, "DELETE FROM grant_import_payloads WHERE import_id=? AND kind='action-precondition'", stageID)
		}()
		stmts = append([]Statement{snapshot.guard(stageID)}, stmts...)
	}
	err := s.Transaction(ctx, stmts)
	if err != nil && checked {
		current, readErr := s.actionSnapshot(ctx, snapshot.query, snapshot.args...)
		if readErr == nil && current.state != snapshot.state {
			return serviceError(412, "action state changed; read the current ETag and retry")
		}
	}
	return err
}

func checkedActionContext(ctx context.Context, v ActionSnapshot, match, none string) (context.Context, error) {
	if match != "" && none != "" {
		return nil, serviceError(400, "use only one conditional header")
	}
	if none != "" {
		if none != "*" {
			return nil, serviceError(400, "If-None-Match must be *")
		}
		if v.state != "null" {
			return nil, serviceError(412, "consent already exists")
		}
	} else {
		if match == "" {
			return nil, serviceError(428, "If-Match is required")
		}
		if v.state == "null" {
			return nil, serviceError(412, "consent does not exist; use If-None-Match: * to create it")
		}
		if match != v.ETag {
			return nil, serviceError(412, "action state changed; read the current ETag and retry")
		}
	}
	return context.WithValue(ctx, actionPreconditionKey{}, v), nil
}

func (s *Service) ProviderActionsSnapshot(ctx context.Context, provider string) (ActionSnapshot, error) {
	if _, err := s.ListProviderActionsAdmin(ctx, provider); err != nil {
		return ActionSnapshot{}, err
	}
	return s.store.actionSnapshot(ctx, actionArraySQL("a.provider_id=?"), provider)
}
func (s *Service) RoleActionsSnapshot(ctx context.Context, user, org, id string) (ActionSnapshot, error) {
	if err := s.requireOrganizationAction(ctx, user, org, org, "organization.permission.manage", "manage organization roles", ReadStrong); err != nil {
		return ActionSnapshot{}, err
	}
	if _, err := serviceRow[Role](ctx, s, ReadStrong, "SELECT * FROM roles WHERE id=? AND (organization_id IS NULL OR organization_id=?)", id, org); err != nil {
		return ActionSnapshot{}, err
	}
	return s.store.actionSnapshot(ctx, actionArraySQL("EXISTS(SELECT 1 FROM role_actions r WHERE r.role_id=? AND r.action_id=a.id)"), id)
}
func (s *Service) ApplicationActionsSnapshot(ctx context.Context, user, org, id string) (ActionSnapshot, error) {
	if _, err := s.GetApplicationAuthorized(ctx, user, org, id, ReadStrong); err != nil {
		return ActionSnapshot{}, err
	}
	// Include identity metadata even for an empty action set. An ETag from a
	// deleted application must not authorize a replacement with the same ID.
	query := `SELECT COALESCE((SELECT json_object('application',json_object('id',app.id,'organizationId',app.organization_id,'createdAt',app.created_at),'actions',json((` + actionArraySQL("EXISTS(SELECT 1 FROM application_actions r WHERE r.application_id=app.id AND r.action_id=a.id)") + `))) FROM applications app WHERE app.id=? AND app.organization_id=?),'null')`
	return s.store.actionSnapshot(ctx, query, id, org)
}
func (s *Service) ConsentSnapshot(ctx context.Context, user, org, id string) (ActionSnapshot, error) {
	if _, err := s.applicationInOrg(ctx, id, org); err != nil {
		return ActionSnapshot{}, err
	}
	query := `SELECT COALESCE((SELECT json_object('grant',json_object('id',g.id,'userId',g.user_id,'applicationId',g.application_id,'organizationId',g.organization_id,'status',g.status,'consentedAt',g.consented_at,'revokedAt',g.revoked_at,'revokedByUserId',g.revoked_by_user_id,'createdAt',g.created_at,'updatedAt',g.updated_at),'actions',json((` + actionArraySQL("EXISTS(SELECT 1 FROM application_grant_actions ga WHERE ga.application_grant_id=g.id AND ga.action_id=a.id)") + `))) FROM application_grants g WHERE g.user_id=? AND g.application_id=? AND g.organization_id=? AND g.status='active'),'null')`
	return s.store.actionSnapshot(ctx, query, user, id, org)
}

func (s *Service) ReplaceProviderActionsConditional(ctx context.Context, provider string, actions []Action, match string) ([]Action, error) {
	snapshot, err := s.ProviderActionsSnapshot(ctx, provider)
	if err != nil {
		return nil, err
	}
	ctx, err = checkedActionContext(ctx, snapshot, match, "")
	if err != nil {
		return nil, err
	}
	return s.ReplaceProviderActionsAdmin(ctx, provider, actions)
}
func (s *Service) ReplaceRoleActionsConditional(ctx context.Context, user, org, id string, actions []string, match string) ([]Action, error) {
	snapshot, err := s.RoleActionsSnapshot(ctx, user, org, id)
	if err != nil {
		return nil, err
	}
	ctx, err = checkedActionContext(ctx, snapshot, match, "")
	if err != nil {
		return nil, err
	}
	return s.ReplaceRoleActionsAuthorized(ctx, user, org, id, actions)
}
func (s *Service) ReplaceApplicationActionsConditional(ctx context.Context, user, org, id string, actions []string, match string) ([]Action, error) {
	snapshot, err := s.ApplicationActionsSnapshot(ctx, user, org, id)
	if err != nil {
		return nil, err
	}
	ctx, err = checkedActionContext(ctx, snapshot, match, "")
	if err != nil {
		return nil, err
	}
	return s.ReplaceApplicationActionsAuthorized(ctx, user, org, id, actions)
}
func (s *Service) ReplaceConsentConditional(ctx context.Context, user, org, id string, actions []string, match, none string) (ApplicationConsent, error) {
	snapshot, err := s.ConsentSnapshot(ctx, user, org, id)
	if err != nil {
		return ApplicationConsent{}, err
	}
	ctx, err = checkedActionContext(ctx, snapshot, match, none)
	if err != nil {
		return ApplicationConsent{}, err
	}
	return s.ReplaceApplicationConsent(ctx, user, org, id, actions)
}
