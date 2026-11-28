package grant

import (
	"context"
	"strings"
	"time"
)

// These are observations of current user authority, not reusable effect permits.
type UserBatchCheckResult struct {
	Results           []AuthorizationDecision `json:"results"`
	PEPPolicyRevision int64                   `json:"pepPolicyRevision"`
}

type UserFilterResult struct {
	ResourceIDs       []string `json:"resource_ids"`
	PEPPolicyRevision int64    `json:"pepPolicyRevision"`
}

// UserBatchCheck derives the only subject from fresh online Goauthy evidence.
// It deliberately does not consult application admission, consent or a catalog.
func (s *Service) UserBatchCheck(ctx context.Context, bearer, org, provider string, checks []ApplicationAuthorizationCheck) (UserBatchCheckResult, error) {
	return s.userAuthorizationRead(ctx, bearer, org, provider, checks, "")
}

func (s *Service) userAuthorizationRead(ctx context.Context, bearer, org, provider string, checks []ApplicationAuthorizationCheck, filterAction string) (UserBatchCheckResult, error) {
	if len(checks) > 1000 {
		return UserBatchCheckResult{}, serviceError(400, "checks must contain at most 1000 items")
	}
	if strings.TrimSpace(provider) == "" || strings.TrimSpace(org) == "" {
		return UserBatchCheckResult{}, serviceError(400, "provider and organization are required")
	}
	for _, check := range checks {
		if check.Resource.ProviderID != provider {
			return UserBatchCheckResult{}, serviceError(400, "resource provider must match path provider")
		}
		for _, value := range []string{check.Action, check.Resource.ResourceType, check.Resource.ID} {
			if strings.TrimSpace(value) == "" {
				return UserBatchCheckResult{}, serviceError(400, "action and resource type and id are required")
			}
		}
	}
	if s.AuthProviderMode() != "goauthy" || s.cfg.Auth.Goauthy == nil {
		return UserBatchCheckResult{}, serviceError(403, "forbidden: Goauthy user authentication required")
	}
	// Bypass identity caches and perform introspection/UserInfo once, outside
	// the three-second local retry budget. The caller supplies the original header.
	identity, ok := ctx.Value(trustedGatewayIdentityKey{}).(ExternalIdentity)
	if !ok {
		var err error
		identity, err = s.externalIdentity(ctx, bearer)
		if err != nil {
			return UserBatchCheckResult{}, err
		}
	}
	ctx = context.WithValue(ctx, replicaIdentityKey{}, identity)
	return capabilityAuthorizationRead(ctx, s, func(ctx context.Context) (out UserBatchCheckResult, err error) {
		defer func() {
			if ctx.Err() != nil {
				out = UserBatchCheckResult{}
				err = serviceError(503, "unavailable: user authorization read budget expired")
			}
		}()
		pep, err := s.configuredPEP(ctx, provider)
		if err != nil {
			return UserBatchCheckResult{}, err
		}
		if pep.AuthorizationMode != "user" {
			return UserBatchCheckResult{}, serviceError(403, "forbidden: explicit user PEP policy required")
		}
		if pep.PolicyRevision <= 0 {
			return UserBatchCheckResult{}, serviceError(503, "unavailable: invalid PEP policy revision")
		}
		principal, err := s.resolveLocalUserAuthorizationContext(ctx, identity, org)
		if err != nil {
			return UserBatchCheckResult{}, err
		}
		exact := make([]AuthorizationCheck, len(checks))
		actionIDs := make([]string, 0, len(checks)+1)
		if filterAction != "" {
			actionIDs = append(actionIDs, filterAction)
		}
		for i, check := range checks {
			actionIDs = append(actionIDs, check.Action)
			exact[i] = AuthorizationCheck{Subject: AuthorizationSubject{SubjectType: "user", ID: principal.UserID, OrganizationID: principal.OrganizationID}, Action: check.Action, Resource: check.Resource}
		}
		if err := s.validateProviderActions(ctx, provider, actionIDs); err != nil {
			return UserBatchCheckResult{}, err
		}
		results, err := s.evaluateAuthorizations(ctx, exact, ReadStrong, nil)
		if err != nil {
			return UserBatchCheckResult{}, err
		}
		// A final pinned read detects a write after the last tuple, including an
		// empty batch. Never return a result after the local budget or PEP expiry.
		if _, err := s.store.Query(ctx, ReadStrong, "SELECT 1"); err != nil {
			return UserBatchCheckResult{}, err
		}
		scope := readScope(ctx)
		if ctx.Err() != nil || scope == nil || !scope.pinned || scope.changed || scope.slot < scope.fence {
			return UserBatchCheckResult{}, serviceError(503, "unavailable: user authorization read proof expired")
		}
		if err := requireActivePEP(pep, time.Now()); err != nil {
			return UserBatchCheckResult{}, err
		}
		return UserBatchCheckResult{Results: results, PEPPolicyRevision: pep.PolicyRevision}, nil
	})
}

