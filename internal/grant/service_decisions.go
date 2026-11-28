package grant

import (
	"context"
	"fmt"
)

type applicationAuthorizationContext struct{ UserID, OrganizationID, ApplicationID, ExternalSubject, ProviderAudience string }

// The caller must enforce the server PEP policy. User mode additionally requires
// the issuer owner's supported-user positive and client-credentials negative
// acceptance artifacts; same-sub UserInfo alone does not establish a human user.
func (s *Service) resolveLocalUserAuthorizationContext(ctx context.Context, identity ExternalIdentity, org string) (applicationAuthorizationContext, error) {
	org, err := required("organizationId", org)
	if err != nil {
		return applicationAuthorizationContext{}, err
	}
	_, trustedGateway := ctx.Value(trustedGatewayIdentityKey{}).(ExternalIdentity)
	cfg := s.cfg.Auth.Goauthy
	if trustedGateway {
		if !identity.Active || identity.ExternalUserID == "" {
			return applicationAuthorizationContext{}, serviceError(403, "forbidden: active trusted gateway identity required")
		}
	} else if s.AuthProviderMode() != "goauthy" || cfg == nil || identity.Provider != "goauthy" || !identity.Active || !identity.UserInfoVerified || identity.Issuer != cfg.Issuer || identity.ExternalUserID == "" || identity.TokenClientID == "" || identity.TokenAudience == "" {
		return applicationAuthorizationContext{}, serviceError(403, "forbidden: verified Goauthy user identity required")
	}
	u, err := s.localIdentity(ctx, identity, ReadStrong)
	if err != nil {
		return applicationAuthorizationContext{}, err
	}
	if !u.Active {
		return applicationAuthorizationContext{}, serviceError(403, "forbidden: local user is inactive")
	}
	member, err := s.store.HasActiveOrganizationMembershipWithConsistency(ctx, u.ID, org, ReadStrong)
	if err != nil {
		return applicationAuthorizationContext{}, err
	}
	if !member {
		return applicationAuthorizationContext{}, serviceError(403, "forbidden: local user is not an active member of an active organization")
	}
	subject := identity.ExternalUserID
	if !trustedGateway && identity.Issuer != "" {
		subject = goauthySubject(identity.Issuer, identity.ExternalUserID)
	}
	audience := identity.TokenAudience
	if trustedGateway && audience == "" {
		audience = identity.TokenClientID
	}
	return applicationAuthorizationContext{UserID: u.ID, OrganizationID: org, ExternalSubject: subject, ProviderAudience: audience}, nil
}

func (s *Service) resolveApplicationAuthorizationContext(ctx context.Context, token, org string) (applicationAuthorizationContext, error) {
	org, err := required("organizationId", org)
	if err != nil {
		return applicationAuthorizationContext{}, err
	}
	identity, err := s.resolveExternalIdentity(ctx, token, ReadStrong)
	if err != nil {
		return applicationAuthorizationContext{}, err
	}
	return s.resolveLocalApplicationAuthorizationContext(ctx, identity, org)
}

