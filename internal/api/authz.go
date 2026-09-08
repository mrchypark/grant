package api

import (
	"encoding/json"
	"fmt"
	"github.com/Conalog/grant/internal/grant"
	"strings"
)

type subject struct {
	Type     string  `json:"type"`
	ID       string  `json:"id"`
	OrgID    *string `json:"org_id"`
	OrgAlias *string `json:"orgId"`
}

func (v subject) input() (grant.AuthorizationSubject, error) {
	if (v.OrgID == nil) == (v.OrgAlias == nil) {
		return grant.AuthorizationSubject{}, failure(422, "subject requires exactly one org_id or orgId")
	}
	org := v.OrgID
	if org == nil {
		org = v.OrgAlias
	}
	return grant.AuthorizationSubject{SubjectType: v.Type, ID: v.ID, OrganizationID: *org}, nil
}

type resource struct {
	Provider string          `json:"provider"`
	Type     string          `json:"type"`
	ID       string          `json:"id"`
	View     json.RawMessage `json:"view,omitempty"`
}

func (v resource) input() (grant.AuthorizationResource, error) {
	if len(v.View) > 0 {
		return grant.AuthorizationResource{}, failure(400, "resource.view is not supported by the coarse authorization API")
	}
	return grant.AuthorizationResource{ProviderID: v.Provider, ResourceType: v.Type, ID: v.ID}, nil
}

type check struct {
	Subject  subject  `json:"subject"`
	Action   string   `json:"action"`
	Resource resource `json:"resource"`
}
type applicationCheck struct {
	Action   string   `json:"action"`
	Resource resource `json:"resource"`
}
type batch struct {
	Checks []check `json:"checks"`
}
type applicationBatch struct {
	Checks []applicationCheck `json:"checks"`
}
type filter struct {
	Subject   subject    `json:"subject"`
	Action    string     `json:"action"`
	Resources []resource `json:"resources"`
}
type applicationFilter struct {
	Action    string     `json:"action"`
	Resources []resource `json:"resources"`
}

func (s *Server) checks(r *request, values []check) ([]grant.AuthorizationDecision, error) {
	if len(values) > 1000 {
		return nil, failure(400, "checks must contain at most 1000 items")
	}
	in := make([]grant.AuthorizationCheck, len(values))
	for i, v := range values {
		res, e := v.Resource.input()
		if e != nil {
			return nil, e
		}
		sub, e := v.Subject.input()
		if e != nil {
			return nil, e
		}
		in[i] = grant.AuthorizationCheck{Subject: sub, Action: v.Action, Resource: res}
	}
	out, e := s.service.CheckAuthorizations(r.Context(), in, r.consistency)
	if e == nil && len(out) != len(in) {
		e = failure(500, "authorization batch result length mismatch")
	}
	return out, e
}
func (s *Server) applicationChecks(r *request, values []applicationCheck) ([]grant.AuthorizationDecision, error) {
	if len(values) > 1000 {
		return nil, failure(400, "checks must contain at most 1000 items")
	}
	in := make([]grant.ApplicationAuthorizationCheck, len(values))
	for i, v := range values {
		res, e := v.Resource.input()
		if e != nil {
			return nil, e
		}
		in[i] = grant.ApplicationAuthorizationCheck{Action: v.Action, Resource: res}
	}
	out, e := s.service.CheckApplicationAuthorizations(r.Context(), r.bearer, r.org, in)
	if e == nil && len(out) != len(in) {
		e = failure(500, "application authorization batch result length mismatch")
	}
	return out, e
}
func first(v []grant.AuthorizationDecision, e error) (any, error) {
	if e != nil {
		return nil, e
	}
	if len(v) != 1 {
		return nil, failure(500, "authorization check failed")
	}
	return v[0], nil
}
func (s *Server) authorizationRoutes() {
	s.route("POST", "/api/v1/authz/check", "master", false, 200, body(false, func(r *request, v check) (any, error) { return first(s.checks(r, []check{v})) }))
	s.route("POST", "/api/v1/authz/batch-check", "master", false, 200, body(false, func(r *request, v batch) (any, error) {
		out, e := s.checks(r, v.Checks)
		return map[string]any{"results": out}, e
	}))
	s.route("POST", "/api/v1/authz/filter", "master", false, 200, body(false, func(r *request, v filter) (any, error) {
		if len(v.Resources) > 1000 {
			return nil, failure(400, "resources must contain at most 1000 items")
		}
		checks := make([]check, len(v.Resources))
		for i, res := range v.Resources {
			checks[i] = check{v.Subject, v.Action, res}
		}
		out, e := s.checks(r, checks)
		if e != nil {
			return nil, e
		}
		ids := []string{}
		for i, d := range out {
			if d.Allowed {
				ids = append(ids, v.Resources[i].ID)
			}
		}
		return map[string]any{"resource_ids": ids}, nil
	}))
	s.route("POST", "/api/v1/application-authz/check", "application", false, 200, body(false, func(r *request, v applicationCheck) (any, error) {
		return first(s.applicationChecks(r, []applicationCheck{v}))
	}))
	s.route("POST", "/api/v1/application-authz/batch-check", "application", false, 200, body(false, func(r *request, v applicationBatch) (any, error) {
		out, e := s.applicationChecks(r, v.Checks)
		return map[string]any{"results": out}, e
	}))
	s.route("POST", "/api/v1/application-authz/filter", "application", false, 200, body(false, func(r *request, v applicationFilter) (any, error) {
		if len(v.Resources) > 1000 {
			return nil, failure(400, "resources must contain at most 1000 items")
		}
		checks := make([]applicationCheck, len(v.Resources))
		for i, res := range v.Resources {
			checks[i] = applicationCheck{v.Action, res}
		}
		out, e := s.applicationChecks(r, checks)
		if e != nil {
			return nil, e
		}
		ids := []string{}
		for i, d := range out {
			if d.Allowed {
				ids = append(ids, v.Resources[i].ID)
			}
		}
		return map[string]any{"resource_ids": ids}, nil
	}))
	s.route("POST", "/api/v1/exposure-authz/read", "application", false, 200, body(true, func(r *request, v exposureRequest) (any, error) {
		if e := v.validate(); e != nil {
			return nil, e
		}
		d, e := s.service.CheckExposureReadAuthorization(r.Context(), r.bearer, r.org, v.ExposureID, v.ResourceID)
		return exposureResponse(v, d), e
	}))
	s.route("POST", "/api/v1/exposure-authz/plant-read", "application", false, 200, body(true, func(r *request, v plantRequest) (any, error) {
		base := exposureRequest{v.ExposureID, v.ExposureVersion, v.ArtifactDigest, v.ResourceID, v.RequestBinding}
		if e := base.validate(); e != nil {
			return nil, e
		}
		if v.OperationID != "plant.get" {
			return nil, failure(400, "operationId must be plant.get")
		}
		d, e := s.service.CheckPatchDataPlantReadAuthorization(r.Context(), r.bearer, r.org, v.ResourceID)
		out := exposureResponse(base, d)
		out["operationId"] = "plant.get"
		out["providerId"] = "patch-data"
		out["actionId"] = "patch-data.plant.read"
		out["resourceType"] = "plant"
		return out, e
	}))
	s.route("POST", "/api/v2/exposure-authz/decide", "application", false, 200, body(true, func(r *request, v operationRequest) (any, error) {
		if v.OperationID != "plant.get" {
			return nil, failure(400, "unknown operationId")
		}
		if v.OperationVersion != 1 || v.ContractDigest != plantContract {
			return nil, failure(400, "operation contract mismatch")
		}
		if e := exact("resource.id", v.Resource.ID); e != nil {
			return nil, e
		}
		if e := exact("requestBinding", v.RequestBinding); e != nil {
			return nil, e
		}
		d, e := s.service.CheckOperationAuthorization(r.Context(), r.bearer, r.org, grant.OperationAuthorizationTarget{ProviderID: "patch-data", ActionID: "patch-data.plant.read", ResourceType: "plant", ResourceID: v.Resource.ID})
		return map[string]any{"allowed": d.Allowed, "reason": d.Reason, "operationId": "plant.get", "operationVersion": 1, "contractDigest": plantContract, "providerId": "patch-data", "actionId": "patch-data.plant.read", "resource": map[string]string{"type": "plant", "id": v.Resource.ID}, "requestBinding": v.RequestBinding, "principalDigest": d.PrincipalDigest, "issuedAt": d.IssuedAt, "expiresAt": d.ExpiresAt}, e
	}))
}

