package api

import (
	"context"
	"net/http"
	"strings"

	"github.com/mrchypark/grant/internal/grant"
)

const providerUserBatchPath = "/api/v2/providers/{providerId}/authz/batch-check"
const providerUserFilterPath = "/api/v2/providers/{providerId}/authz/filter"

func (s *Server) providerUserAuthorizationRead(r *http.Request) bool {
	_, pattern := s.mux.Handler(r)
	return pattern == "POST "+providerUserBatchPath || pattern == "POST "+providerUserFilterPath
}

func (s *Server) providerUserAuthorizationRoutes() {
	s.route("POST", providerUserBatchPath, "application", false, 200, providerUserAuthorizationBody(func(r *request, v applicationBatch) (any, error) {
		return providerUserChecks(r.Context(), s.service, r.bearer, r.org, r.PathValue("providerId"), v.Checks)
	}))
	s.route("POST", providerUserFilterPath, "application", false, 200, providerUserAuthorizationBody(func(r *request, v applicationFilter) (any, error) {
		resources := make([]grant.AuthorizationResource, len(v.Resources))
		for i, resource := range v.Resources {
			var err error
			resources[i], err = resource.input()
			if err != nil {
				return nil, err
			}
		}
		out, err := s.service.UserFilter(r.Context(), r.bearer, r.org, r.PathValue("providerId"), v.Action, resources)
		if err == nil && (out.PEPPolicyRevision <= 0 || out.ResourceIDs == nil) {
			return nil, failure(503, "incomplete provider user authorization result")
		}
		return out, err
	}))
}

func providerUserChecks(ctx context.Context, service *grant.Service, bearer, org, provider string, values []applicationCheck) (grant.UserBatchCheckResult, error) {
	checks := make([]grant.ApplicationAuthorizationCheck, len(values))
	for i, value := range values {
		resource, err := value.Resource.input()
		if err != nil {
			return grant.UserBatchCheckResult{}, err
		}
		checks[i] = grant.ApplicationAuthorizationCheck{Action: value.Action, Resource: resource}
	}
	out, err := service.UserBatchCheck(ctx, bearer, org, provider, checks)
	if err == nil && (out.Results == nil || len(out.Results) != len(checks) || out.PEPPolicyRevision <= 0) {
		return grant.UserBatchCheckResult{}, failure(503, "incomplete provider user authorization result")
	}
	return out, err
}

func validProviderUserID(provider string) bool {
	return provider != "" && len(provider) <= 256 && !strings.ContainsFunc(provider, func(c rune) bool {
		return !(c >= 'a' && c <= 'z' || c >= 'A' && c <= 'Z' || c >= '0' && c <= '9' || strings.ContainsRune("._:-", c))
	})
}

func validateProviderUserChecks(provider string, checks []applicationCheck) error {
	if checks == nil {
		return failure(422, "checks is required")
	}
	if len(checks) > 1000 {
		return failure(400, "checks must contain at most 1000 items")
	}
	for _, check := range checks {
		if _, err := check.Resource.input(); err != nil {
			return err
		}
		for _, value := range []string{check.Action, check.Resource.Provider, check.Resource.Type, check.Resource.ID} {
			if strings.TrimSpace(value) == "" {
				return failure(422, "action and resource provider, type, id are required")
			}
		}
		if check.Resource.Provider != provider {
			return failure(400, "resource provider must match path provider")
		}
	}
	return nil
}

func providerUserAuthorizationBody[T any](fn func(*request, T) (any, error)) preparedEndpoint {
	return preparedEndpoint{prepare: func(r *request) (endpoint, error) {
		provider := r.PathValue("providerId")
		if !validProviderUserID(provider) {
			return nil, failure(422, "invalid providerId")
		}
		for _, name := range []string{"Authorization", "X-Grant-App-Auth-Key", "X-Organization-ID"} {
			if len(r.Header.Values(name)) > 1 {
				return nil, failure(400, "multiple "+name+" headers")
			}
		}
		v, err := decode[T](r.Request, true)
		if err != nil {
			return nil, err
		}
		var checks []applicationCheck
		switch value := any(v).(type) {
		case applicationBatch:
			checks = value.Checks
		case applicationFilter:
			if value.Resources == nil || strings.TrimSpace(value.Action) == "" {
				return nil, failure(422, "action and resources are required")
			}
			checks = make([]applicationCheck, len(value.Resources))
			for i, resource := range value.Resources {
				checks[i] = applicationCheck{Action: value.Action, Resource: resource}
			}
		}
		if err := validateProviderUserChecks(provider, checks); err != nil {
			return nil, err
		}
		return func(r *request) (any, error) { return fn(r, v) }, nil
	}}
}
