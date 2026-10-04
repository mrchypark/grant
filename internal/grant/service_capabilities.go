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
	for _, node := range v.ResponseResources {
		if !ids[node.ActionID] {
			return StagedHttpCapabilityGeneration{}, serviceError(400, "response resource action not found for provider")
		}
	}
	return s.store.StageHttpCapabilityGeneration(ctx, provider, v)
}
func (s *Service) ActivateHttpCapabilityGenerationAdmin(ctx context.Context, provider string, v ActivateHttpCapabilityGeneration) (HttpCapabilityActivation, error) {
	return s.store.ActivateHttpCapabilityGeneration(ctx, provider, ManagementActorFromContext(ctx), v)
}
func (s *Service) ActiveHttpCapabilityCatalogAdmin(ctx context.Context, provider string) (AdminHttpCapabilityCatalog, error) {
	return s.store.ActiveHttpCapabilityCatalogAdmin(ctx, provider)
}
func (s *Service) SetProviderPepClientAdmin(ctx context.Context, provider, pep string, v UpsertProviderPepClient) (ProviderPepClient, error) {
	return s.store.UpsertProviderPepClient(ctx, provider, pep, ManagementActorFromContext(ctx), v)
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
	if cap.Batch == nil && (len(r.BatchChildren) != 0 || r.BatchDigest != "") {
		return HttpCapabilityDecision{}, serviceError(403, "forbidden: unexpected batch evidence")
	}
	if cap.ResponseAuthorization == nil && (r.ResponseDigest != "" || r.ResponseLength != 0 || r.ResponseReferencesDigest != "" || len(r.ResponseReferences) != 0) {
		return HttpCapabilityDecision{}, serviceError(403, "forbidden: unexpected response evidence")
	}
	if _, err = responseExpansionPaths(*cap, catalog.ResponseResources, r.CanonicalQuery); err != nil {
		return HttpCapabilityDecision{}, err
	}
	if err = validateHTTPRequestEvidence(*cap, r); err != nil {
		return HttpCapabilityDecision{}, err
	}
	if CredentialDigest(token) != r.ProviderCredentialDigest {
		return HttpCapabilityDecision{}, serviceError(403, "forbidden: provider credential digest mismatch")
	}
	orglessOnboarding := org == "" && managementOnboardingCapability(r.ProviderID, *cap)
	trustedTenantManagement := false
	tenantReactivation := managementReactivationCapability(r.ProviderID, *cap, org, r.ResourceID, r.CanonicalPath)
	if org != "" && managementOrganizationCapability(r.ProviderID, *cap) {
		app, e := s.store.GetApplicationWithConsistency(ctx, s.cfg.Security.HTTPCapabilityPEPApplicationID, ReadStrong)
		if e == nil && app.FirstParty && app.ConsentPolicy == "auto_approve" && app.OrganizationID != org {
			trustedTenantManagement = true
		}
	}
	var c applicationAuthorizationContext
	if orglessOnboarding || trustedTenantManagement {
		c, err = s.resolveOnboardingContext(ctx, token, *cap.ActionID)
		if err == nil && trustedTenantManagement {
			var principal AuthenticatedPrincipal
			if tenantReactivation {
				principal, err = s.AuthenticateRequestForInactiveOrganization(ctx, token, org, ReadStrong)
			} else {
				principal, err = s.Authenticate(ctx, token, org, ReadStrong)
			}
			c.UserID, c.OrganizationID = principal.UserID, org
		}
	} else {
		c, err = s.resolveApplicationAuthorizationContext(ctx, token, org)
	}
	if err != nil {
		return HttpCapabilityDecision{}, err
	}
	if c.ExternalSubject != r.ProviderCredential.Subject || c.ProviderAudience != r.ProviderCredential.Audience {
		return HttpCapabilityDecision{}, serviceError(403, "forbidden: provider credential principal mismatch")
	}
	if cap.ActionID == nil || cap.Resource == nil {
		return HttpCapabilityDecision{}, serviceError(503, "authorized capability action or resource missing")
	}
	var scope *HttpRecordScope
	allowed, reason := false, "denied"
	managementRoute, managementKnown := LookupManagementRoute(cap.Method, cap.PathTemplate)
	if orglessOnboarding {
		allowed, reason = true, "management_onboarding"
	} else if trustedTenantManagement {
		decisions, e := s.store.evaluateUserActions(ctx, []UserActionCheck{{UserID: c.UserID, OrganizationID: org, ProviderID: r.ProviderID, ResourceType: "organization", ResourceID: r.ResourceID, ActionID: *cap.ActionID, Consistency: ReadStrong}}, tenantReactivation)
		if e != nil {
			return HttpCapabilityDecision{}, e
		}
		allowed, reason = len(decisions) == 1 && decisions[0].Allowed, "trusted_tenant_management"
	} else if r.ProviderID == "grant-management" && managementKnown && managementRoute.Platform &&
		(s.cfg.Security.ManagementOrganizationID == "" || org != s.cfg.Security.ManagementOrganizationID) {
		allowed, reason = false, "platform_management_organization_required"
	} else if r.ResponseDigest != "" {
		allowed, reason, err = s.responseDecision(ctx, token, org, r, c)
		if err != nil {
			return HttpCapabilityDecision{}, err
		}
	} else if r.ResponseLength != 0 || r.ResponseReferencesDigest != "" || len(r.ResponseReferences) != 0 {
		return HttpCapabilityDecision{}, serviceError(403, "forbidden: incomplete response evidence")
	} else if cap.Batch != nil {
		allowed, reason, err = s.batchDecision(ctx, token, org, r, c)
		if err != nil {
			return HttpCapabilityDecision{}, err
		}
	} else if cap.RecordScope != nil {
		scope, err = s.recordScopeDecision(ctx, r.ProviderID, *cap, c, r.CatalogGeneration, r.CatalogDigest, r.PEPClientID)
		if err != nil {
			return HttpCapabilityDecision{}, err
		}
		allowed, reason = true, "record_scope"
	} else if managementReactivationCapability(r.ProviderID, *cap, org, r.ResourceID, r.CanonicalPath) {
		decisions, e := s.store.evaluateUserActions(ctx, []UserActionCheck{{
			UserID: c.UserID, OrganizationID: c.OrganizationID, ProviderID: r.ProviderID,
			ResourceType: "organization", ResourceID: org, ActionID: *cap.ActionID,
			ApplicationID: &c.ApplicationID, Consistency: ReadStrong,
		}}, true)
		if e != nil {
			return HttpCapabilityDecision{}, e
		}
		allowed, reason = len(decisions) == 1 && decisions[0].Allowed, "management_reactivation"
	} else if managementEnsureCapability(r.ProviderID, *cap) {
		// Ensure endpoint: auth-only provisioning for the authenticated identity.
		// Requires active org/provider and explicit application_actions admission
		// but no membership/role/consent because it only provisions self and
		// creates no membership, binding, or role.
		rows, e := serviceRows[struct{}](ctx, s, ReadStrong,
			`SELECT 1
			 FROM application_actions aa
			 JOIN actions a ON a.id = aa.action_id AND a.provider_id = ? AND a.grantable_to_applications = 1
			 JOIN service_providers sp ON sp.id = a.provider_id AND sp.active = 1
			 JOIN organizations org ON org.id = ? AND org.inactivated_at IS NULL
			 WHERE aa.application_id = ? AND aa.action_id = ?`,
			r.ProviderID, c.OrganizationID, c.ApplicationID, *cap.ActionID)
		if e != nil {
			return HttpCapabilityDecision{}, e
		}
		allowed, reason = len(rows) == 1, "management_materialize"
	} else {
		decisions, err := s.evaluateAuthorizations(ctx, []AuthorizationCheck{{Subject: AuthorizationSubject{SubjectType: "user", ID: c.UserID, OrganizationID: c.OrganizationID}, Action: *cap.ActionID, Resource: AuthorizationResource{ProviderID: r.ProviderID, ResourceType: cap.Resource.ResourceType, ID: r.ResourceID}}}, ReadStrong, &c.ApplicationID)
		if err != nil {
			return HttpCapabilityDecision{}, err
		}
		allowed, reason = decisions[0].Allowed, decisions[0].Reason
	}
	now := time.Now().UTC()
	return HttpCapabilityDecision{Allowed: allowed, Reason: reason, ProviderID: r.ProviderID, PEPClientID: r.PEPClientID, CatalogGeneration: r.CatalogGeneration, CatalogDigest: r.CatalogDigest, CapabilityID: r.CapabilityID, CapabilityVersion: r.CapabilityVersion, ContractDigest: r.ContractDigest, ActionID: *cap.ActionID, ResourceType: r.ResourceType, ResourceID: r.ResourceID, Method: r.Method, CanonicalPath: r.CanonicalPath, CanonicalQueryDigest: r.CanonicalQueryDigest, ForwardedHeaderDigest: r.ForwardedHeaderDigest, BodyDigest: r.BodyDigest, BodyLength: r.BodyLength, ProviderCredentialDigest: r.ProviderCredentialDigest, ProviderSubject: c.ExternalSubject, ProviderAudience: c.ProviderAudience, RequestBinding: r.RequestBinding, PrincipalDigest: ExposurePrincipalDigest(c.UserID, c.OrganizationID, c.ApplicationID), IssuedAt: now.Format(time.RFC3339Nano), ExpiresAt: now.Add(5 * time.Second).Format(time.RFC3339Nano), RecordScope: scope, BatchDigest: r.BatchDigest, ResponseDigest: r.ResponseDigest, ResponseLength: r.ResponseLength, ResponseReferencesDigest: r.ResponseReferencesDigest}, nil
}
