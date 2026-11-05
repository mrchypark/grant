package grant

import "context"

type managementActorKey struct{}

func WithManagementActor(ctx context.Context, userID string) context.Context {
	return context.WithValue(ctx, managementActorKey{}, userID)
}

func ManagementActorFromContext(ctx context.Context) string {
	if actor, ok := ctx.Value(managementActorKey{}).(string); ok && actor != "" {
		return actor
	}
	return "grant-master"
}

// AuthorizePlatformManagement keeps tenant role administrators from granting
// themselves platform access. The trusted operator organization is configuration,
// not request data or an editable role. The server PEP policy selects user or
// application authorization; requests cannot select the mode.
func (s *Service) AuthorizePlatformManagement(ctx context.Context, token, org, actionCode string) (string, error) {
	if s.cfg.Security.ManagementOrganizationID == "" {
		return "", serviceError(503, "management bearer auth is not configured")
	}
	if org != s.cfg.Security.ManagementOrganizationID {
		return "", serviceError(403, "forbidden: platform management organization required")
	}
	if s.cfg.Security.HTTPCapabilityPEPClientID == "" {
		return s.authorizePlatformManagementApplication(ctx, token, org, actionCode)
	}
	policy, err := s.store.ProviderPepClientAdmin(ctx, "grant-management", s.cfg.Security.HTTPCapabilityPEPClientID)
	if err != nil {
		if ErrorStatus(err) == 404 {
			return s.authorizePlatformManagementApplication(ctx, token, org, actionCode)
		}
		return "", err
	}
	switch policy.AuthorizationMode {
	case "", "application":
		return s.authorizePlatformManagementApplication(ctx, token, org, actionCode)
	case "user":
		// User authorization requires a fresh active policy in the fenced read.
	default:
		return "", serviceError(503, "unavailable: unsupported PEP policy")
	}
	identity, err := s.externalIdentity(ctx, token)
	if err != nil {
		return "", err
	}
	ctx = context.WithValue(ctx, replicaIdentityKey{}, identity)
	return capabilityPolicyRead(ctx, s, "grant-management", 0, func(ctx context.Context) (string, error) {
		pep, err := s.configuredPEP(ctx, "grant-management")
		if err != nil {
			return "", err
		}
		switch pep.AuthorizationMode {
		case "", "application":
			return s.authorizePlatformManagementApplication(ctx, token, org, actionCode)
		case "user":
			if pep.PolicyRevision <= 0 {
				return "", serviceError(503, "unavailable: invalid PEP policy")
			}
		default:
			return "", serviceError(503, "unavailable: unsupported PEP policy")
		}
		principal, err := s.resolveLocalUserAuthorizationContext(ctx, identity, org)
		if err != nil {
			return "", err
		}
		decisions, err := s.evaluateAuthorizations(ctx, []AuthorizationCheck{{
			Subject:  AuthorizationSubject{SubjectType: "user", ID: principal.UserID, OrganizationID: org},
			Action:   "grant-management." + actionCode,
			Resource: AuthorizationResource{ProviderID: "grant-management", ResourceType: "platform", ID: "grant"},
		}}, ReadStrong, nil)
		if err != nil {
			return "", err
		}
		if len(decisions) != 1 || !decisions[0].Allowed {
			return "", serviceError(403, "forbidden: platform management action denied")
		}
		return principal.UserID, nil
	})
}

func (s *Service) authorizePlatformManagementApplication(ctx context.Context, token, org, actionCode string) (string, error) {
	principal, err := s.Authenticate(ctx, token, org, ReadStrong)
	if err != nil {
		return "", err
	}
	decisions, err := s.CheckApplicationAuthorizations(ctx, token, org, []ApplicationAuthorizationCheck{{
		Action:   "grant-management." + actionCode,
		Resource: AuthorizationResource{ProviderID: "grant-management", ResourceType: "platform", ID: "grant"},
	}})
	if err != nil {
		return "", err
	}
	if len(decisions) != 1 || !decisions[0].Allowed {
		return "", serviceError(403, "forbidden: platform management action denied")
	}
	return principal.UserID, nil
}

// This exception only removes the inactive-organization gate for the exact
// reactivation request. Identity, membership, binding and app checks still run.
func managementReactivationCapability(provider string, cap HttpCapability, org, resourceID, path string) bool {
	return provider == "grant-management" && org != "" && resourceID == org &&
		cap.Method == "POST" && cap.PathTemplate == "/api/v1/orgs/{organizationId}/reactivate" &&
		path == "/api/v1/orgs/"+org+"/reactivate" &&
		cap.ActionID != nil && *cap.ActionID == "grant-management.organization.reactivate" &&
		cap.Resource != nil && cap.Resource.ResourceType == "organization" &&
		cap.Resource.ID.Kind == "pathSegment" && cap.Resource.ID.Name == "organizationId" &&
		cap.RecordScope == nil && cap.Batch == nil && cap.ResponseAuthorization == nil
}

// managementEnsureCapability matches the exact POST /api/v1/me/ensure
// capability for auth-only self provisioning. It requires active org/provider and
// explicit application_actions admission but no membership/role/consent.
func managementEnsureCapability(provider string, cap HttpCapability) bool {
	return provider == "grant-management" &&
		cap.Method == "POST" && cap.PathTemplate == "/api/v1/me/ensure" &&
		cap.ActionID != nil && *cap.ActionID == "grant-management.user.materialize" &&
		cap.Resource != nil && cap.Resource.ResourceType == "service" &&
		cap.Resource.ID.Kind == "constant" && cap.Resource.ID.Value == "grant" &&
		cap.Request.Body == "none" && cap.Request.Authorization == "forward-user" &&
		cap.RecordScope == nil && cap.Batch == nil && cap.ResponseAuthorization == nil
}
