package api

import (
	"github.com/mrchypark/grant/internal/grant"
	"net/http"
)

type actionsRequest struct {
	ActionIDs []string `json:"actionIds"`
}
type activeRequest struct {
	Active bool `json:"active"`
}
type trustRequest struct {
	Trusted bool `json:"trusted"`
}
type updateGroupRequest struct {
	Name        string  `json:"name"`
	Description *string `json:"description"`
}
type applicationResponse struct {
	grant.Application
	RedirectURI *string `json:"redirectUri,omitempty"`
}

func appResponse(v grant.ApplicationRead, e error) (any, error) {
	return applicationResponse{v.Application, v.RedirectURI}, e
}
func (s *Server) routes() {
	s.mux.HandleFunc("GET /healthz", func(w http.ResponseWriter, r *http.Request) {
		writeJSON(w, http.StatusOK, map[string]string{"status": "ok"})
	})
	s.mux.HandleFunc("GET /readyz", func(w http.ResponseWriter, r *http.Request) {
		health, ready := s.service.Readiness(r.Context())
		code := 200
		if !ready {
			code = 503
		}
		writeJSON(w, code, health)
	})
	s.route("GET", "/api/v1/me", "bearer", false, 200, func(r *request) (any, error) {
		return s.service.CurrentUser(r.Context(), r.bearer, r.org, r.consistency)
	})
	s.route("POST", "/api/v1/me/ensure", "bearer", true, 200, bodyless(func(r *request) (any, error) { return s.service.EnsureCurrentUser(r.Context(), r.bearer) }))
	s.route("GET", "/api/v1/providers", "", false, 200, func(r *request) (any, error) { return s.service.ListServiceProviders(r.Context(), r.consistency) })
	s.route("POST", "/api/v1/admin/providers", "master", true, 201, body(false, func(r *request, v grant.ServiceProvider) (any, error) {
		return s.service.CreateServiceProviderAdmin(r.Context(), v)
	}))
	s.route("PUT", "/api/v1/admin/providers/{providerId}/actions", "master", true, 200, body(false, s.replaceProviderActions))
	s.route("GET", "/api/v1/admin/providers/{providerId}/actions", "master", true, 200, func(r *request) (any, error) {
		return snapshotResult(s.service.ProviderActionsSnapshot(r.Context(), r.PathValue("providerId")))
	})
	s.route("GET", "/api/v1/orgs/{organizationId}/providers/{providerId}/actions", "user", true, 200, func(r *request) (any, error) {
		return s.service.ListProviderActionsAuthorized(r.Context(), r.principal.UserID, r.principal.OrganizationID, r.PathValue("providerId"))
	})
	s.route("GET", "/api/v1/orgs/{organizationId}/apps", "user", false, 200, func(r *request) (any, error) {
		return s.service.ListApplicationsAuthorized(r.Context(), r.principal.UserID, r.principal.OrganizationID, r.consistency)
	})
	s.route("POST", "/api/v1/orgs/{organizationId}/apps", "user", true, 201, body(false, func(r *request, v grant.CreateApplicationInput) (any, error) {
		return appResponse(s.service.CreateApplication(r.Context(), r.principal.UserID, r.principal.OrganizationID, v))
	}))
	s.route("GET", "/api/v1/orgs/{organizationId}/apps/{applicationId}", "user", false, 200, func(r *request) (any, error) {
		return appResponse(s.service.GetApplicationDetailAuthorized(r.Context(), r.principal.UserID, r.principal.OrganizationID, r.PathValue("applicationId"), r.consistency))
	})
	s.route("GET", "/api/v1/orgs/{organizationId}/apps/{applicationId}/actions", "user", false, 200, func(r *request) (any, error) {
		return snapshotResult(s.service.ApplicationActionsSnapshot(r.Context(), r.principal.UserID, r.principal.OrganizationID, r.PathValue("applicationId")))
	})
	s.route("PUT", "/api/v1/orgs/{organizationId}/apps/{applicationId}/actions", "user", true, 200, body(false, func(r *request, v actionsRequest) (any, error) {
		match, err := conditionalHeader(r.Request, "If-Match")
		if err != nil {
			return nil, err
		}
		return s.service.ReplaceApplicationActionsConditional(r.Context(), r.principal.UserID, r.principal.OrganizationID, r.PathValue("applicationId"), v.ActionIDs, match)
	}))
	s.route("GET", "/api/v1/orgs/{organizationId}/app-grants", "user", false, 200, func(r *request) (any, error) {
		return s.service.ListApplicationGrantsAuthorized(r.Context(), r.principal.UserID, r.principal.OrganizationID, r.consistency)
	})
	s.route("GET", "/api/v1/orgs/{organizationId}/apps/{applicationId}/consent", "user", false, 200, func(r *request) (any, error) {
		v, e := s.service.ConsentSnapshot(r.Context(), r.principal.UserID, r.principal.OrganizationID, r.PathValue("applicationId"))
		if e == nil && string(v.Value) == "null" {
			return nil, failure(404, "application consent not found")
		}
		return snapshotResult(v, e)
	})
	s.route("PUT", "/api/v1/orgs/{organizationId}/apps/{applicationId}/consent", "user", true, 200, body(false, func(r *request, v actionsRequest) (any, error) {
		match, err := conditionalHeader(r.Request, "If-Match")
		if err != nil {
			return nil, err
		}
		none, err := conditionalHeader(r.Request, "If-None-Match")
		if err != nil {
			return nil, err
		}
		return s.service.ReplaceConsentConditional(r.Context(), r.principal.UserID, r.principal.OrganizationID, r.PathValue("applicationId"), v.ActionIDs, match, none)
	}))
	s.route("DELETE", "/api/v1/orgs/{organizationId}/apps/{applicationId}/consent", "user", true, 204, func(r *request) (any, error) {
		return nil, s.service.RevokeApplicationConsent(r.Context(), r.principal.UserID, r.principal.OrganizationID, r.PathValue("applicationId"))
	})
	s.route("PUT", "/api/v1/admin/orgs/{organizationId}/apps/{applicationId}/trust", "master", true, 200, body(false, func(r *request, v trustRequest) (any, error) {
		return s.service.SetApplicationTrustAdmin(r.Context(), r.PathValue("organizationId"), r.PathValue("applicationId"), v.Trusted)
	}))
	// Operational bootstrap only: deliberately absent from the management catalog.
	s.route("PUT", "/api/v1/admin/orgs/{organizationId}/apps/{applicationId}/actions/{actionId}", "master", true, 204, func(r *request) (any, error) {
		return nil, s.service.AdmitFirstPartyApplicationActionAdmin(r.Context(), r.PathValue("organizationId"), r.PathValue("applicationId"), r.PathValue("actionId"))
	})
	s.route("GET", "/api/v1/admin/users", "master", true, 200, bodyless(s.listAdminUsers))
	s.route("PUT", "/api/v1/admin/users/{userId}/active", "master", true, 200, body(false, func(r *request, v activeRequest) (any, error) {
		return s.service.SetUserActiveAdmin(r.Context(), r.PathValue("userId"), v.Active)
	}))
	s.privateAdminAccountLinkRoutes()
	s.route("GET", "/api/v1/orgs/{organizationId}/groups", "user", false, 200, func(r *request) (any, error) {
		return s.service.ListGroupsAuthorized(r.Context(), r.principal.UserID, r.principal.OrganizationID, r.consistency)
	})
	s.route("POST", "/api/v1/orgs/{organizationId}/groups", "user", true, 201, body(false, func(r *request, v grant.UpsertGroupInput) (any, error) {
		return s.service.CreateGroupAuthorized(r.Context(), r.principal.UserID, r.principal.OrganizationID, v)
	}))
	s.route("GET", "/api/v1/orgs/{organizationId}/groups/{groupId}", "user", false, 200, func(r *request) (any, error) {
		return s.service.GetGroupAuthorized(r.Context(), r.principal.UserID, r.principal.OrganizationID, r.PathValue("groupId"), r.consistency)
	})
	s.route("PUT", "/api/v1/orgs/{organizationId}/groups/{groupId}", "user", true, 200, body(false, func(r *request, v updateGroupRequest) (any, error) {
		return s.service.UpdateGroupAuthorized(r.Context(), r.principal.UserID, r.principal.OrganizationID, r.PathValue("groupId"), grant.UpsertGroupInput{ID: r.PathValue("groupId"), Name: v.Name, Description: v.Description})
	}))
	s.route("DELETE", "/api/v1/orgs/{organizationId}/groups/{groupId}", "user", true, 204, func(r *request) (any, error) {
		return nil, s.service.DeleteGroupAuthorized(r.Context(), r.principal.UserID, r.principal.OrganizationID, r.PathValue("groupId"))
	})
	s.route("GET", "/api/v1/orgs/{organizationId}/groups/{groupId}/members", "user", false, 200, func(r *request) (any, error) {
		return s.service.ListGroupMembersAuthorized(r.Context(), r.principal.UserID, r.principal.OrganizationID, r.PathValue("groupId"), r.consistency)
	})
	s.route("PUT", "/api/v1/orgs/{organizationId}/groups/{groupId}/members/{userId}", "user", true, 200, func(r *request) (any, error) {
		return s.service.AddGroupMemberAuthorized(r.Context(), r.principal.UserID, r.principal.OrganizationID, r.PathValue("groupId"), r.PathValue("userId"))
	})
	s.route("DELETE", "/api/v1/orgs/{organizationId}/groups/{groupId}/members/{userId}", "user", true, 204, func(r *request) (any, error) {
		return nil, s.service.RemoveGroupMemberAuthorized(r.Context(), r.principal.UserID, r.principal.OrganizationID, r.PathValue("groupId"), r.PathValue("userId"))
	})
	s.route("PUT", "/api/v1/orgs/{organizationId}/members/{userId}", "user", true, 200, func(r *request) (any, error) {
		return s.service.EnrollOrganizationMemberAuthorized(r.Context(), r.principal.UserID, r.principal.OrganizationID, r.PathValue("userId"))
	})
	s.route("GET", "/api/v1/orgs/{organizationId}/agreements", "user", false, 200, func(r *request) (any, error) {
		return s.service.ListAgreementsAuthorized(r.Context(), r.principal.UserID, r.principal.OrganizationID, r.consistency)
	})
	s.route("POST", "/api/v1/orgs/{organizationId}/agreements", "user", true, 201, body(false, func(r *request, v grant.UpsertAgreementInput) (any, error) {
		return s.service.CreateAgreementAuthorized(r.Context(), r.principal.UserID, r.principal.OrganizationID, v)
	}))
	s.route("GET", "/api/v1/orgs/{organizationId}/agreements/{agreementId}", "user", false, 200, func(r *request) (any, error) {
		return s.service.GetAgreementAuthorized(r.Context(), r.principal.UserID, r.principal.OrganizationID, r.PathValue("agreementId"), r.consistency)
	})
	s.route("PUT", "/api/v1/orgs/{organizationId}/agreements/{agreementId}", "user", true, 200, body(false, func(r *request, v grant.UpsertAgreementInput) (any, error) {
		return s.service.UpdateAgreementAuthorized(r.Context(), r.principal.UserID, r.principal.OrganizationID, r.PathValue("agreementId"), v)
	}))
	s.route("GET", "/api/v1/orgs/{organizationId}/roles", "user", false, 200, func(r *request) (any, error) {
		return s.service.ListRolesAuthorized(r.Context(), r.principal.UserID, r.principal.OrganizationID, r.consistency)
	})
	s.route("POST", "/api/v1/orgs/{organizationId}/roles", "user", true, 201, body(false, func(r *request, v grant.CreateRoleInput) (any, error) {
		return s.service.CreateRoleAuthorized(r.Context(), r.principal.UserID, r.principal.OrganizationID, v)
	}))
	s.route("PUT", "/api/v1/orgs/{organizationId}/roles/{roleId}/actions", "user", true, 200, body(false, func(r *request, v actionsRequest) (any, error) {
		match, err := conditionalHeader(r.Request, "If-Match")
		if err != nil {
			return nil, err
		}
		return s.service.ReplaceRoleActionsConditional(r.Context(), r.principal.UserID, r.principal.OrganizationID, r.PathValue("roleId"), v.ActionIDs, match)
	}))
	s.route("GET", "/api/v1/orgs/{organizationId}/bindings", "user", false, 200, func(r *request) (any, error) {
		return s.service.ListRoleBindingsAuthorized(r.Context(), r.principal.UserID, r.principal.OrganizationID, r.consistency)
	})
	s.route("POST", "/api/v1/orgs/{organizationId}/bindings", "user", true, 201, body(false, func(r *request, v grant.CreateRoleBindingInput) (any, error) {
		return s.service.CreateRoleBindingAuthorized(r.Context(), r.principal.UserID, r.principal.OrganizationID, v)
	}))
	s.route("DELETE", "/api/v1/orgs/{organizationId}/bindings/{bindingId}", "user", true, 204, func(r *request) (any, error) {
		return nil, s.service.DeleteRoleBindingAuthorized(r.Context(), r.principal.UserID, r.principal.OrganizationID, r.PathValue("bindingId"))
	})
	s.route("POST", "/api/v1/orgs/{organizationId}/deactivate", "user", true, 200, func(r *request) (any, error) {
		return s.service.DeactivateOrganization(r.Context(), r.principal.UserID, r.principal.OrganizationID, r.PathValue("organizationId"))
	})
	s.route("POST", "/api/v1/orgs/{organizationId}/reactivate", "inactive", true, 200, func(r *request) (any, error) {
		return s.service.ReactivateOrganization(r.Context(), r.principal.UserID, r.principal.OrganizationID, r.PathValue("organizationId"))
	})
	s.route("POST", "/api/v1/admin/bootstrap/owner", "master", true, 201, func(r *request) (any, error) {
		token, e := bearerToken(r.Request)
		if e != nil {
			return nil, e
		}
		org, e := header(r.Request, "x-organization-id", true)
		if e != nil {
			return nil, e
		}
		v, e := s.service.BootstrapOwner(r.Context(), token, org)
		return map[string]string{"userId": v.UserID, "organizationId": v.OrganizationID}, e
	})
	s.route("POST", "/ops/backups", "master", true, 202, func(r *request) (any, error) { return nil, s.service.TriggerBackup(r.Context()) })
	s.route("GET", "/ops/backups", "master", true, 200, func(r *request) (any, error) {
		v, e := s.service.ListBackups(r.Context())
		return map[string]any{"scope": "logical-snapshots", "items": v}, e
	})
	s.authorizationRoutes()
	s.capabilityRoutes()
	s.route("GET", "/api/v1/admin/providers/{providerId}/http-capability-generations/{generation}", "master", true, 200, bodyless(s.httpCapabilityGeneration))
	s.actionMutationRoutes()
	s.lifecycleRoutes()
	if s.learner {
		for _, path := range []string{"/api/v1/admin/providers", "/api/v1/admin/providers/{providerId}/actions", "/api/v1/admin/providers/{providerId}/http-capability-generations", "/api/v1/admin/providers/{providerId}/http-capability-active", "/ops/providers/{providerId}/enforcers/{pepClientId}", "/ops/backups", "/api/v1/admin/bootstrap/owner", "/ops/migrations/patch-data-authority"} {
			s.mux.HandleFunc(path, func(w http.ResponseWriter, r *http.Request) {
				writeError(w, failure(503, "endpoint requires voter node"))
			})
		}
	}
}

