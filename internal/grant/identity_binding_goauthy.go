package grant

import (
	"context"
	"crypto/sha256"
	"encoding/hex"
	"fmt"
	"io"
	"mime"
	"net/http"
	"sort"
	"strings"
)

func (s *Service) verifyGoauthyIdentityBinding(ctx context.Context, a Application, clientID string, uri *string) error {
	cfg := s.cfg.Auth.Goauthy
	if cfg == nil || a.ID != cfg.ApplicationID || clientID != cfg.IntrospectionClientID || clientID != cfg.ExpectedClientID {
		return serviceError(409, "Goauthy bootstrap client cannot verify this identity binding")
	}
	issuer, err := canonicalGoauthyOrigin(cfg.Issuer)
	if err != nil || issuer != cfg.Issuer {
		return serviceError(503, "invalid Goauthy configuration")
	}
	resource, err := canonicalGoauthyResource(cfg.Resource)
	if err != nil || resource != cfg.Resource {
		return serviceError(503, "invalid Goauthy configuration")
	}
	req, err := http.NewRequestWithContext(ctx, http.MethodGet, issuer+"/oidc/client-configuration", nil)
	if err != nil {
		return serviceError(503, "invalid Goauthy configuration")
	}
	req.SetBasicAuth(cfg.IntrospectionClientID, cfg.IntrospectionClientSecret)
	// Goauthy hashes the exact response bytes for ETag; don't let transport gzip alter them.
	req.Header.Set("Accept-Encoding", "identity")
	client := *s.client
	client.CheckRedirect = func(*http.Request, []*http.Request) error { return http.ErrUseLastResponse }
	transport := client.Transport
	if transport == nil {
		transport = http.DefaultTransport
	}
	if base, ok := transport.(*http.Transport); ok {
		clone := base.Clone()
		clone.Proxy = nil
		client.Transport = clone
	}
	defer client.CloseIdleConnections()
	res, err := client.Do(req)
	if err != nil {
		return serviceError(503, "Goauthy client configuration fetch failed")
	}
	defer res.Body.Close()
	if res.StatusCode < 200 || res.StatusCode >= 300 {
		return serviceError(503, "Goauthy client configuration fetch failed")
	}
	mediaType, _, err := mime.ParseMediaType(res.Header.Get("Content-Type"))
	if err != nil || !strings.EqualFold(mediaType, "application/json") || len(res.Header.Values("Content-Type")) != 1 {
		return serviceError(503, "invalid Goauthy client configuration media type")
	}
	body, err := io.ReadAll(io.LimitReader(res.Body, 65537))
	if err != nil || len(body) > 65536 {
		return serviceError(503, "invalid Goauthy client configuration")
	}
	digest := sha256.Sum256(body)
	if len(res.Header.Values("ETag")) != 1 || res.Header.Get("ETag") != fmt.Sprintf(`"sha256:%s"`, hex.EncodeToString(digest[:])) || len(res.Header.Values("Cache-Control")) != 1 || !strings.EqualFold(strings.TrimSpace(res.Header.Get("Cache-Control")), "no-store") {
		return serviceError(503, "invalid Goauthy client configuration validator")
	}
	fields, err := goauthyJSONObject(body, "Goauthy")
	if err != nil {
		return serviceError(503, "invalid Goauthy client configuration")
	}
	var v struct {
		Version       string   `json:"version"`
		Issuer        string   `json:"issuer"`
		ClientID      string   `json:"client_id"`
		ClientKind    string   `json:"client_kind"`
		Enabled       bool     `json:"enabled"`
		Confidential  bool     `json:"confidential"`
		RedirectURIs  []string `json:"redirect_uris"`
		GrantTypes    []string `json:"grant_types"`
		Scopes        []string `json:"scopes"`
		DefaultScopes []string `json:"default_scopes"`
		Audience      []string `json:"audience"`
		DefaultAud    []string `json:"default_aud"`
	}
	if len(fields) != 12 || !goauthyField(fields, "version", &v.Version) || !goauthyField(fields, "issuer", &v.Issuer) || !goauthyField(fields, "client_id", &v.ClientID) || !goauthyField(fields, "client_kind", &v.ClientKind) || !goauthyField(fields, "enabled", &v.Enabled) || !goauthyField(fields, "confidential", &v.Confidential) || !goauthyField(fields, "redirect_uris", &v.RedirectURIs) || !goauthyField(fields, "grant_types", &v.GrantTypes) || !goauthyField(fields, "scopes", &v.Scopes) || !goauthyField(fields, "default_scopes", &v.DefaultScopes) || !goauthyField(fields, "audience", &v.Audience) || !goauthyField(fields, "default_aud", &v.DefaultAud) {
		return serviceError(503, "invalid Goauthy client configuration")
	}
	for _, name := range []string{"redirect_uris", "grant_types", "scopes", "default_scopes", "audience", "default_aud"} {
		if strings.TrimSpace(string(fields[name]))[0] != '[' {
			return serviceError(503, "invalid Goauthy client configuration")
		}
	}
	for _, values := range [][]string{v.RedirectURIs, v.GrantTypes, v.Scopes, v.DefaultScopes, v.Audience, v.DefaultAud} {
		if !uniqueGoauthyStrings(values) {
			return serviceError(503, "invalid Goauthy client configuration")
		}
	}
	expectedRedirects := []string{}
	if uri != nil {
		canonical, e := CanonicalRedirectURI(*uri)
		if e != nil || canonical != *uri {
			return serviceError(409, "local application redirect URI manifest is invalid")
		}
		expectedRedirects = append(expectedRedirects, *uri)
	}
	for _, scope := range cfg.RequiredScopes {
		if !containsGoauthyString(v.Scopes, scope) {
			return serviceError(409, "Goauthy client configuration does not authorize Grant scopes")
		}
	}
	for _, scope := range v.DefaultScopes {
		if !containsGoauthyString(v.Scopes, scope) {
			return serviceError(503, "invalid Goauthy client configuration")
		}
	}
	if !containsGoauthyString(v.GrantTypes, "authorization_code") || !containsGoauthyString(v.GrantTypes, "refresh_token") {
		return serviceError(409, "Goauthy bootstrap configuration does not support Grant login flows")
	}
	if v.Version != "goauthy.client-configuration.v1" || v.Issuer != issuer || v.ClientID != clientID || v.ClientKind != "bootstrap" || !v.Enabled || !v.Confidential || v.Confidential != (a.ApplicationType == "confidential") || !a.Active || !sameGoauthyStrings(v.RedirectURIs, expectedRedirects) || !containsGoauthyString(v.Audience, resource) || !sameGoauthyStrings(v.DefaultAud, []string{resource}) {
		return serviceError(409, "Goauthy bootstrap configuration does not match the local application requirements")
	}
	return nil
}

func sameGoauthyStrings(a, b []string) bool {
	if len(a) != len(b) {
		return false
	}
	left, right := append([]string(nil), a...), append([]string(nil), b...)
	sort.Strings(left)
	sort.Strings(right)
	for i := range left {
		if left[i] != right[i] {
			return false
		}
	}
	return true
}

func uniqueGoauthyStrings(values []string) bool {
	seen := make(map[string]bool, len(values))
	for _, value := range values {
		if strings.TrimSpace(value) == "" || seen[value] {
			return false
		}
		seen[value] = true
	}
	return true
}

func containsGoauthyString(values []string, want string) bool {
	for _, value := range values {
		if value == want {
			return true
		}
	}
	return false
}
