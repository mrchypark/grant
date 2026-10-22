package api

import (
	"bytes"
	"encoding/json"
	"reflect"
	"strings"

	"github.com/mrchypark/grant/internal/grant"
)

func decodeLifecycleFields(encoded []byte, value any) error {
	decoder := json.NewDecoder(bytes.NewReader(encoded))
	decoder.DisallowUnknownFields()
	if err := decoder.Decode(value); err != nil {
		return failure(422, "invalid lifecycle request: "+err.Error())
	}
	return nil
}

// Patch DTO pointers preserve omission. Validate explicit null separately so a
// malformed non-nullable value cannot silently turn into an omitted field.
func lifecycleBody[T any](requiredValues []string, fn func(*request, T) (any, error)) preparedEndpoint {
	return body(true, func(r *request, fields map[string]json.RawMessage) (any, error) {
		shape := reflect.TypeFor[T]()
		allowed := make(map[string]bool, shape.NumField())
		for i := 0; i < shape.NumField(); i++ {
			allowed[strings.Split(shape.Field(i).Tag.Get("json"), ",")[0]] = true
		}
		for name := range fields {
			if !allowed[name] {
				return nil, failure(422, "unknown or immutable lifecycle field: "+name)
			}
		}
		for _, name := range requiredValues {
			if raw, ok := fields[name]; ok && bytes.Equal(bytes.TrimSpace(raw), []byte("null")) {
				return nil, failure(422, name+" must not be null")
			}
		}
		encoded, err := json.Marshal(fields)
		if err != nil {
			return nil, err
		}
		var value T
		// The outer map decoder rejects duplicate keys; the typed decoder rejects
		// unknown fields, including immutable ownership and security attributes.
		if err = decodeLifecycleFields(encoded, &value); err != nil {
			return nil, err
		}
		return fn(r, value)
	})
}

func (s *Server) lifecycleRoutes() {
	s.route("POST", "/api/v1/orgs", "bearer", true, 201, body(true, func(r *request, v grant.SelfOrganizationInput) (any, error) {
		return s.service.CreateOwnOrganization(r.Context(), r.bearer, v)
	}))
	s.ownershipRoutes()
	s.route("PATCH", "/api/v1/admin/providers/{providerId}", "master", true, 200, lifecycleBody([]string{"name", "active"}, func(r *request, v grant.PatchServiceProviderInput) (any, error) {
		return s.service.PatchServiceProviderAdmin(r.Context(), r.PathValue("providerId"), v)
	}))
	s.route("PUT", "/api/v1/admin/orgs/{organizationId}/apps/{applicationId}/identity-binding", "master", true, 200, body(true, func(r *request, v grant.IdentityBindingInput) (any, error) {
		return s.service.BindApplicationIdentityAdmin(r.Context(), r.PathValue("organizationId"), r.PathValue("applicationId"), v)
	}))
	s.route("POST", "/api/v1/admin/orgs", "master", true, 201, body(true, func(r *request, v grant.CreateOrganizationInput) (any, error) {
		return s.service.CreateOrganizationAdmin(r.Context(), v)
	}))
	s.route("GET", "/api/v1/orgs", "bearer", false, 200, func(r *request) (any, error) {
		current, err := s.service.CurrentUser(r.Context(), r.bearer, "", r.consistency)
		if err != nil {
			return nil, err
		}
		return s.service.ListOrganizationsAuthorized(r.Context(), current.User.ID, r.consistency)
	})
	s.route("GET", "/api/v1/orgs/{organizationId}", "user", false, 200, func(r *request) (any, error) {
		return s.service.GetOrganizationAuthorized(r.Context(), r.principal.UserID, r.principal.OrganizationID, r.consistency)
	})
	s.route("PATCH", "/api/v1/orgs/{organizationId}", "user", true, 200, lifecycleBody([]string{"name"}, func(r *request, v grant.PatchOrganizationInput) (any, error) {
		return s.service.PatchOrganizationAuthorized(r.Context(), r.principal.UserID, r.principal.OrganizationID, v)
	}))
	s.route("PATCH", "/api/v1/orgs/{organizationId}/groups/{groupId}", "user", true, 200, lifecycleBody([]string{"name"}, func(r *request, v grant.PatchGroupInput) (any, error) {
		return s.service.PatchGroupAuthorized(r.Context(), r.principal.UserID, r.principal.OrganizationID, r.PathValue("groupId"), v)
	}))
	s.route("PATCH", "/api/v1/orgs/{organizationId}/agreements/{agreementId}", "user", true, 200, lifecycleBody([]string{"title", "agreementType", "status"}, func(r *request, v grant.PatchAgreementInput) (any, error) {
		return s.service.PatchAgreementAuthorized(r.Context(), r.principal.UserID, r.principal.OrganizationID, r.PathValue("agreementId"), v)
	}))
	s.route("PATCH", "/api/v1/orgs/{organizationId}/apps/{applicationId}", "user", true, 200, lifecycleBody([]string{"name"}, func(r *request, v grant.MetadataPatch) (any, error) {
		return s.service.PatchApplicationAuthorized(r.Context(), r.principal.UserID, r.principal.OrganizationID, r.PathValue("applicationId"), v)
	}))
	s.route("PUT", "/api/v1/orgs/{organizationId}/apps/{applicationId}/active", "user", true, 200, body(true, func(r *request, v struct {
		Active *bool `json:"active"`
	}) (any, error) {
		if v.Active == nil {
			return nil, failure(422, "active must be boolean")
		}
		return s.service.SetApplicationActiveAuthorized(r.Context(), r.principal.UserID, r.principal.OrganizationID, r.PathValue("applicationId"), *v.Active)
	}))
	s.route("DELETE", "/api/v1/orgs/{organizationId}/apps/{applicationId}", "user", true, 204, func(r *request) (any, error) {
		return nil, s.service.DeleteApplicationAuthorized(r.Context(), r.principal.UserID, r.principal.OrganizationID, r.PathValue("applicationId"))
	})
	s.route("PATCH", "/api/v1/orgs/{organizationId}/roles/{roleId}", "user", true, 200, lifecycleBody([]string{"name"}, func(r *request, v grant.MetadataPatch) (any, error) {
		return s.service.PatchRoleAuthorized(r.Context(), r.principal.UserID, r.principal.OrganizationID, r.PathValue("roleId"), v)
	}))
	s.route("DELETE", "/api/v1/orgs/{organizationId}/roles/{roleId}", "user", true, 204, func(r *request) (any, error) {
		return nil, s.service.DeleteRoleAuthorized(r.Context(), r.principal.UserID, r.principal.OrganizationID, r.PathValue("roleId"))
	})
	s.route("GET", "/api/v1/orgs/{organizationId}/members", "user", false, 200, func(r *request) (any, error) {
		return s.service.ListOrganizationMembersAuthorized(r.Context(), r.principal.UserID, r.principal.OrganizationID, r.consistency)
	})
	s.route("DELETE", "/api/v1/orgs/{organizationId}/members/{userId}", "user", true, 204, func(r *request) (any, error) {
		return nil, s.service.RemoveOrganizationMemberAuthorized(r.Context(), r.principal.UserID, r.principal.OrganizationID, r.PathValue("userId"))
	})
	s.route("DELETE", "/api/v1/orgs/{organizationId}/app-grants/{grantId}", "user", true, 204, func(r *request) (any, error) {
		return nil, s.service.RevokeApplicationGrantAuthorized(r.Context(), r.principal.UserID, r.principal.OrganizationID, r.PathValue("grantId"))
	})
}
