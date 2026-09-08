package api

import "github.com/Conalog/grant/internal/grant"

func (s *Server) capabilityRoutes() {
	s.route("POST", "/api/v1/admin/providers/{providerId}/http-capability-generations", "master", true, 201, body(true, func(r *request, v grant.StageHttpCapabilityGeneration) (any, error) {
		return s.service.StageHttpCapabilityGenerationAdmin(r.Context(), r.PathValue("providerId"), v)
	}))
	s.route("POST", "/api/v1/admin/providers/{providerId}/http-capability-active", "master", true, 200, body(true, func(r *request, v grant.ActivateHttpCapabilityGeneration) (any, error) {
		return s.service.ActivateHttpCapabilityGenerationAdmin(r.Context(), r.PathValue("providerId"), v)
	}))
	s.route("PUT", "/api/v1/admin/providers/{providerId}/pep-clients/{pepClientId}", "master", true, 200, body(true, func(r *request, v grant.UpsertProviderPepClient) (any, error) {
		return s.service.SetProviderPepClientAdmin(r.Context(), r.PathValue("providerId"), r.PathValue("pepClientId"), v)
	}))
	s.route("GET", "/api/v2/http-capabilities/catalog/{providerId}", "app", false, 200, func(r *request) (any, error) {
		return s.service.ActiveHttpCapabilityCatalog(r.Context(), r.PathValue("providerId"))
	})
	s.route("POST", "/api/v2/http-capabilities/decide", "application", false, 200, body(true, func(r *request, v grant.HttpCapabilityDecisionInput) (any, error) {
		return s.service.DecideHttpCapability(r.Context(), r.bearer, r.org, v)
	}))
	s.route("POST", "/api/v1/admin/imports/patch-data-authority", "master", true, 200, body(true, func(r *request, v grant.PatchDataAuthorityImport) (any, error) {
		return s.service.ImportPatchDataAuthority(r.Context(), v)
	}))
	s.route("POST", "/api/v2/admin/imports/authority-snapshots", "master", true, 200, body(true, func(r *request, v grant.AuthoritySnapshotV2) (any, error) {
		if v.SourceID == "patch-data" {
			if e := validatePatchDataSnapshot(v); e != nil {
				return nil, e
			}
		}
		return s.service.ImportAuthoritySnapshotV2(r.Context(), v)
	}))
}
func validatePatchDataSnapshot(v grant.AuthoritySnapshotV2) error {
	principals := map[string]grant.AuthorityPrincipalV2{}
	active := map[[2]string]bool{}
	for _, p := range v.Principals {
		if p.Kind != "user" && p.Kind != "viewer" && p.Kind != "temporary" && p.Kind != "service" || p.ExternalSubject != "patch-data:"+p.Kind+":"+p.ID {
			return failure(400, "invalid Patch Data principal semantics")
		}
		principals[p.ID] = p
	}
	for _, m := range v.Memberships {
		valid := (m.PrincipalType == "user" && (m.Role == "owner" || m.Role == "member")) || (m.PrincipalType == "viewer" && m.Role == "viewer") || (m.PrincipalType == "temporary" && m.Role == "temporary")
		ref := m.PrincipalType + ":" + m.PrincipalID
		if m.Role == "owner" {
			ref = "organizations.owner"
		}
		if !valid || len(m.SourceRefs) != 1 || m.SourceRefs[0] != ref {
			return failure(400, "invalid Patch Data membership semantics")
		}
		if m.Lifecycle == "active" {
			active[[2]string{m.PrincipalID, m.OrganizationID}] = true
		}
	}
	validDerivation := map[string]bool{"direct": true, "organization-owner": true, "organization-member": true, "descendant": true, "ancestor": true, "plant-permission": true, "temporary-assignment": true, "work-order-assignment": true}
	for _, b := range v.EffectiveBindings {
		if b.ProviderID != "patch-data" || b.ActionID != "patch-data.plant.read" || b.ResourceType != "plant" || !validDerivation[b.Derivation] {
			return failure(400, "invalid Patch Data binding semantics")
		}
		p, ok := principals[b.PrincipalID]
		if !ok {
			return failure(400, "Patch Data binding principal missing")
		}
		if p.Kind == "service" {
			if b.Derivation != "direct" {
				return failure(400, "Patch Data service binding must be direct")
			}
		} else if !active[[2]string{p.ID, b.ApplicationScope.OrganizationID}] {
			return failure(400, "Patch Data human binding requires active membership")
		}
	}
	return nil
}
