package grant

import (
	"bytes"
	"context"
	"encoding/json"
	"io"
	"net/http"
	"net/url"
	"strings"
	"time"

	whatwg "github.com/nlnwa/whatwg-url/url"
)

func canonicalGoauthyOrigin(value string) (string, error) {
	u, err := url.Parse(value)
	if err != nil || u.Scheme != "https" || u.Host == "" || u.User != nil || u.RawQuery != "" || u.ForceQuery || u.Fragment != "" || (u.Path != "" && u.Path != "/") {
		return "", serviceError(400, "invalid Goauthy issuer origin")
	}
	p, err := whatwg.Parse(value)
	if err != nil {
		return "", err
	}
	canonical := strings.TrimSuffix(p.Href(false), "/")
	if canonical != value {
		return "", serviceError(400, "Goauthy issuer must be canonical")
	}
	return canonical, nil
}

func canonicalGoauthyResource(value string) (string, error) {
	u, err := url.Parse(value)
	if err != nil || u.Scheme != "https" || u.Host == "" || u.User != nil || u.RawQuery != "" || u.ForceQuery || u.Fragment != "" {
		return "", serviceError(400, "invalid Goauthy resource")
	}
	p, err := whatwg.Parse(value)
	if err != nil {
		return "", serviceError(400, "Goauthy resource must be canonical")
	}
	return p.Href(false), nil
}

func goauthyBearer(value string) (string, error) {
	parts := strings.Fields(value)
	if len(parts) != 2 || !strings.EqualFold(parts[0], "Bearer") || !goauthyToken68(parts[1]) {
		return "", serviceError(401, "unauthorized: missing bearer token")
	}
	return parts[1], nil
}

func goauthyToken68(value string) bool {
	padded := false
	for i, r := range value {
		if r == '=' && i > 0 {
			padded = true
			continue
		}
		if !padded && (r >= 'a' && r <= 'z' || r >= 'A' && r <= 'Z' || r >= '0' && r <= '9' || strings.ContainsRune("-._~+/", r)) {
			continue
		}
		return false
	}
	return value != ""
}

func goauthySubject(issuer, subject string) string { return "goauthy:" + issuer + "/" + subject }

func (s *Service) goauthyJSON(ctx context.Context, request *http.Request, provider string) (map[string]json.RawMessage, error) {
	response, err := s.client.Do(request.WithContext(ctx))
	if err != nil {
		return nil, serviceError(503, "unavailable: "+provider+" authentication is unavailable")
	}
	defer response.Body.Close()
	if response.StatusCode == 401 || response.StatusCode == 403 {
		return nil, serviceError(401, "unauthorized: "+provider+" token rejected")
	}
	if response.StatusCode < 200 || response.StatusCode >= 300 {
		return nil, serviceError(503, "unavailable: "+provider+" authentication is unavailable")
	}
	body, err := io.ReadAll(io.LimitReader(response.Body, 65537))
	if err != nil || len(body) > 65536 {
		return nil, serviceError(503, "unavailable: "+provider+" auth response is too large")
	}
	return goauthyJSONObject(body, provider)
}

func goauthyJSONObject(body []byte, provider string) (map[string]json.RawMessage, error) {
	d := json.NewDecoder(bytes.NewReader(body))
	token, err := d.Token()
	if err != nil || token != json.Delim('{') {
		return nil, serviceError(503, "unavailable: "+provider+" auth response was invalid")
	}
	fields := map[string]json.RawMessage{}
	for d.More() {
		key, e := d.Token()
		if e != nil {
			return nil, serviceError(503, "unavailable: "+provider+" auth response was invalid")
		}
		name, ok := key.(string)
		if !ok || fields[name] != nil {
			return nil, serviceError(503, "unavailable: "+provider+" auth response was invalid")
		}
		var raw json.RawMessage
		if e = d.Decode(&raw); e != nil {
			return nil, serviceError(503, "unavailable: "+provider+" auth response was invalid")
		}
		fields[name] = raw
	}
	if _, err = d.Token(); err != nil || d.More() {
		return nil, serviceError(503, "unavailable: "+provider+" auth response was invalid")
	}
	if token, err = d.Token(); err != io.EOF || token != nil {
		return nil, serviceError(503, "unavailable: "+provider+" auth response was invalid")
	}
	return fields, nil
}

