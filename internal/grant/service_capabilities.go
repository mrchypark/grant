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
	pepID := s.cfg.Security.HTTPCapabilityPEPClientID
	if pepID == "" {
		return ProviderPepClient{}, serviceError(503, "HTTP capability PEP client is not configured")
	}
	pep, err := s.store.ProviderPepClient(ctx, provider, pepID)
	if err != nil {
		return ProviderPepClient{}, err
	}
	if err = requireActivePEP(pep, time.Now()); err != nil {
		return ProviderPepClient{}, err
	}
	if s.store.db == nil && s.cfg.Database.LocalNodeRole != "learner" && pep.AuthorizationMode != "application" {
		return ProviderPepClient{}, serviceError(503, "unavailable: PEP policy requires local fenced authority")
	}
	return pep, nil
}
func (s *Service) GetProviderPepClientAdmin(ctx context.Context, provider, pep string) (ProviderPepClient, error) {
	return s.store.ProviderPepClientAdmin(ctx, provider, pep)
}
func (s *Service) SetProviderPepClientAdminConditional(ctx context.Context, provider, pep string, v UpsertProviderPepClient, expectedRevision *int64) (ProviderPepClient, error) {
	return s.store.UpsertProviderPepClientConditional(ctx, provider, pep, ManagementActorFromContext(ctx), v, expectedRevision)
}

