package grant

import "context"

// AdmitFirstPartyApplicationActionAdmin is an operational master-only addition;
// it does not change trust, identities, role permissions, or user consent.
func (s *Service) AdmitFirstPartyApplicationActionAdmin(ctx context.Context, org, app, action string) error {
	for name, value := range map[string]string{"organizationId": org, "applicationId": app, "actionId": action} {
		if _, err := required(name, value); err != nil {
			return err
		}
	}
	return applicationMutationError(s.store.admitFirstPartyApplicationAction(ctx, org, app, action))
}