func (s *Service) validateProviderActions(ctx context.Context, provider string, actionIDs []string) error {
	if len(actionIDs) == 0 {
		return nil
	}
	seen := make(map[string]struct{}, len(actionIDs))
	unique := make([]string, 0, len(actionIDs))
	for _, raw := range actionIDs {
		id := strings.TrimSpace(raw)
		if _, ok := seen[id]; ok {
			continue
		}
		seen[id] = struct{}{}
		unique = append(unique, id)
	}
	const chunkSize = 200
	for start := 0; start < len(unique); start += chunkSize {
		end := min(start+chunkSize, len(unique))
		chunk := unique[start:end]
		placeholders := make([]string, len(chunk))
		args := make([]any, len(chunk))
		for i, id := range chunk {
			placeholders[i] = "?"
			args[i] = id
		}
		rows, err := serviceRows[Action](ctx, s, ReadStrong, "SELECT id,provider_id FROM actions WHERE id IN ("+strings.Join(placeholders, ",")+")", args...)
		if err != nil {
			return err
		}
		found := make(map[string]string, len(rows))
		for _, a := range rows {
			found[a.ID] = a.ProviderID
		}
		for _, id := range chunk {
			pid, ok := found[id]
			if !ok {
				return serviceError(404, "not found")
			}
			if pid != provider {
				return serviceError(400, "action provider must match path provider")
			}
		}
	}
	return nil
}

// UserFilter filters only supplied candidates. Bare IDs require one resource
// type within the path provider; this endpoint does not discover resources.
func (s *Service) UserFilter(ctx context.Context, bearer, org, provider, action string, resources []AuthorizationResource) (UserFilterResult, error) {
	if len(resources) > 1000 {
		return UserFilterResult{}, serviceError(400, "resources must contain at most 1000 items")
	}
	if strings.TrimSpace(action) == "" {
		return UserFilterResult{}, serviceError(400, "action is required")
	}
	checks := make([]ApplicationAuthorizationCheck, len(resources))
	for i, resource := range resources {
		if i > 0 && strings.TrimSpace(resource.ResourceType) != strings.TrimSpace(resources[0].ResourceType) {
			return UserFilterResult{}, serviceError(400, "filter resources must use one resource type")
		}
		checks[i] = ApplicationAuthorizationCheck{Action: action, Resource: resource}
	}
	// Even no candidates must authenticate and obtain current fenced policy.
	// Validate the filter action inside that same snapshot for empty filters too.
	batch, err := s.userAuthorizationRead(ctx, bearer, org, provider, checks, action)
	if err != nil {
		return UserFilterResult{}, err
	}
	ids := []string{}
	for i, resource := range resources {
		if batch.Results[i].Allowed {
			ids = append(ids, resource.ID)
		}
	}
	return UserFilterResult{ResourceIDs: ids, PEPPolicyRevision: batch.PEPPolicyRevision}, nil
}
