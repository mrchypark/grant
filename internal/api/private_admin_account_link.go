package api

import "github.com/Conalog/grant/internal/grant"

// goauthyIdentityLinkRequest is intentionally closed: this private master-only
// operation accepts only an operator-attested identity tuple and matching email.
type goauthyIdentityLinkRequest struct {
	ExistingUserID string `json:"existingUserId"`
	Issuer         string `json:"issuer"`
	Subject        string `json:"subject"`
	ExpectedEmail  string `json:"expectedEmail"`
}

func (s *Server) privateAdminAccountLinkRoutes() {
	s.route("POST", "/api/v1/admin/identity-links/goauthy", "master", true, 200, body(true, func(r *request, v goauthyIdentityLinkRequest) (any, error) {
		in := grant.GoauthyIdentityLinkInput{ExistingUserID: v.ExistingUserID, Issuer: v.Issuer, Subject: v.Subject, ExpectedEmail: v.ExpectedEmail}
		return map[string]string{"userId": in.ExistingUserID}, s.service.LinkGoauthyIdentityAdmin(r.Context(), in)
	}))
}