type providerAction struct {
	ID                      string  `json:"id"`
	Code                    string  `json:"code"`
	Name                    string  `json:"name"`
	Description             *string `json:"description"`
	RiskLevel               string  `json:"riskLevel"`
	VisibilityGranting      bool    `json:"visibilityGranting"`
	GrantableToApplications bool    `json:"grantableToApplications"`
	ConsentLabel            *string `json:"consentLabel"`
	ConsentDescription      *string `json:"consentDescription"`
}
type providerActions struct {
	Actions []providerAction `json:"actions"`
}

func (s *Server) replaceProviderActions(r *request, v providerActions) (any, error) {
	out := make([]grant.Action, len(v.Actions))
	for i, a := range v.Actions {
		if a.VisibilityGranting {
			return nil, failure(400, "visibilityGranting=true is not supported by the coarse authorization API")
		}
		out[i] = grant.Action{ID: a.ID, ProviderID: r.PathValue("providerId"), Code: a.Code, Name: a.Name, Description: a.Description, RiskLevel: a.RiskLevel, VisibilityGranting: a.VisibilityGranting, GrantableToApplications: a.GrantableToApplications, ConsentLabel: a.ConsentLabel, ConsentDescription: a.ConsentDescription}
	}
	match, err := conditionalHeader(r.Request, "If-Match")
	if err != nil {
		return nil, err
	}
	return s.service.ReplaceProviderActionsConditional(r.Context(), r.PathValue("providerId"), out, match)
}
