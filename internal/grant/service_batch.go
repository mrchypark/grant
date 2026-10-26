package grant

import (
	"context"
	"github.com/mrchypark/rhiza"
	"strings"
	"time"
)

func batchChildInput(root HttpCapabilityDecisionInput, child HttpBatchChild) HttpCapabilityDecisionInput {
	root.CapabilityID, root.CapabilityVersion, root.ContractDigest = child.CapabilityID, child.CapabilityVersion, child.ContractDigest
	root.Method, root.CanonicalPath, root.CanonicalQuery = child.Method, child.CanonicalPath, child.CanonicalQuery
	root.CanonicalQueryDigest, root.ForwardedHeaderDigest = child.CanonicalQueryDigest, child.ForwardedHeaderDigest
	root.BodyDigest, root.BodyLength = child.BodyDigest, child.BodyLength
	root.ResourceEvidence, root.ResourceType, root.ResourceID = child.ResourceEvidence, child.ResourceType, child.ResourceID
	root.BatchChildren, root.BatchDigest = nil, ""
	return root
}

// The trusted PEP describes interpreted JSON children. The outer request hash
// binds original forwarded bytes, including indexed multipart attachments.
func validateBatchEvidence(root HttpCapability, catalog ActiveHttpCapabilityCatalog, input HttpCapabilityDecisionInput) ([]AuthorizationCheck, error) {
	bad := func(message string) ([]AuthorizationCheck, error) {
		return nil, serviceError(403, "forbidden: "+message)
	}
	p := root.Batch
	if p == nil || len(input.BatchChildren) == 0 || len(input.BatchChildren) > int(p.MaxOperations)*2 {
		return bad("invalid batch child count")
	}
	digest, err := HttpBatchChildrenDigest(input.BatchChildren)
	if err != nil {
		return nil, err
	}
	if !validDigest(input.BatchDigest) || digest != input.BatchDigest {
		return bad("batch digest mismatch")
	}
	allowed := map[string]bool{}
	for _, id := range p.AllowedCapabilityIDs {
		allowed[id] = true
	}
	byID := map[string]HttpCapability{}
	for _, cap := range catalog.Capabilities {
		if cap.Enabled {
			byID[cap.CapabilityID] = cap
		}
	}
	checks := make([]AuthorizationCheck, 0, len(input.BatchChildren))
	for _, child := range input.BatchChildren {
		cap, ok := byID[child.CapabilityID]
		if !ok || !allowed[child.CapabilityID] || cap.Batch != nil || cap.RecordScope != nil || cap.Version != child.CapabilityVersion {
			return bad("batch child is not eligible")
		}
		if child.CanonicalQuery != "" || child.ForwardedHeaderDigest != digestBytes(nil) {
			return bad("batch child queries or headers are unsupported")
		}
		if err := validateHTTPRequestEvidence(cap, batchChildInput(input, child)); err != nil {
			return nil, err
		}
		checks = append(checks, AuthorizationCheck{Action: *cap.ActionID, Resource: AuthorizationResource{ProviderID: input.ProviderID, ResourceType: child.ResourceType, ID: child.ResourceID}})
	}
	operation := uint64(0)
	for pos := 0; pos < len(input.BatchChildren); {
		first := input.BatchChildren[pos]
		if first.Index != operation || operation >= p.MaxOperations {
			return bad("batch indices are not contiguous")
		}
		end := pos + 1
		for end < len(input.BatchChildren) && input.BatchChildren[end].Index == first.Index {
			end++
		}
		var upsert *HttpBatchUpsert
		for i := range p.Upserts {
			u := &p.Upserts[i]
			if u.Method == first.OriginalMethod && u.PathTemplate == first.OriginalPath {
				upsert = u
				break
			}
		}
		if upsert == nil {
			if end-pos != 1 || first.UpsertID != "" || first.OriginalMethod != first.Method || first.OriginalPath != first.CanonicalPath {
				return bad("undeclared batch transformation")
			}
		} else {
			if first.CapabilityID != upsert.CreateCapabilityID || first.Method != "POST" || first.CanonicalPath != upsert.PathTemplate {
				return bad("upsert create check missing")
			}
			if first.UpsertID == "" {
				if end-pos != 1 {
					return bad("upsert without id has extra checks")
				}
			} else {
				if end-pos != 2 {
					return bad("upsert with id needs both checks")
				}
				second := input.BatchChildren[pos+1]
				update := byID[upsert.UpdateCapabilityID]
				path := strings.Replace(update.PathTemplate, "{"+update.Resource.ID.Name+"}", first.UpsertID, 1)
				if second.CapabilityID != upsert.UpdateCapabilityID || second.Method != "PATCH" || second.OriginalMethod != first.OriginalMethod || second.OriginalPath != first.OriginalPath || second.UpsertID != first.UpsertID || second.ResourceID != first.UpsertID || second.CanonicalPath != path || second.BodyDigest != first.BodyDigest || second.BodyLength != first.BodyLength {
					return bad("upsert check pair mismatch")
				}
			}
		}
		pos = end
		operation++
	}
	return checks, nil
}

