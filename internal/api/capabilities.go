package api

import "github.com/Conalog/grant/internal/grant"

func (s *Server) capabilityRoutes() {
	s.route("GET", "/api/v1/admin/providers/{providerId}/http-capability-active", "master", true, 200, func(r *request) (any, error) {
		return s.service.ActiveHttpCapabilityCatalogAdmin(r.Context(), r.PathValue("providerId"))
	})
	s.route("POST", "/api/v1/admin/providers/{providerId}/http-capability-generations", "master", true, 201, body(true, func(r *request, v grant.StageHttpCapabilityGeneration) (any, error) {
		return s.service.StageHttpCapabilityGenerationAdmin(r.Context(), r.PathValue("providerId"), v)
	}))
	s.route("POST", "/api/v1/admin/providers/{providerId}/http-capability-active", "master", true, 200, body(true, func(r *request, v grant.ActivateHttpCapabilityGeneration) (any, error) {
		return s.service.ActivateHttpCapabilityGenerationAdmin(r.Context(), r.PathValue("providerId"), v)
	}))
	s.route("PUT", "/ops/providers/{providerId}/enforcers/{pepClientId}", "master", true, 200, body(true, func(r *request, v grant.UpsertProviderPepClient) (any, error) {
		return s.service.SetProviderPepClientAdmin(r.Context(), r.PathValue("providerId"), r.PathValue("pepClientId"), v)
	}))
	s.route("GET", "/api/v2/http-capabilities/catalog/{providerId}", "app", false, 200, func(r *request) (any, error) {
		return s.service.ActiveHttpCapabilityCatalog(r.Context(), r.PathValue("providerId"))
	})
	s.route("POST", "/api/v2/http-capabilities/decide", "application", false, 200, body(true, func(r *request, v grant.HttpCapabilityDecisionInput) (any, error) {
		return s.service.DecideHttpCapability(r.Context(), r.bearer, r.org, v)
	}))
	s.route("POST", "/ops/migrations/patch-data-authority", "master", true, 200, body(true, func(r *request, v grant.PatchDataAuthorityImport) (any, error) {
		return s.service.ImportPatchDataAuthority(r.Context(), v)
	}))
}