const plantContract = "sha256:7ca72b56a8f32ab247d4d8dc936b30cedf2f9995c1ad8cc5b2f2703dc20f488d"

type exposureRequest struct {
	ExposureID      string `json:"exposureId"`
	ExposureVersion uint64 `json:"exposureVersion"`
	ArtifactDigest  string `json:"artifactDigest"`
	ResourceID      string `json:"resourceId"`
	RequestBinding  string `json:"requestBinding"`
}
type plantRequest struct {
	ExposureID      string `json:"exposureId"`
	ExposureVersion uint64 `json:"exposureVersion"`
	ArtifactDigest  string `json:"artifactDigest"`
	ResourceID      string `json:"resourceId"`
	RequestBinding  string `json:"requestBinding"`
	OperationID     string `json:"operationId"`
}
type operationRequest struct {
	OperationID      string `json:"operationId"`
	OperationVersion uint64 `json:"operationVersion"`
	ContractDigest   string `json:"contractDigest"`
	Resource         struct {
		ID string `json:"id"`
	} `json:"resource"`
	RequestBinding string `json:"requestBinding"`
}

func exact(name, value string) error {
	if value == "" || strings.TrimSpace(value) != value {
		return failure(400, fmt.Sprintf("%s must be non-empty and must not have surrounding whitespace", name))
	}
	return nil
}
func (v exposureRequest) validate() error {
	for _, p := range [][2]string{{"exposureId", v.ExposureID}, {"resourceId", v.ResourceID}, {"requestBinding", v.RequestBinding}} {
		if e := exact(p[0], p[1]); e != nil {
			return e
		}
	}
	if v.ExposureVersion == 0 {
		return failure(400, "exposureVersion must be greater than zero")
	}
	if len(v.ArtifactDigest) != 71 || !strings.HasPrefix(v.ArtifactDigest, "sha256:") || strings.Trim(v.ArtifactDigest[7:], "0123456789abcdef") != "" {
		return failure(400, "artifactDigest must be the literal sha256: followed by exactly 64 lowercase hexadecimal characters")
	}
	return nil
}
func exposureResponse(v exposureRequest, d grant.ExposureReadAuthorizationDecision) map[string]any {
	return map[string]any{"allowed": d.Allowed, "reason": d.Reason, "exposureId": v.ExposureID, "exposureVersion": v.ExposureVersion, "artifactDigest": v.ArtifactDigest, "resourceId": v.ResourceID, "requestBinding": v.RequestBinding, "principalDigest": d.PrincipalDigest, "issuedAt": d.IssuedAt, "expiresAt": d.ExpiresAt}
}
