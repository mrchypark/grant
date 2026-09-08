package grant

import (
	"context"
	"fmt"
	"strings"
	"time"
)

func (s *Service) ImportPatchDataAuthority(ctx context.Context, v PatchDataAuthorityImport) (PatchDataAuthorityImportResult, error) {
	return s.store.ImportPatchDataAuthority(ctx, v)
}
func (s *Service) StageHttpCapabilityGenerationAdmin(ctx context.Context, provider string, v StageHttpCapabilityGeneration) (StagedHttpCapabilityGeneration, error) {
	actions, err := serviceRows[Action](ctx, s, ReadStrong, "SELECT * FROM actions WHERE provider_id=?", provider)
	if err != nil {
		return StagedHttpCapabilityGeneration{}, err
	}
	ids := map[string]bool{}
	for _, a := range actions {
		ids[a.ID] = true
	}
	for _, c := range v.Capabilities {
		if c.ActionID != nil && !ids[*c.ActionID] {
			return StagedHttpCapabilityGeneration{}, serviceError(400, fmt.Sprintf("HTTP capability action not found for provider %s: %s", provider, *c.ActionID))
		}
	}
	return s.store.StageHttpCapabilityGeneration(ctx, provider, v)
}
func (s *Service) ActivateHttpCapabilityGenerationAdmin(ctx context.Context, provider string, v ActivateHttpCapabilityGeneration) (HttpCapabilityActivation, error) {
	return s.store.ActivateHttpCapabilityGeneration(ctx, provider, "grant-master", v)
}
func (s *Service) SetProviderPepClientAdmin(ctx context.Context, provider, pep string, v UpsertProviderPepClient) (ProviderPepClient, error) {
	return s.store.UpsertProviderPepClient(ctx, provider, pep, "grant-master", v)
}
func (s *Service) configuredPEP(ctx context.Context, provider string) (ProviderPepClient, error) {
	app := s.cfg.Security.HTTPCapabilityPEPApplicationID
	if app == "" {
		return ProviderPepClient{}, serviceError(503, "HTTP capability PEP application is not configured")
	}
	pep, err := s.store.ProviderPepClientForApplication(ctx, provider, app)
	if err != nil {
		return ProviderPepClient{}, err
	}
	if pep.Lifecycle != "active" {
		return ProviderPepClient{}, serviceError(403, "forbidden: PEP client is not active")
	}
	if pep.ValidUntil != nil {
		expiry, err := time.Parse(time.RFC3339Nano, *pep.ValidUntil)
		if err != nil {
			return ProviderPepClient{}, serviceError(503, "provider PEP expiry is invalid")
		}
		if !expiry.After(time.Now()) {
			return ProviderPepClient{}, serviceError(403, "forbidden: PEP client is expired")
		}
	}
	return pep, nil
}
func (s *Service) ActiveHttpCapabilityCatalog(ctx context.Context, provider string) (ActiveHttpCapabilityCatalog, error) {
	return localReplicaRead(ctx, s, func(ctx context.Context) (ActiveHttpCapabilityCatalog, error) {
		return s.activeHttpCapabilityCatalog(ctx, provider)
	})
}
func (s *Service) activeHttpCapabilityCatalog(ctx context.Context, provider string) (ActiveHttpCapabilityCatalog, error) {
	pep, err := s.configuredPEP(ctx, provider)
	if err != nil {
		return ActiveHttpCapabilityCatalog{}, err
	}
	return s.store.ActiveHttpCapabilityCatalog(ctx, provider, pep.PepClientID)
}
func (s *Service) DecideHttpCapability(ctx context.Context, token, org string, r HttpCapabilityDecisionInput) (HttpCapabilityDecision, error) {
	if s.cfg.Database.LocalNodeRole == "learner" {
		var cancel context.CancelFunc
		ctx, cancel = context.WithTimeout(ctx, 3*time.Second)
		defer cancel()
		identity, e := s.externalIdentity(ctx, token)
		if e != nil {
			return HttpCapabilityDecision{}, e
		}
		ctx = context.WithValue(ctx, replicaIdentityKey{}, identity)
	}
	return localReplicaRead(ctx, s, func(ctx context.Context) (HttpCapabilityDecision, error) {
		return s.decideHttpCapability(ctx, token, org, r)
	})
}
func (s *Service) decideHttpCapability(ctx context.Context, token, org string, r HttpCapabilityDecisionInput) (HttpCapabilityDecision, error) {
	token = strings.TrimSpace(token)
	if token == "" {
		return HttpCapabilityDecision{}, serviceError(401, "unauthorized: missing bearer token")
	}
	pep, err := s.configuredPEP(ctx, r.ProviderID)
	if err != nil {
		return HttpCapabilityDecision{}, err
	}
	if pep.PepClientID != r.PEPClientID {
		return HttpCapabilityDecision{}, serviceError(403, "forbidden: PEP client binding mismatch")
	}
	catalog, err := s.ActiveHttpCapabilityCatalog(ctx, r.ProviderID)
	if err != nil {
		return HttpCapabilityDecision{}, err
	}
	if catalog.Generation != r.CatalogGeneration || catalog.CatalogDigest != r.CatalogDigest {
		return HttpCapabilityDecision{}, serviceError(403, "forbidden: stale HTTP capability catalog")
	}
	var cap *HttpCapability
	for i := range catalog.Capabilities {
		v := &catalog.Capabilities[i]
		if v.Enabled && v.CapabilityID == r.CapabilityID && v.Version == r.CapabilityVersion {
			cap = v
			break
		}
	}
	if cap == nil {
		return HttpCapabilityDecision{}, serviceError(403, "forbidden: HTTP capability is not active")
	}
	if cap.AccessMode != "authorized" {
		return HttpCapabilityDecision{}, serviceError(403, "forbidden: public authentication capability has no authorization decision")
	}
	if err = validateHTTPRequestEvidence(*cap, r); err != nil {
		return HttpCapabilityDecision{}, err
	}
	if CredentialDigest(token) != r.ProviderCredentialDigest {
		return HttpCapabilityDecision{}, serviceError(403, "forbidden: provider credential digest mismatch")
	}
	c, err := s.resolveApplicationAuthorizationContext(ctx, token, org)
	if err != nil {
		return HttpCapabilityDecision{}, err
	}
	if c.ExternalSubject != r.ProviderCredential.Subject || c.ProviderAudience != r.ProviderCredential.Audience {
		return HttpCapabilityDecision{}, serviceError(403, "forbidden: provider credential principal mismatch")
	}
	if cap.ActionID == nil || cap.Resource == nil {
		return HttpCapabilityDecision{}, serviceError(503, "authorized capability action or resource missing")
	}
	decisions, err := s.evaluateAuthorizations(ctx, []AuthorizationCheck{{Subject: AuthorizationSubject{SubjectType: "user", ID: c.UserID, OrganizationID: c.OrganizationID}, Action: *cap.ActionID, Resource: AuthorizationResource{ProviderID: r.ProviderID, ResourceType: cap.Resource.ResourceType, ID: r.ResourceID}}}, ReadStrong, &c.ApplicationID)
	if err != nil {
		return HttpCapabilityDecision{}, err
	}
	now := time.Now().UTC()
	return HttpCapabilityDecision{Allowed: decisions[0].Allowed, Reason: decisions[0].Reason, ProviderID: r.ProviderID, PEPClientID: r.PEPClientID, CatalogGeneration: r.CatalogGeneration, CatalogDigest: r.CatalogDigest, CapabilityID: r.CapabilityID, CapabilityVersion: r.CapabilityVersion, ContractDigest: r.ContractDigest, ActionID: *cap.ActionID, ResourceType: r.ResourceType, ResourceID: r.ResourceID, Method: r.Method, CanonicalPath: r.CanonicalPath, CanonicalQueryDigest: r.CanonicalQueryDigest, ForwardedHeaderDigest: r.ForwardedHeaderDigest, BodyDigest: r.BodyDigest, BodyLength: r.BodyLength, ProviderCredentialDigest: r.ProviderCredentialDigest, ProviderSubject: c.ExternalSubject, ProviderAudience: c.ProviderAudience, RequestBinding: r.RequestBinding, PrincipalDigest: ExposurePrincipalDigest(c.UserID, c.OrganizationID, c.ApplicationID), IssuedAt: now.Format(time.RFC3339Nano), ExpiresAt: now.Add(5 * time.Second).Format(time.RFC3339Nano)}, nil
}
