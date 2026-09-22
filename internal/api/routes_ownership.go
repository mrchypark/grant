package api

import (
	"encoding/json"
	"github.com/Conalog/grant/internal/grant"
)

func ownerSnapshotResult(v grant.OrganizationOwnerSnapshot, err error) (any, error) {
	if err != nil {
		return nil, err
	}
	encoded, err := json.Marshal(v)
	if err != nil {
		return nil, err
	}
	return snapshotResponse{grant.ActionSnapshot{Value: encoded, ETag: v.ETag}}, nil
}
func (s *Server) ownershipRoutes() {
	s.route("GET", "/api/v1/orgs/{organizationId}/owner", "user", false, 200, func(r *request) (any, error) {
		return ownerSnapshotResult(s.service.GetOrganizationOwnerAuthorized(r.Context(), r.principal.UserID, r.principal.OrganizationID))
	})
	s.route("PUT", "/api/v1/orgs/{organizationId}/owner", "user", true, 200, lifecycleBody([]string{"ownerUserId"}, func(r *request, v struct {
		OwnerUserID string `json:"ownerUserId"`
	}) (any, error) {
		match, err := conditionalHeader(r.Request, "If-Match")
		if err != nil {
			return nil, err
		}
		return ownerSnapshotResult(s.service.TransferOrganizationOwnerAuthorized(r.Context(), r.principal.UserID, r.principal.OrganizationID, v.OwnerUserID, match))
	}))
}
