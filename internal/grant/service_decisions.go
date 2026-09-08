package grant

import (
	"context"
	"fmt"
	"time"
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
func (s *Service) CheckExposureReadAuthorization(ctx context.Context, token, org, exposure, resource string) (ExposureReadAuthorizationDecision, error) {
	return s.CheckOperationAuthorization(ctx, token, org, OperationAuthorizationTarget{ProviderID: "exposure", ActionID: "exposure.read", ResourceType: exposure, ResourceID: resource})
}
func (s *Service) CheckPatchDataPlantReadAuthorization(ctx context.Context, token, org, resource string) (ExposureReadAuthorizationDecision, error) {
	return s.CheckOperationAuthorization(ctx, token, org, OperationAuthorizationTarget{ProviderID: "patch-data", ActionID: "patch-data.plant.read", ResourceType: "plant", ResourceID: resource})
}
func (s *Service) CheckOperationAuthorization(ctx context.Context, token, org string, target OperationAuthorizationTarget) (ExposureReadAuthorizationDecision, error) {
	c, err := s.resolveApplicationAuthorizationContext(ctx, token, org)
	if err != nil {
		return ExposureReadAuthorizationDecision{}, err
	}
	ds, err := s.evaluateAuthorizations(ctx, []AuthorizationCheck{{Subject: AuthorizationSubject{SubjectType: "user", ID: c.UserID, OrganizationID: c.OrganizationID}, Action: target.ActionID, Resource: AuthorizationResource{ProviderID: target.ProviderID, ResourceType: target.ResourceType, ID: target.ResourceID}}}, ReadStrong, &c.ApplicationID)
	if err != nil {
		return ExposureReadAuthorizationDecision{}, err
	}
	issued := time.Now().UTC()
	return ExposureReadAuthorizationDecision{Allowed: ds[0].Allowed, Reason: ds[0].Reason, PrincipalDigest: ExposurePrincipalDigest(c.UserID, c.OrganizationID, c.ApplicationID), IssuedAt: issued.Format(time.RFC3339Nano), ExpiresAt: issued.Add(5 * time.Second).Format(time.RFC3339Nano)}, nil
}