func validateHTTPPolicy(pep ProviderPepClient, r HttpCapabilityDecisionInput) error {
	if pep.AuthorizationMode != "" && pep.AuthorizationMode != "application" && pep.AuthorizationMode != "user" {
		return serviceError(503, "unavailable: unsupported PEP policy")
	}
	if r.WireVersion != 0 && r.WireVersion != 2 {
		return serviceError(400, "unsupported HTTP capability wire version")
	}
	if pep.AuthorizationMode == "user" && r.WireVersion != 2 {
		return serviceError(400, "user authorization requires wire version 2")
	}
	if r.WireVersion == 2 {
		if r.PEPPolicyRevision <= 0 {
			return serviceError(400, "PEP policy revision is required")
		}
		if pep.PolicyRevision <= 0 || r.PEPPolicyRevision != pep.PolicyRevision {
			return serviceError(403, "forbidden: stale PEP policy")
		}
	} else if r.PEPPolicyRevision != 0 {
		return serviceError(400, "PEP policy revision requires wire version 2")
	}
	return nil
}
func (s *Service) ActiveHttpCapabilityCatalog(ctx context.Context, provider string) (ActiveHttpCapabilityCatalog, error) {
	if readScope(ctx) != nil {
		return s.activeHttpCapabilityCatalog(ctx, provider)
	}
	return capabilityPolicyRead(ctx, s, provider, 0, func(ctx context.Context) (ActiveHttpCapabilityCatalog, error) {
		return s.activeHttpCapabilityCatalog(ctx, provider)
	})
}
func (s *Service) activeHttpCapabilityCatalog(ctx context.Context, provider string) (ActiveHttpCapabilityCatalog, error) {
	pep, err := s.configuredPEP(ctx, provider)
	if err != nil {
		return ActiveHttpCapabilityCatalog{}, err
	}
	catalog, err := s.store.ActiveHttpCapabilityCatalog(ctx, provider, pep.PepClientID)
	if err != nil {
		return ActiveHttpCapabilityCatalog{}, err
	}
	if pep.AuthorizationMode == "user" {
		if pep.PolicyRevision <= 0 {
			return ActiveHttpCapabilityCatalog{}, serviceError(503, "unavailable: invalid PEP policy")
		}
		catalog.AuthorizationMode, catalog.PEPPolicyRevision = "user", pep.PolicyRevision
	} else {
		catalog.AuthorizationMode, catalog.PEPPolicyRevision = "", 0
	}
	return catalog, nil
}
func (s *Service) DecideHttpCapability(ctx context.Context, token, org string, r HttpCapabilityDecisionInput) (HttpCapabilityDecision, error) {
	if strings.TrimSpace(token) == "" {
		return HttpCapabilityDecision{}, serviceError(401, "unauthorized: missing bearer token")
	}
	identity, ok := ctx.Value(replicaIdentityKey{}).(ExternalIdentity)
	if !ok {
		var err error
		identity, err = s.externalIdentity(ctx, token)
		if err != nil {
			return HttpCapabilityDecision{}, err
		}
	}
	ctx = context.WithValue(ctx, replicaIdentityKey{}, identity)
	return capabilityPolicyRead(ctx, s, r.ProviderID, r.WireVersion, func(ctx context.Context) (HttpCapabilityDecision, error) {
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
	if err = validateHTTPPolicy(pep, r); err != nil {
		return HttpCapabilityDecision{}, err
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
	if org == "" && cap.OrganizationContext != "optional" {
		return HttpCapabilityDecision{}, serviceError(400, "organization context is required")
	}
	managementRoute, managementKnown := LookupManagementRoute(cap.Method, cap.PathTemplate)
	platformManagementOrgRequired := r.ProviderID == "grant-management" && managementKnown && managementRoute.Platform &&
		(s.cfg.Security.ManagementOrganizationID == "" || org != s.cfg.Security.ManagementOrganizationID)
	if pep.AuthorizationMode == "user" {
		identity, ok := ctx.Value(replicaIdentityKey{}).(ExternalIdentity)
		if !ok {
			return HttpCapabilityDecision{}, serviceError(401, "unauthorized: verified identity required")
		}
		c, e := s.resolveLocalUserAuthorizationContext(ctx, identity, org)
		if e != nil {
			return HttpCapabilityDecision{}, e
		}
		if platformManagementOrgRequired {
			return httpCapabilityDecisionResult(r, c, *cap, pep, false, "platform_management_organization_required", nil), nil
		}
		return s.decideUserHTTPCapability(ctx, token, org, r, pep, *cap, c)
	}
	orglessOnboarding := org == "" && managementOnboardingCapability(r.ProviderID, *cap)
	trustedTenantManagement := false
	tenantReactivation := managementReactivationCapability(r.ProviderID, *cap, org, r.ResourceID, r.CanonicalPath)
	if org != "" && managementOrganizationCapability(r.ProviderID, *cap) {
		app, e := s.store.GetApplicationWithConsistency(ctx, s.cfg.Security.ManagementApplicationID, ReadStrong)
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
	} else if org == "" {
		c, err = s.resolveOrglessApplicationContext(ctx, token)
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
	if orglessOnboarding {
		allowed, reason = true, "management_onboarding"
	} else if trustedTenantManagement {
		decisions, e := s.store.evaluateUserActions(ctx, []UserActionCheck{{UserID: c.UserID, OrganizationID: org, ProviderID: r.ProviderID, ResourceType: "organization", ResourceID: r.ResourceID, ActionID: *cap.ActionID, Consistency: ReadStrong}}, tenantReactivation)
		if e != nil {
			return HttpCapabilityDecision{}, e
		}
		allowed, reason = len(decisions) == 1 && decisions[0].Allowed, "trusted_tenant_management"
	} else if platformManagementOrgRequired {
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
	} else if org == "" {
		allowed, err = s.orglessActionAllowed(ctx, c, r.ProviderID, *cap.ActionID, r.ResourceType, r.ResourceID)
		if err != nil {
			return HttpCapabilityDecision{}, err
		}
		reason = "orgless_explicit_binding"
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
	return httpCapabilityDecisionResult(r, c, *cap, pep, allowed, reason, scope), nil
}

func httpCapabilityDecisionResult(r HttpCapabilityDecisionInput, c applicationAuthorizationContext, cap HttpCapability, pep ProviderPepClient, allowed bool, reason string, scope *HttpRecordScope) HttpCapabilityDecision {
	now := time.Now().UTC()
	var revision int64
	if r.WireVersion == 2 {
		revision = pep.PolicyRevision
	}
	return HttpCapabilityDecision{WireVersion: r.WireVersion, PEPPolicyRevision: revision, Allowed: allowed, Reason: reason, ProviderID: r.ProviderID, PEPClientID: r.PEPClientID, CatalogGeneration: r.CatalogGeneration, CatalogDigest: r.CatalogDigest, CapabilityID: r.CapabilityID, CapabilityVersion: r.CapabilityVersion, ContractDigest: r.ContractDigest, ActionID: *cap.ActionID, ResourceType: r.ResourceType, ResourceID: r.ResourceID, Method: r.Method, CanonicalPath: r.CanonicalPath, CanonicalQueryDigest: r.CanonicalQueryDigest, ForwardedHeaderDigest: r.ForwardedHeaderDigest, BodyDigest: r.BodyDigest, BodyLength: r.BodyLength, ProviderCredentialDigest: r.ProviderCredentialDigest, ProviderSubject: c.ExternalSubject, ProviderAudience: c.ProviderAudience, RequestBinding: r.RequestBinding, PrincipalDigest: ExposurePrincipalDigest(c.UserID, c.OrganizationID, c.ApplicationID), IssuedAt: now.Format(time.RFC3339Nano), ExpiresAt: now.Add(5 * time.Second).Format(time.RFC3339Nano), RecordScope: scope, BatchDigest: r.BatchDigest, ResponseDigest: r.ResponseDigest, ResponseLength: r.ResponseLength, ResponseReferencesDigest: r.ResponseReferencesDigest}
}

func (s *Service) decideUserHTTPCapability(ctx context.Context, token, org string, r HttpCapabilityDecisionInput, pep ProviderPepClient, cap HttpCapability, c applicationAuthorizationContext) (HttpCapabilityDecision, error) {
	if c.ExternalSubject != r.ProviderCredential.Subject || c.ProviderAudience != r.ProviderCredential.Audience {
		return HttpCapabilityDecision{}, serviceError(403, "forbidden: provider credential principal mismatch")
	}
	if cap.ActionID == nil || cap.Resource == nil {
		return HttpCapabilityDecision{}, serviceError(503, "authorized capability action or resource missing")
	}
	var scope *HttpRecordScope
	var allowed bool
	var reason string
	var err error
	switch {
	case r.ResponseDigest != "":
		allowed, reason, err = s.responseDecision(ctx, token, org, r, c)
	case r.ResponseLength != 0 || r.ResponseReferencesDigest != "" || len(r.ResponseReferences) != 0:
		err = serviceError(403, "forbidden: incomplete response evidence")
	case cap.Batch != nil:
		allowed, reason, err = s.batchDecision(ctx, token, org, r, c)
	case cap.RecordScope != nil:
		scope, err = s.recordScopeDecision(ctx, r.ProviderID, cap, c, r.CatalogGeneration, r.CatalogDigest, r.PEPClientID)
		allowed, reason = err == nil, "record_scope"
	default:
		var decisions []AuthorizationDecision
		decisions, err = s.evaluateAuthorizations(ctx, []AuthorizationCheck{{Subject: AuthorizationSubject{SubjectType: "user", ID: c.UserID, OrganizationID: c.OrganizationID}, Action: *cap.ActionID, Resource: AuthorizationResource{ProviderID: r.ProviderID, ResourceType: cap.Resource.ResourceType, ID: r.ResourceID}}}, ReadStrong, nil)
		if err == nil {
			if len(decisions) != 1 {
				err = serviceError(503, "incomplete authorization decision")
			} else {
				allowed, reason = decisions[0].Allowed, decisions[0].Reason
			}
		}
	}
	if err != nil {
		return HttpCapabilityDecision{}, err
	}
	return httpCapabilityDecisionResult(r, c, cap, pep, allowed, reason, scope), nil
}

// Follow-up evidence retains the original principal and server-selected mode.
func (s *Service) httpFollowupPrincipal(ctx context.Context, token, org string, input HttpCapabilityDecisionInput, pep ProviderPepClient, principal applicationAuthorizationContext) (applicationAuthorizationContext, error) {
	if err := validateHTTPPolicy(pep, input); err != nil {
		return applicationAuthorizationContext{}, err
	}
	var current applicationAuthorizationContext
	var err error
	if pep.AuthorizationMode == "user" {
		if principal.ApplicationID != "" {
			return current, serviceError(403, "forbidden: authorization mode changed")
		}
		identity, ok := ctx.Value(replicaIdentityKey{}).(ExternalIdentity)
		if !ok {
			return current, serviceError(401, "unauthorized: verified identity required")
		}
		current, err = s.resolveLocalUserAuthorizationContext(ctx, identity, org)
	} else {
		if principal.ApplicationID == "" {
			return current, serviceError(403, "forbidden: authorization mode changed")
		}
		if org == "" {
			current, err = s.resolveOrglessApplicationContext(ctx, token)
		} else {
			current, err = s.resolveApplicationAuthorizationContext(ctx, token, org)
		}
	}
	if err != nil {
		return current, err
	}
	if current != principal {
		return current, serviceError(403, "forbidden: authorization principal changed")
	}
	return current, nil
}

func httpAuthorizationApplication(c applicationAuthorizationContext) *string {
	if c.ApplicationID == "" {
		return nil
	}
	return &c.ApplicationID
}
