package api

import (
	"encoding/json"
	"github.com/mrchypark/grant/internal/grant"
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
}
