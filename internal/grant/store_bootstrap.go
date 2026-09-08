package grant

import (
	"context"
	"encoding/json"
	"fmt"
	"strings"
)

func (s *Store) EnsureBootstrapData(ctx context.Context) error {
	id, name := s.cfg.Security.BootstrapUserID, s.cfg.Security.BootstrapUserName
	if id == "" {
		id = "conalog-grant"
	}
	if name == "" {
		name = "Conalog Grant"
	}
	now := nowString()
	actions := []struct {
		code, name, risk string
		grantable        bool
	}{{"organization.application.manage", "Manage Organization Applications", "high", false}, {"organization.profile.read", "Read Organization Profile", "low", true}, {"organization.member.manage", "Manage Organization Members", "high", false}, {"organization.group.manage", "Manage Organization Groups", "high", false}, {"organization.role.manage", "Manage Organization Roles", "high", false}, {"organization.permission.manage", "Manage Organization Permissions", "critical", false}}
	stmts := []Statement{{SQL: "INSERT INTO users(id,email,display_name,active,created_at) VALUES(?,NULL,?,1,?) ON CONFLICT(id) DO UPDATE SET display_name=excluded.display_name", Args: []any{id, name, now}}, {SQL: "INSERT INTO organizations(id,name,created_at,updated_at) VALUES(?,?,?,?) ON CONFLICT(id) DO UPDATE SET name=excluded.name,updated_at=excluded.updated_at", Args: []any{id, name, now, now}}, {SQL: "INSERT INTO organization_memberships(id,organization_id,user_id,membership_role,status,created_at,updated_at) VALUES(?,?,?,'owner','active',?,?) ON CONFLICT(organization_id,user_id) DO UPDATE SET membership_role='owner',status='active',updated_at=excluded.updated_at", Args: []any{requestID(), id, id, now, now}}, {SQL: "INSERT INTO service_providers(id,name,description,active,created_at,updated_at) VALUES('grant','Grant','Grant organization authorization service',1,?,?) ON CONFLICT(id) DO UPDATE SET name=excluded.name,description=excluded.description,active=excluded.active,updated_at=excluded.updated_at", Args: []any{now, now}}, {SQL: "INSERT INTO roles(id,organization_id,code,name,description,created_at,updated_at) VALUES('role-system-bootstrap-owner',NULL,'system.bootstrap.owner','System Bootstrap Owner','Bootstrap organization owner capabilities',?,?) ON CONFLICT(id) DO UPDATE SET name=excluded.name,updated_at=excluded.updated_at", Args: []any{now, now}}}
	for _, a := range actions {
		stmts = append(stmts, Statement{SQL: "INSERT INTO actions(id,provider_id,code,name,risk_level,visibility_granting,grantable_to_applications,created_at,updated_at) VALUES(?,'grant',?,?,?,0,?,?,?) ON CONFLICT(id) DO UPDATE SET name=excluded.name,risk_level=excluded.risk_level,grantable_to_applications=excluded.grantable_to_applications,updated_at=excluded.updated_at", Args: []any{a.code, a.code, a.name, a.risk, a.grantable, now, now}}, Statement{SQL: "INSERT INTO role_actions(role_id,action_id) VALUES('role-system-bootstrap-owner',?) ON CONFLICT DO NOTHING", Args: []any{a.code}})
	}
	stmts = append(stmts, Statement{SQL: "INSERT INTO role_bindings(id,subject_type,subject_id,subject_organization_id,provider_id,resource_type,resource_id,role_id,state,created_by_user_id,created_at,updated_at) VALUES('binding-system-bootstrap-owner','user',?,?,'grant','organization',?,'role-system-bootstrap-owner','active',?,?,?) ON CONFLICT(id) DO UPDATE SET updated_at=excluded.updated_at", Args: []any{id, id, id, id, now, now}})
	// The admission row and every seed write share one transaction. Existing bootstrap
	// identity (including disabled restored authority) makes the entire seed a no-op.
	gate := requestID()
	for i := range stmts {
		q := stmts[i].SQL
		start := strings.Index(q, "VALUES(")
		end := strings.Index(q, ") ON CONFLICT")
		if start < 0 || end < start {
			return fmt.Errorf("bootstrap SQL is not a guarded insert")
		}
		stmts[i].SQL = q[:start] + "SELECT " + q[start+len("VALUES("):end] + " WHERE EXISTS(SELECT 1 FROM application_consent_assertions WHERE id='" + gate + "')" + q[end+1:]
	}
	stmts = append([]Statement{{SQL: "INSERT INTO application_consent_assertions(id,valid) SELECT ?,1 WHERE NOT EXISTS(SELECT 1 FROM users WHERE id=?)", Args: []any{gate, id}}}, stmts...)
	return s.Transaction(ctx, stmts)
}
func jsonString(v any) (string, error) { b, e := json.Marshal(v); return string(b), e }
func requireOne() *int64               { n := int64(1); return &n }
func assertion(sql string, args ...any) Statement {
	return Statement{SQL: fmt.Sprintf("INSERT INTO application_consent_assertions(id,valid) SELECT ?,CASE WHEN (%s) THEN 1 ELSE 0 END", sql), Args: append([]any{requestID()}, args...)}
}
