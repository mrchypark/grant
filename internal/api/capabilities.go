package api

import (
	"crypto/sha256"
	"encoding/json"
	"fmt"
	"strings"

	"github.com/mrchypark/grant/internal/grant"
)

func (s *Server) capabilityRoutes() {
	s.route("PUT", "/ops/orgless-bindings/{bindingId}", "master", true, 200, body(true, func(r *request, v grant.OrglessUserBinding) (any, error) {
		return s.service.SetOrglessUserBindingAdmin(r.Context(), r.PathValue("bindingId"), v)
	}))
	s.route("GET", "/api/v1/admin/providers/{providerId}/http-capability-active", "master", true, 200, func(r *request) (any, error) {
		return s.service.ActiveHttpCapabilityCatalogAdmin(r.Context(), r.PathValue("providerId"))
	})
	s.route("POST", "/api/v1/admin/providers/{providerId}/http-capability-generations", "master", true, 201, body(true, func(r *request, v grant.StageHttpCapabilityGeneration) (any, error) {
		return s.service.StageHttpCapabilityGenerationAdmin(r.Context(), r.PathValue("providerId"), v)
	}))
	s.route("POST", "/api/v1/admin/providers/{providerId}/http-capability-active", "master", true, 200, body(true, func(r *request, v grant.ActivateHttpCapabilityGeneration) (any, error) {
		return s.service.ActivateHttpCapabilityGenerationAdmin(r.Context(), r.PathValue("providerId"), v)
	}))
	s.route("GET", "/ops/providers/{providerId}/enforcers/{pepClientId}", "master", true, 200, func(r *request) (any, error) {
		if err := validatePEPPolicyIDs(r); err != nil {
			return nil, err
		}
		return pepPolicyResult(s.service.GetProviderPepClientAdmin(r.Context(), r.PathValue("providerId"), r.PathValue("pepClientId")))
	})
	s.route("PUT", "/ops/providers/{providerId}/enforcers/{pepClientId}", "master", true, 200, lifecycleBody([]string{"lifecycle", "authorizationMode"}, func(r *request, v grant.UpsertProviderPepClient) (any, error) {
		return s.setPEPPolicy(r, v)
	}))
	s.route("GET", "/api/v2/http-capabilities/catalog/{providerId}", "app", false, 200, func(r *request) (any, error) {
		return s.service.ActiveHttpCapabilityCatalog(r.Context(), r.PathValue("providerId"))
	})
	s.route("POST", "/api/v2/http-capabilities/decide", "http-decision", false, 200, body(true, func(r *request, v grant.HttpCapabilityDecisionInput) (any, error) {
		if v.PEPPolicyRevision < 0 {
			return nil, failure(422, "pepPolicyRevision must not be negative")
		}
		return s.service.DecideHttpCapability(r.Context(), r.bearer, r.org, v)
	}))
	s.route("POST", "/ops/migrations/patch-data-authority", "master", true, 200, body(true, func(r *request, v grant.PatchDataAuthorityImport) (any, error) {
		return s.service.ImportPatchDataAuthority(r.Context(), v)
	}))
}

func validatePEPPolicyIDs(r *request) error {
	if strings.ContainsAny(r.PathValue("providerId")+r.PathValue("pepClientId"), "\r\n") {
		return failure(422, "invalid PEP policy identifier")
	}
	return nil
}

func pepPolicyETag(v grant.ProviderPepClient) string {
	digest := sha256.Sum256([]byte(fmt.Sprintf("%s\n%s\n%d", v.ProviderID, v.PepClientID, v.PolicyRevision)))
	return fmt.Sprintf(`"pep-policy-sha256:%x"`, digest)
}

func pepPolicyResult(v grant.ProviderPepClient, err error) (any, error) {
	if err != nil {
		return nil, err
	}
	encoded, err := json.Marshal(v)
	if err != nil {
		return nil, err
	}
	return snapshotResponse{grant.ActionSnapshot{Value: encoded, ETag: pepPolicyETag(v)}}, nil
}

func (s *Server) setPEPPolicy(r *request, v grant.UpsertProviderPepClient) (any, error) {
	if err := validatePEPPolicyIDs(r); err != nil {
		return nil, err
	}
	match, err := conditionalHeader(r.Request, "If-Match")
	if err != nil {
		return nil, err
	}
	none, err := conditionalHeader(r.Request, "If-None-Match")
	if err != nil {
		return nil, err
	}
	if match != "" && none != "" || none != "" && none != "*" {
		return nil, failure(400, "invalid PEP policy preconditions")
	}
	if match == "" && none == "" {
		return nil, failure(428, "PEP policy precondition required")
	}
	var revision int64
	provider, pep := r.PathValue("providerId"), r.PathValue("pepClientId")
	if match != "" {
		current, err := s.service.GetProviderPepClientAdmin(r.Context(), provider, pep)
		if err != nil {
			if grant.ErrorStatus(err) == 404 {
				return nil, failure(412, "PEP policy precondition failed")
			}
			return nil, err
		}
		if match != pepPolicyETag(current) {
			return nil, failure(412, "PEP policy precondition failed")
		}
		revision = current.PolicyRevision
	}
	return pepPolicyResult(s.service.SetProviderPepClientAdminConditional(r.Context(), provider, pep, v, &revision))
}