func (s *Service) resolveLocalApplicationAuthorizationContext(ctx context.Context, identity ExternalIdentity, org string) (applicationAuthorizationContext, error) {
	u, err := s.localIdentity(ctx, identity, ReadStrong)
	if err != nil {
		return applicationAuthorizationContext{}, err
	}
	if !u.Active {
		return applicationAuthorizationContext{}, serviceError(403, "forbidden: local user is inactive")
	}
	if s.cfg.Database.LocalNodeRole == "learner" {
		member, err := s.store.HasActiveOrganizationMembershipWithConsistency(ctx, u.ID, org, ReadStrong)
		if err != nil {
			return applicationAuthorizationContext{}, err
		}
		if !member {
			return applicationAuthorizationContext{}, serviceError(403, "forbidden: local user is not an active member of an active organization")
		}
	}
	var a Application
	if s.AuthProviderMode() == "pocketbase" {
		a, err = s.store.GetApplicationWithConsistency(ctx, s.cfg.Auth.PocketBase.ApplicationID, ReadStrong)
		if err != nil {
			return applicationAuthorizationContext{}, serviceError(403, "forbidden: configured PocketBase application is not ready")
		}
		if a.OrganizationID != org || !a.Active || a.ProvisioningStatus != "ready" {
			return applicationAuthorizationContext{}, serviceError(403, "forbidden: configured PocketBase application is not active for this organization")
		}
	} else if s.AuthProviderMode() == "external-check" {
		if s.cfg.Auth.ExternalCheck == nil {
			return applicationAuthorizationContext{}, serviceError(403, "forbidden: external-check application is not configured")
		}
		a, err = s.store.GetApplicationWithConsistency(ctx, s.cfg.Auth.ExternalCheck.ApplicationID, ReadStrong)
		if err != nil {
			return applicationAuthorizationContext{}, serviceError(403, "forbidden: configured external-check application is not ready")
		}
		if a.OrganizationID != org || !a.Active || a.ProvisioningStatus != "ready" {
			return applicationAuthorizationContext{}, serviceError(403, "forbidden: configured external-check application is not active for this organization")
		}
	} else if s.AuthProviderMode() == "goauthy" {
		if s.cfg.Auth.Goauthy == nil {
			return applicationAuthorizationContext{}, serviceError(503, "Goauthy is not configured")
		}
		a, err = s.store.GetApplicationWithConsistency(ctx, s.cfg.Auth.Goauthy.ApplicationID, ReadStrong)
		if err != nil {
			return applicationAuthorizationContext{}, serviceError(403, "forbidden: configured Goauthy application is not ready")
		}
		if a.OrganizationID != org || !a.Active || a.ProvisioningStatus != "ready" {
			return applicationAuthorizationContext{}, serviceError(403, "forbidden: configured Goauthy application is not active for this organization")
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
	subject := identity.ExternalUserID
	if identity.Provider == "pocketbase" {
		subject = pocketBaseSubject(identity.Issuer, identity.CollectionID, identity.ExternalUserID)
	} else if identity.Provider == "goauthy" {
		subject = goauthySubject(identity.Issuer, identity.ExternalUserID)
	}
	audience := identity.ClientID
	if identity.Provider == "goauthy" {
		audience = a.ID
	}
	return applicationAuthorizationContext{u.ID, org, a.ID, subject, audience}, nil
}
func (s *Service) CheckApplicationAuthorizations(ctx context.Context, token, org string, checks []ApplicationAuthorizationCheck) ([]AuthorizationDecision, error) {
	org, err := required("organizationId", org)
	if err != nil {
		return nil, err
	}
	identity, err := s.resolveApplicationIdentity(ctx, token)
	if err != nil {
		return nil, err
	}
	return localReplicaRead(ctx, s, func(readCtx context.Context) (out []AuthorizationDecision, err error) {
		if s.cfg.Database.LocalNodeRole == "learner" {
			defer func() {
				if readCtx.Err() != nil {
					out = nil
					err = serviceError(503, "unavailable: authorization read budget expired")
				}
			}()
		}
		c, err := s.resolveLocalApplicationAuthorizationContext(readCtx, identity, org)
		if err != nil {
			return nil, err
		}
		exact := make([]AuthorizationCheck, len(checks))
		for i, v := range checks {
			exact[i] = AuthorizationCheck{Subject: AuthorizationSubject{SubjectType: "user", ID: c.UserID, OrganizationID: c.OrganizationID}, Action: v.Action, Resource: v.Resource}
		}
		out, err = s.evaluateAuthorizations(readCtx, exact, ReadStrong, &c.ApplicationID)
		if err != nil {
			return nil, err
		}
		if err := verifyLearnerReadProof(readCtx, s); err != nil {
			return nil, err
		}
		return out, nil
	})
}
