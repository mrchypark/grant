package grant

import (
	"context"
	"strings"
	"time"
)

func normalizeLookupResourcesInput(subject AuthorizationSubject, providerID, resourceType, actionID string, limit *int, cursor *string, consistency ReadConsistency) (LookupResourcesInput, error) {
	if subject.SubjectType != "user" {
		return LookupResourcesInput{}, serviceError(400, "only user authz subjects are supported")
	}
	userID, err := required("subject.id", subject.ID)
	if err != nil {
		return LookupResourcesInput{}, err
	}
	orgID, err := required("subject.orgId", subject.OrganizationID)
	if err != nil {
		return LookupResourcesInput{}, err
	}
	provider, err := required("resource.provider", providerID)
	if err != nil {
		return LookupResourcesInput{}, err
	}
	resType, err := required("resource.type", resourceType)
	if err != nil {
		return LookupResourcesInput{}, err
	}
	action, err := required("action", actionID)
	if err != nil {
		return LookupResourcesInput{}, err
	}
	resolvedLimit := 100
	if limit != nil {
		if *limit < 1 || *limit > 1000 {
			return LookupResourcesInput{}, serviceError(400, "limit must be between 1 and 1000")
		}
		resolvedLimit = *limit
	}
	var normalizedCursor string
	if cursor != nil {
		normalizedCursor = strings.TrimSpace(*cursor)
	}
	return LookupResourcesInput{
		UserID:         userID,
		OrganizationID: orgID,
		ProviderID:     provider,
		ResourceType:   resType,
		ActionID:       action,
		Limit:          resolvedLimit,
		Cursor:         normalizedCursor,
		Consistency:    consistency,
	}, nil
}

func verifyLearnerReadProof(ctx context.Context, s *Service) error {
	if s.cfg.Database.LocalNodeRole != "learner" {
		return nil
	}
	if _, err := s.store.Query(ctx, ReadStrong, "SELECT 1"); err != nil {
		return err
	}
	scope := readScope(ctx)
	if ctx.Err() != nil || scope == nil || !scope.pinned || scope.changed || scope.slot < scope.fence {
		return serviceError(503, "unavailable: replica authorization read proof expired")
	}
	return nil
}

func (s *Service) LookupResources(ctx context.Context, subject AuthorizationSubject, providerID, resourceType, actionID string, limit *int, cursor *string, consistency ReadConsistency) (LookupResourcesResult, error) {
	in, err := normalizeLookupResourcesInput(subject, providerID, resourceType, actionID, limit, cursor, consistency)
	if err != nil {
		return LookupResourcesResult{}, err
	}
	return localReplicaRead(ctx, s, func(readCtx context.Context) (out LookupResourcesResult, err error) {
		if s.cfg.Database.LocalNodeRole == "learner" {
			defer func() {
				if readCtx.Err() != nil {
					out = LookupResourcesResult{}
					err = serviceError(503, "unavailable: authorization read budget expired")
				}
			}()
		}
		out, err = s.store.LookupResources(readCtx, in)
		if err != nil {
			return LookupResourcesResult{}, err
		}
		if err := verifyLearnerReadProof(readCtx, s); err != nil {
			return LookupResourcesResult{}, err
		}
		return out, nil
	})
}

func (s *Service) resolveApplicationIdentity(ctx context.Context, token string) (ExternalIdentity, error) {
	if s.cfg.Database.LocalNodeRole == "learner" {
		// No replicated cache lookup/write or inner Rauthy memory-cache hit:
		// this request needs fresh external evidence before its local fence.
		if s.AuthProviderMode() == "rauthy" {
			return s.rauthyIdentityWithCache(ctx, token, false)
		}
		return s.externalIdentity(ctx, token)
	}
	return s.resolveExternalIdentity(ctx, token, ReadStrong)
}

