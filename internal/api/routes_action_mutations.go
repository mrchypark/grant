package api

import (
	"encoding/json"
	"net/http"
	"strings"

	"github.com/mrchypark/grant/internal/grant"
)

type snapshotResponse struct{ grant.ActionSnapshot }

func snapshotResult(v grant.ActionSnapshot, err error) (any, error) {
	if err != nil {
		return nil, err
	}
	return snapshotResponse{v}, nil
}

func conditionalHeader(r *http.Request, name string) (string, error) {
	values := r.Header.Values(name)
	if len(values) == 0 {
		return "", nil
	}
	if len(values) != 1 || strings.TrimSpace(values[0]) == "" {
		return "", failure(400, "expected one nonempty "+name+" header")
	}
	return strings.TrimSpace(values[0]), nil
}

func (s *Server) actionMutationRoutes() {
	s.route("GET", "/api/v1/orgs/{organizationId}/roles/{roleId}/actions", "user", true, 200, func(r *request) (any, error) {
		return snapshotResult(s.service.RoleActionsSnapshot(r.Context(), r.principal.UserID, r.principal.OrganizationID, r.PathValue("roleId")))
	})
	for _, present := range []bool{true, false} {
		method := "DELETE"
		if present {
			method = "PUT"
		}
		s.route(method, "/api/v1/orgs/{organizationId}/roles/{roleId}/actions/{actionId}", "user", true, 204, func(r *request) (any, error) {
			return nil, s.service.SetRoleAction(r.Context(), r.principal.UserID, r.principal.OrganizationID, r.PathValue("roleId"), r.PathValue("actionId"), present)
		})
		s.route(method, "/api/v1/orgs/{organizationId}/apps/{applicationId}/actions/{actionId}", "user", true, 204, func(r *request) (any, error) {
			return nil, s.service.SetApplicationAction(r.Context(), r.principal.UserID, r.principal.OrganizationID, r.PathValue("applicationId"), r.PathValue("actionId"), present)
		})
	}
	s.route("POST", "/api/v1/admin/providers/{providerId}/actions", "master", true, 201, body(true, func(r *request, v providerAction) (any, error) {
		return s.service.CreateProviderAction(r.Context(), r.PathValue("providerId"), providerActionValue(r.PathValue("providerId"), v))
	}))
	s.route("PATCH", "/api/v1/admin/providers/{providerId}/actions/{actionId}", "master", true, 200, body(true, func(r *request, v map[string]json.RawMessage) (any, error) {
		return s.service.PatchProviderAction(r.Context(), r.PathValue("providerId"), r.PathValue("actionId"), v)
	}))
	s.route("DELETE", "/api/v1/admin/providers/{providerId}/actions/{actionId}", "master", true, 204, func(r *request) (any, error) {
		return nil, s.service.DeleteProviderAction(r.Context(), r.PathValue("providerId"), r.PathValue("actionId"))
	})
}

func providerActionValue(provider string, a providerAction) grant.Action {
	return grant.Action{ID: a.ID, ProviderID: provider, Code: a.Code, Name: a.Name, Description: a.Description, RiskLevel: a.RiskLevel, VisibilityGranting: a.VisibilityGranting, GrantableToApplications: a.GrantableToApplications, ConsentLabel: a.ConsentLabel, ConsentDescription: a.ConsentDescription}
}