func (s *Service) batchDecision(ctx context.Context, token, org string, input HttpCapabilityDecisionInput, principal applicationAuthorizationContext) (bool, string, error) {
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
			return false, serviceError(403, "forbidden: stale batch catalog or PEP")
		}
		var current applicationAuthorizationContext
		if org == "" {
			current, err = s.resolveOrglessApplicationContext(ctx, token)
		} else {
			current, err = s.resolveApplicationAuthorizationContext(ctx, token, org)
		}
		if err != nil {
			return false, err
		}
		if current.UserID != principal.UserID || current.OrganizationID != principal.OrganizationID || current.ApplicationID != principal.ApplicationID || current.ExternalSubject != principal.ExternalSubject || current.ProviderAudience != principal.ProviderAudience {
			return false, serviceError(403, "forbidden: batch principal changed")
		}
		var root *HttpCapability
		for i := range catalog.Capabilities {
			cap := &catalog.Capabilities[i]
			if cap.Enabled && cap.CapabilityID == input.CapabilityID && cap.Version == input.CapabilityVersion {
				root = cap
				break
			}
		}
		if root == nil || root.Batch == nil {
			return false, serviceError(403, "forbidden: batch capability inactive")
		}
		if err = validateHTTPRequestEvidence(*root, input); err != nil {
			return false, err
		}
		checks, err := validateBatchEvidence(*root, catalog, input)
		if err != nil {
			return false, err
		}
		checks = append([]AuthorizationCheck{{Action: *root.ActionID, Resource: AuthorizationResource{ProviderID: input.ProviderID, ResourceType: input.ResourceType, ID: input.ResourceID}}}, checks...)
		if org == "" {
			if root.OrganizationContext != "optional" {
				return false, serviceError(403, "batch requires organization")
			}
			for _, check := range checks {
				allowed, e := s.orglessActionAllowed(ctx, current, check.Resource.ProviderID, check.Action, check.Resource.ResourceType, check.Resource.ID)
				if e != nil || !allowed {
					return false, e
				}
			}
			return true, nil
		}
		for i := range checks {
			checks[i].Subject = AuthorizationSubject{SubjectType: "user", ID: current.UserID, OrganizationID: current.OrganizationID}
		}
		decisions, err := s.evaluateAuthorizations(ctx, checks, ReadStrong, &current.ApplicationID)
		if err != nil {
			return false, err
		}
		if len(decisions) != len(checks) {
			return false, serviceError(503, "incomplete batch decision")
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
			return false, "", serviceError(503, "batch needs local fenced store")
		}
		ctx, cancel := context.WithTimeout(ctx, 3*time.Second)
		defer cancel()
		fence, e := s.store.db.Query(ctx, rhiza.QueryRequest{SQL: "SELECT 1", Consistency: rhiza.ConsistencyLinearizable})
		if e != nil {
			return false, "", serviceError(503, "batch quorum fence failed")
		}
		allowed, err = readAtFence(ctx, s, fence.AppliedSlot, read)
	}
	if err != nil {
		return false, "", err
	}
	if !allowed {
		return false, "batch_denied", nil
	}
	return true, "batch_allowed", nil
}