func (s *Service) LookupApplicationResources(ctx context.Context, token, org, providerID, resourceType, actionID string, limit *int, cursor *string) (LookupResourcesResult, error) {
	org, err := required("organizationId", org)
	if err != nil {
		return LookupResourcesResult{}, err
	}
	in, err := normalizeLookupResourcesInput(
		AuthorizationSubject{SubjectType: "user", ID: "placeholder", OrganizationID: org},
		providerID,
		resourceType,
		actionID,
		limit,
		cursor,
		ReadStrong,
	)
	if err != nil {
		return LookupResourcesResult{}, err
	}
	identity, err := s.resolveApplicationIdentity(ctx, token)
	if err != nil {
		return LookupResourcesResult{}, err
	}
	return localReplicaRead(ctx, s, func(readCtx context.Context) (out LookupResourcesResult, err error) {
		if s.cfg.Database.LocalNodeRole == "learner" {
			defer func() {
				if readCtx.Err() != nil {
					out = LookupResourcesResult{}
					err = serviceError(503, "unavailable: authorization read budget expired")
				}
			}()
		}
		c, err := s.resolveLocalApplicationAuthorizationContext(readCtx, identity, org)
		if err != nil {
			return LookupResourcesResult{}, err
		}
		in.UserID = c.UserID
		in.OrganizationID = c.OrganizationID
		in.ApplicationID = c.ApplicationID
		out, err = s.store.LookupResources(readCtx, in)
		if err != nil {
			return LookupResourcesResult{}, err
		}
		if err := verifyLearnerReadProof(readCtx, s); err != nil {
			return LookupResourcesResult{}, err
		}
		return out, nil
	})
}

func (s *Service) UserLookupResources(ctx context.Context, bearer, org, provider, resourceType, actionID string, limit *int, cursor *string) (UserLookupResourcesResult, error) {
	if s.AuthProviderMode() != "goauthy" || s.cfg.Auth.Goauthy == nil {
		return UserLookupResourcesResult{}, serviceError(403, "forbidden: Goauthy user authentication required")
	}
	org, err := required("organizationId", org)
	if err != nil {
		return UserLookupResourcesResult{}, err
	}
	in, err := normalizeLookupResourcesInput(
		AuthorizationSubject{SubjectType: "user", ID: "placeholder", OrganizationID: org},
		provider,
		resourceType,
		actionID,
		limit,
		cursor,
		ReadStrong,
	)
	if err != nil {
		return UserLookupResourcesResult{}, err
	}
	identity, err := s.externalIdentity(ctx, bearer)
	if err != nil {
		return UserLookupResourcesResult{}, err
	}
	ctx = context.WithValue(ctx, replicaIdentityKey{}, identity)
	return capabilityAuthorizationRead(ctx, s, func(ctx context.Context) (out UserLookupResourcesResult, err error) {
		defer func() {
			if ctx.Err() != nil {
				out = UserLookupResourcesResult{}
				err = serviceError(503, "unavailable: user authorization read budget expired")
			}
		}()
		pep, err := s.configuredPEP(ctx, in.ProviderID)
		if err != nil {
			return UserLookupResourcesResult{}, err
		}
		if pep.AuthorizationMode != "user" {
			return UserLookupResourcesResult{}, serviceError(403, "forbidden: explicit user PEP policy required")
		}
		if pep.PolicyRevision <= 0 {
			return UserLookupResourcesResult{}, serviceError(503, "unavailable: invalid PEP policy revision")
		}
		principal, err := s.resolveLocalUserAuthorizationContext(ctx, identity, org)
		if err != nil {
			return UserLookupResourcesResult{}, err
		}
		action, err := serviceRow[Action](ctx, s, ReadStrong, "SELECT id,provider_id FROM actions WHERE id=?", in.ActionID)
		if err != nil {
			return UserLookupResourcesResult{}, err
		}
		if action.ProviderID != in.ProviderID {
			return UserLookupResourcesResult{}, serviceError(400, "action provider must match path provider")
		}
		in.UserID = principal.UserID
		in.OrganizationID = principal.OrganizationID
		res, err := s.store.LookupResources(ctx, in)
		if err != nil {
			return UserLookupResourcesResult{}, err
		}
		if _, err := s.store.Query(ctx, ReadStrong, "SELECT 1"); err != nil {
			return UserLookupResourcesResult{}, err
		}
		scope := readScope(ctx)
		if ctx.Err() != nil || scope == nil || !scope.pinned || scope.changed || scope.slot < scope.fence {
			return UserLookupResourcesResult{}, serviceError(503, "unavailable: user authorization read proof expired")
		}
		if err := requireActivePEP(pep, time.Now()); err != nil {
			return UserLookupResourcesResult{}, err
		}
		return UserLookupResourcesResult{
			ResourceIDs:       res.ResourceIDs,
			NextCursor:        res.NextCursor,
			PEPPolicyRevision: pep.PolicyRevision,
		}, nil
	})
}