func goauthyField(fields map[string]json.RawMessage, name string, out any) bool {
	raw, ok := fields[name]
	return ok && json.Unmarshal(raw, out) == nil
}
func (s *Service) goauthyIdentity(ctx context.Context, authorization string) (ExternalIdentity, error) {
	token, err := goauthyBearer(authorization)
	if err != nil {
		return ExternalIdentity{}, err
	}
	cfg := s.cfg.Auth.Goauthy
	if cfg == nil {
		return ExternalIdentity{}, serviceError(503, "Goauthy is not configured")
	}
	issuer, err := canonicalGoauthyOrigin(cfg.Issuer)
	if err != nil || issuer != cfg.Issuer {
		return ExternalIdentity{}, serviceError(503, "invalid Goauthy configuration")
	}
	resource, err := canonicalGoauthyResource(cfg.Resource)
	if err != nil || resource != cfg.Resource {
		return ExternalIdentity{}, serviceError(503, "invalid Goauthy configuration")
	}
	req, err := http.NewRequest(http.MethodPost, issuer+"/oidc/introspect", strings.NewReader(url.Values{"token": {token}}.Encode()))
	if err != nil {
		return ExternalIdentity{}, err
	}
	req.SetBasicAuth(cfg.IntrospectionClientID, cfg.IntrospectionClientSecret)
	req.Header.Set("Content-Type", "application/x-www-form-urlencoded")
	fields, err := s.goauthyJSON(ctx, req, "Goauthy")
	if err != nil {
		return ExternalIdentity{}, err
	}
	var active bool
	if !goauthyField(fields, "active", &active) {
		return ExternalIdentity{}, serviceError(503, "unavailable: Goauthy introspection response was invalid")
	}
	if !active {
		return ExternalIdentity{}, serviceError(401, "unauthorized: Goauthy token is inactive")
	}
	var sub, clientID, scope string
	var exp int64
	var audiences []string
	if !goauthyField(fields, "sub", &sub) || !goauthyField(fields, "client_id", &clientID) || !goauthyField(fields, "scope", &scope) || !goauthyField(fields, "exp", &exp) || !goauthyField(fields, "aud", &audiences) || strings.TrimSpace(sub) == "" || strings.TrimSpace(clientID) != cfg.ExpectedClientID || exp <= time.Now().Unix() || !goauthyAudiences(audiences, resource) || !goauthyScopes(scope, cfg.RequiredScopes) {
		return ExternalIdentity{}, serviceError(401, "unauthorized: Goauthy token claims rejected")
	}
	if _, ok := fields["iss"]; ok {
		var responseIssuer string
		if !goauthyField(fields, "iss", &responseIssuer) || responseIssuer != issuer {
			return ExternalIdentity{}, serviceError(401, "unauthorized: Goauthy issuer mismatch")
		}
	}
	if _, ok := fields["nbf"]; ok {
		var nbf int64
		if !goauthyField(fields, "nbf", &nbf) || nbf > time.Now().Unix() {
			return ExternalIdentity{}, serviceError(401, "unauthorized: Goauthy token is not active")
		}
	}
	if fields["act"] != nil || fields["cnf"] != nil {
		return ExternalIdentity{}, serviceError(401, "unauthorized: Goauthy delegated or sender-constrained token rejected")
	}
	if _, ok := fields["token_type"]; ok {
		var tokenType string
		if !goauthyField(fields, "token_type", &tokenType) || !strings.EqualFold(tokenType, "Bearer") {
			return ExternalIdentity{}, serviceError(401, "unauthorized: Goauthy token type rejected")
		}
	}
	req, err = http.NewRequest(http.MethodGet, issuer+"/oidc/userinfo", nil)
	if err != nil {
		return ExternalIdentity{}, err
	}
	req.Header.Set("Authorization", "Bearer "+token)
	profile, err := s.goauthyJSON(ctx, req, "Goauthy")
	if err != nil {
		return ExternalIdentity{}, err
	}
	var profileSub, email, name, username, given, family string
	var verified bool
	if !goauthyField(profile, "sub", &profileSub) || profileSub != strings.TrimSpace(sub) {
		return ExternalIdentity{}, serviceError(401, "unauthorized: Goauthy userinfo subject mismatch")
	}
	if _, ok := profile["email"]; ok && (!goauthyField(profile, "email", &email) || !goauthyField(profile, "email_verified", &verified)) {
		return ExternalIdentity{}, serviceError(503, "unavailable: Goauthy userinfo response was invalid")
	}
	_ = goauthyField(profile, "name", &name)
	_ = goauthyField(profile, "preferred_username", &username)
	_ = goauthyField(profile, "given_name", &given)
	_ = goauthyField(profile, "family_name", &family)
	var verifiedEmail *string
	if verified && strings.Contains(email, "@") {
		value := strings.ToLower(strings.TrimSpace(email))
		verifiedEmail = &value
	}
	display := strings.TrimSpace(name)
	if display == "" {
		display = strings.TrimSpace(username)
	}
	if display == "" {
		display = strings.TrimSpace(given + " " + family)
	}
	if display == "" {
		display = strings.TrimSpace(sub)
	}
	return ExternalIdentity{Provider: "goauthy", Issuer: issuer, ExternalUserID: strings.TrimSpace(sub), ClientID: cfg.ApplicationID, Email: verifiedEmail, DisplayName: display, Active: true, TokenClientID: clientID, TokenAudience: audiences[0], UserInfoVerified: true}, nil
}

func goauthyAudiences(values []string, resource string) bool {
	return len(values) == 1 && values[0] == resource
}
func goauthyScopes(value string, required []string) bool {
	got := map[string]bool{}
	for _, scope := range strings.Fields(value) {
		got[scope] = true
	}
	for _, scope := range required {
		if !got[scope] {
			return false
		}
	}
	return true
}
