package grant

import (
	"context"
	"github.com/mrchypark/rhiza"
	"strings"
	"time"
)

func validateResponseEvidence(cap HttpCapability, resources []HttpResponseResource, input HttpCapabilityDecisionInput) ([]AuthorizationCheck, error) {
	bad := func(message string) ([]AuthorizationCheck, error) {
		return nil, serviceError(403, "forbidden: "+message)
	}
	p := cap.ResponseAuthorization
	if p == nil || !validDigest(input.ResponseDigest) || input.ResponseLength == 0 || input.ResponseLength > cap.Response.MaxBytes || uint64(len(input.ResponseReferences)) > p.MaxRecords {
		return bad("invalid response evidence bounds")
	}
	digest, err := HttpResponseReferencesDigest(input.ResponseReferences)
	if err != nil {
		return nil, err
	}
	if !validDigest(input.ResponseReferencesDigest) || input.ResponseReferencesDigest != digest {
		return bad("response references digest mismatch")
	}
	requested, err := responseExpansionPaths(cap, resources, input.CanonicalQuery)
	if err != nil {
		return nil, err
	}
	if len(requested) == 0 {
		return bad("response evidence without requested expansion")
	}
	nodes := map[string]HttpResponseResource{}
	for _, node := range resources {
		nodes[node.Key] = node
	}
	paths := make([]string, len(input.ResponseReferences))
	depths := make([]uint64, len(input.ResponseReferences))
	roots := 0
	checks := make([]AuthorizationCheck, 0, len(input.ResponseReferences))
	for i, ref := range input.ResponseReferences {
		node, ok := nodes[ref.ResourceKey]
		if !ok {
			return bad("response resource is unmapped")
		}
		if err := scalarFormat(node.IDFormat, ref.ID); err != nil {
			return nil, err
		}
		if ref.ParentIndex == nil {
			if ref.ResourceKey != p.RootKey || ref.RelationField != "" {
				return bad("invalid response root")
			}
			roots++
			if p.RecordsPointer == "" && ref.ID != input.ResourceID {
				return bad("response root record mismatch")
			}
		} else {
			if *ref.ParentIndex >= uint64(i) {
				return bad("response parent must precede child")
			}
			parent := input.ResponseReferences[*ref.ParentIndex]
			target := ""
			for _, edge := range nodes[parent.ResourceKey].Relations {
				if edge.Field == ref.RelationField {
					target = edge.TargetKey
					break
				}
			}
			if target != ref.ResourceKey {
				return bad("response relation graph mismatch")
			}
			depths[i] = depths[*ref.ParentIndex] + 1
			if depths[i] > p.MaxDepth {
				return bad("response expansion depth exceeded")
			}
			paths[i] = ref.RelationField
			if paths[*ref.ParentIndex] != "" {
				paths[i] = paths[*ref.ParentIndex] + "." + ref.RelationField
			}
			admitted := false
			for _, path := range requested {
				admitted = admitted || path == paths[i] || strings.HasPrefix(path, paths[i]+".")
			}
			if !admitted {
				return bad("response relation was not requested")
			}
		}
		checks = append(checks, AuthorizationCheck{Action: node.ActionID, Resource: AuthorizationResource{ProviderID: input.ProviderID, ResourceType: node.ResourceType, ID: ref.ID}})
	}
	if p.RecordsPointer == "" && roots != 1 {
		return bad("detail response must have one root")
	}
	return checks, nil
}

func (s *Service) responseDecision(ctx context.Context, token, org string, input HttpCapabilityDecisionInput, principal applicationAuthorizationContext) (bool, string, error) {
	read := func(ctx context.Context) (bool, error) {
		pep, err := s.configuredPEP(ctx, input.ProviderID)
		if err != nil {
			return false, err
		}
		catalog, err := s.ActiveHttpCapabilityCatalog(ctx, input.ProviderID)
		if err != nil {
			return false, err
		}
		if pep.PepClientID != input.PEPClientID || catalog.Generation != input.CatalogGeneration || catalog.CatalogDigest != input.CatalogDigest {
			return false, serviceError(403, "forbidden: stale response catalog or PEP")
		}
		current, err := s.resolveApplicationAuthorizationContext(ctx, token, org)
		if err != nil {
			return false, err
		}
		if current.UserID != principal.UserID || current.OrganizationID != principal.OrganizationID || current.ApplicationID != principal.ApplicationID || current.ExternalSubject != principal.ExternalSubject || current.ProviderAudience != principal.ProviderAudience {
			return false, serviceError(403, "forbidden: response principal changed")
		}
		var cap *HttpCapability
		for i := range catalog.Capabilities {
			c := &catalog.Capabilities[i]
			if c.Enabled && c.CapabilityID == input.CapabilityID && c.Version == input.CapabilityVersion {
				cap = c
				break
			}
		}
		if cap == nil || cap.ResponseAuthorization == nil {
			return false, serviceError(403, "forbidden: response capability is inactive")
		}
		if err = validateHTTPRequestEvidence(*cap, input); err != nil {
			return false, err
		}
		checks, err := validateResponseEvidence(*cap, catalog.ResponseResources, input)
		if err != nil {
			return false, err
		}
		if cap.RecordScope != nil {
			// A list's constant resource is not a collection-wide read grant. Check
			// admission and exact root IDs using the existing record-scope evaluator.
			scope, err := s.authorizedRecordScope(ctx, input.ProviderID, *cap, current)
			if err != nil {
				return false, err
			}
			allowed := map[string]bool{}
			for _, id := range scope.IDs {
				allowed[id] = true
			}
			for _, ref := range input.ResponseReferences {
				if ref.ParentIndex == nil && !allowed[ref.ID] {
					return false, nil
				}
			}
		} else {
			checks = append([]AuthorizationCheck{{Action: *cap.ActionID, Resource: AuthorizationResource{ProviderID: input.ProviderID, ResourceType: input.ResourceType, ID: input.ResourceID}}}, checks...)
		}
		for i := range checks {
			checks[i].Subject = AuthorizationSubject{SubjectType: "user", ID: current.UserID, OrganizationID: current.OrganizationID}
		}
		decisions, err := s.evaluateAuthorizations(ctx, checks, ReadStrong, &current.ApplicationID)
		if err != nil {
			return false, err
		}
		if len(decisions) != len(checks) {
			return false, serviceError(503, "incomplete response decision")
		}
		for _, decision := range decisions {
			if !decision.Allowed {
				return false, nil
			}
		}
		return true, nil
	}
	var allowed bool
	var err error
	if readScope(ctx) != nil {
		allowed, err = read(ctx)
	} else {
		if s.store.db == nil {
			return false, "", serviceError(503, "response authorization needs local fenced store")
		}
		ctx, cancel := context.WithTimeout(ctx, 3*time.Second)
		defer cancel()
		fence, e := s.store.db.Query(ctx, rhiza.QueryRequest{SQL: "SELECT 1", Consistency: rhiza.ConsistencyLinearizable})
		if e != nil {
			return false, "", serviceError(503, "response quorum fence failed")
		}
		allowed, err = readAtFence(ctx, s, fence.AppliedSlot, read)
	}
	if err != nil {
		return false, "", err
	}
	if !allowed {
		return false, "response_denied", nil
	}
	return true, "response_allowed", nil
}
