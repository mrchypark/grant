package grant

import (
	"context"
	"github.com/mrchypark/rhiza"
	"time"
)

func (s *Service) recordScopeDecision(ctx context.Context, provider string, cap HttpCapability, principal applicationAuthorizationContext, generation uint64, digest, pepID string) (*HttpRecordScope, error) {
	read := func(ctx context.Context) (*HttpRecordScope, error) {
		catalog, e := s.ActiveHttpCapabilityCatalog(ctx, provider)
		if e != nil {
			return nil, e
		}
		if catalog.Generation != generation || catalog.CatalogDigest != digest || catalog.PepClientID != pepID {
			return nil, serviceError(403, "forbidden: stale HTTP capability catalog")
		}
		return s.authorizedRecordScope(ctx, provider, cap, principal)
	}
	if readScope(ctx) != nil {
		return read(ctx)
	}
	if s.store.db == nil {
		return nil, serviceError(503, "record scope requires a local fenced authority store")
	}
	ctx, cancel := context.WithTimeout(ctx, 3*time.Second)
	defer cancel()
	fence, e := s.store.db.Query(ctx, rhiza.QueryRequest{SQL: "SELECT 1", Consistency: rhiza.ConsistencyLinearizable})
	if e != nil {
		return nil, serviceError(503, "record scope quorum fence failed")
	}
	return readAtFence(ctx, s, fence.AppliedSlot, read)
}

// Candidates are a bounded superset. The existing evaluator remains the only
// authority for membership, role, agreement, application scope and consent.
func (s *Service) authorizedRecordScope(ctx context.Context, provider string, cap HttpCapability, principal applicationAuthorizationContext) (*HttpRecordScope, error) {
	member, e := s.store.HasActiveOrganizationMembershipWithConsistency(ctx, principal.UserID, principal.OrganizationID, ReadStrong)
	if e != nil {
		return nil, e
	}
	if !member {
		return nil, serviceError(403, "forbidden: active organization membership required")
	}
	check := UserActionCheck{UserID: principal.UserID, OrganizationID: principal.OrganizationID, ProviderID: provider, ResourceType: cap.Resource.ResourceType, ResourceID: cap.Resource.ID.Value, ActionID: *cap.ActionID, ApplicationID: &principal.ApplicationID, Consistency: ReadStrong}
	admission, e := s.store.EvaluateUserActions(ctx, []UserActionCheck{check})
	if e != nil {
		return nil, e
	}
	if len(admission) != 1 || !admission[0].ApplicationAllowed {
		return nil, serviceError(403, "forbidden: application action scope or consent is missing")
	}
	const maxCandidates = 1000
	rows, e := s.store.Query(ctx, ReadStrong, "SELECT DISTINCT resource_id FROM role_bindings WHERE provider_id=? AND resource_type=? AND subject_organization_id=? AND state='active' ORDER BY resource_id LIMIT ?", provider, cap.Resource.ResourceType, principal.OrganizationID, maxCandidates+1)
	if e != nil {
		return nil, e
	}
	if len(rows) > maxCandidates {
		return nil, serviceError(503, "record scope candidate limit exceeded")
	}
	checks := make([]UserActionCheck, 0, len(rows))
	for _, row := range rows {
		id := rowString(row, "resource_id")
		if id == "" || boundedUTF8(id, 256) != nil {
			return nil, serviceError(503, "record scope has an unsupported resource identifier")
		}
		v := check
		v.ResourceID = id
		checks = append(checks, v)
	}
	decisions, e := s.store.EvaluateUserActions(ctx, checks)
	if e != nil {
		return nil, e
	}
	scope := &HttpRecordScope{IDs: []string{}}
	for i, d := range decisions {
		if d.Allowed {
			scope.IDs = append(scope.IDs, checks[i].ResourceID)
			if uint64(len(scope.IDs)) > cap.RecordScope.MaxIDs {
				return nil, serviceError(503, "record scope authorized identifier limit exceeded")
			}
		}
	}
	return scope, nil
}
