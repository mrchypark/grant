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
// not request data or an editable role. Application admission and consent apply.
func (s *Service) AuthorizePlatformManagement(ctx context.Context, token, org, actionCode string) (string, error) {
	if s.cfg.Security.ManagementOrganizationID == "" {
		return "", serviceError(503, "management bearer auth is not configured")
	}
	if org != s.cfg.Security.ManagementOrganizationID {
		return "", serviceError(403, "forbidden: platform management organization required")
	}
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
