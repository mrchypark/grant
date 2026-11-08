package api

import (
	"net/http"
	"strings"
	"time"

	"github.com/mrchypark/grant/internal/grant"
)

const (
	grpcCatalogPrefix  = "/api/v2/grpc-capabilities/catalog/"
	grpcCatalogPath    = grpcCatalogPrefix + "{providerId}"
	grpcDecisionPath   = "/api/v2/grpc-capabilities/decide"
	grpcGenerationPath = "/api/v1/admin/providers/{providerId}/grpc-capability-generations"
	grpcActivationPath = "/api/v1/admin/providers/{providerId}/grpc-capability-active"
	grpcBodyTimeout    = 5 * time.Second
)

type grpcBodyEndpoint struct{ preparedEndpoint }

func grpcBody[T any](fn func(*request, T) (any, error)) grpcBodyEndpoint {
	return grpcBodyEndpoint{body(true, fn)}
}

func grpcCapabilityRead(r *http.Request) bool {
	return (r.Method == http.MethodPost && r.URL.Path == grpcDecisionPath) ||
		((r.Method == http.MethodGet || r.Method == http.MethodHead) && strings.HasPrefix(r.URL.Path, grpcCatalogPrefix))
}

func (s *Server) grpcCapabilityRoutes() {
	s.route("GET", grpcCatalogPath, "app", false, 200, func(r *request) (any, error) {
		return s.service.ActiveGRPCCapabilityCatalog(r.Context(), r.PathValue("providerId"))
	})
	s.route("POST", grpcDecisionPath, "grpc-decision", false, 200, grpcBody(func(r *request, v grant.GRPCCapabilityDecisionInput) (any, error) {
		return s.service.DecideGRPCCapability(r.Context(), r.bearer, r.org, v)
	}))
	s.route("POST", grpcGenerationPath, "master", true, 201, grpcBody(func(r *request, v grant.StageGRPCCapabilityGeneration) (any, error) {
		return s.service.StageGRPCCapabilityGenerationAdmin(r.Context(), r.PathValue("providerId"), v)
	}))
	s.route("POST", grpcActivationPath, "master", true, 200, grpcBody(func(r *request, v grant.ActivateGRPCCapabilityGeneration) (any, error) {
		return s.service.ActivateGRPCCapabilityGenerationAdmin(r.Context(), r.PathValue("providerId"), v)
	}))
}
