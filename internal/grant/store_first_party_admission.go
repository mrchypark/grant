package grant

import "context"

func (s *Store) admitFirstPartyApplicationAction(ctx context.Context, org, app, action string) error {
	return s.Transaction(ctx, []Statement{
		assertion(`SELECT 1 FROM applications a JOIN organizations o ON o.id=a.organization_id
WHERE a.id=? AND a.organization_id=? AND o.inactivated_at IS NULL
AND a.active=1 AND a.provisioning_status='ready' AND a.first_party=1 AND a.consent_policy='auto_approve'
AND (a.security_mutation_token IS NULL OR a.security_mutation_expires_at<=?)`, app, org, nowString()),
		assertion(`SELECT 1 FROM actions a JOIN service_providers p ON p.id=a.provider_id
WHERE a.id=? AND p.active=1 AND a.grantable_to_applications=1
AND a.id=CASE WHEN a.provider_id='grant' OR substr(a.code,1,length(a.provider_id)+1)=a.provider_id||'.'
THEN a.code ELSE a.provider_id||'.'||a.code END`, action),
		assertion(`SELECT 1 WHERE (SELECT COUNT(*) FROM application_actions WHERE application_id=?)<999
OR EXISTS(SELECT 1 FROM application_actions WHERE application_id=? AND action_id=?)`, app, app, action),
		{SQL: "INSERT INTO application_actions(application_id,action_id) VALUES(?,?) ON CONFLICT DO NOTHING", Args: []any{app, action}},
	})
}
