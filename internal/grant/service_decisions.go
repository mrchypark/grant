package grant

import (
	"context"
	"fmt"
)

type applicationAuthorizationContext struct{ UserID, OrganizationID, ApplicationID, ExternalSubject, ProviderAudience string }

func (s *Service) resolveApplicationAuthorizationContext(ctx context.Context, token, org string) (applicationAuthorizationContext, error) {
	org, err := required("organizationId", org)
	if err != nil {
		return applicationAuthorizationContext{}, err
	}
	identity, err := s.resolveExternalIdentity(ctx, token, ReadStrong)
	if err != nil {
		return applicationAuthorizationContext{}, err
	}
	u, err := s.localIdentity(ctx, identity, ReadStrong)
	if err != nil {
		return applicationAuthorizationContext{}, err
	}
	if !u.Active {
		return applicationAuthorizationContext{}, serviceError(403, "forbidden: local user is inactive")
	}
	var a Application
	if s.AuthProviderMode() == "patch-data" {
		a, err = s.store.GetApplicationWithConsistency(ctx, s.cfg.Auth.PatchData.ExposureApplicationID, ReadStrong)
		if err != nil {
			return applicationAuthorizationContext{}, serviceError(403, "forbidden: configured Patch Data exposure application is not ready")
		}
		if a.OrganizationID != org || !a.Active || a.ProvisioningStatus != "ready" {
			return applicationAuthorizationContext{}, serviceError(403, "forbidden: configured Patch Data exposure application is not active for this organization")
		}
	} else {
		rows, e := serviceRows[Application](ctx, s, ReadStrong, "SELECT * FROM applications WHERE rauthy_client_id=? AND organization_id=? AND active=1 AND provisioning_status='ready'", identity.ClientID, org)
		if e != nil {
			return applicationAuthorizationContext{}, e
		}
		if len(rows) != 1 {
			return applicationAuthorizationContext{}, serviceError(403, fmt.Sprintf("forbidden: token client %s is not an active application in organization %s", identity.ClientID, org))
		}
		a = rows[0]
	}
	return applicationAuthorizationContext{u.ID, org, a.ID, identity.ExternalUserID, identity.ClientID}, nil
}
func (s *Service) CheckApplicationAuthorizations(ctx context.Context, token, org string, checks []ApplicationAuthorizationCheck) ([]AuthorizationDecision, error) {
	c, err := s.resolveApplicationAuthorizationContext(ctx, token, org)
	if err != nil {
		return nil, err
	}
	exact := make([]AuthorizationCheck, len(checks))
	for i, v := range checks {
		exact[i] = AuthorizationCheck{Subject: AuthorizationSubject{SubjectType: "user", ID: c.UserID, OrganizationID: c.OrganizationID}, Action: v.Action, Resource: v.Resource}
	}
	return s.evaluateAuthorizations(ctx, exact, ReadStrong, &c.ApplicationID)
}
