package grant

import (
	"bytes"
	"context"
	"encoding/json"
	whatwg "github.com/nlnwa/whatwg-url/url"
	"io"
	"net/http"
	"net/url"
	"strings"
	"time"
)

func (s *Service) externalIdentity(ctx context.Context, authorization string) (ExternalIdentity, error) {
	if strings.TrimSpace(authorization) == "" {
		return ExternalIdentity{}, serviceError(401, "unauthorized: missing bearer token")
	}
	switch s.cfg.Auth.ProviderMode {
	case "pocketbase":
		return s.pocketBaseIdentity(ctx, authorization)
	case "external-check":
		return s.externalCheckIdentity(ctx, authorization)
	case "goauthy":
		return s.goauthyIdentity(ctx, authorization)
	case "jwks":
		return s.jwksIdentity(ctx, authorization)
	case "rauthy", "oidc":
		if s.cfg.Auth.Rauthy.JWKSURL != "" {
			return s.jwksIdentity(ctx, authorization)
		}
		return s.rauthyIdentity(ctx, authorization)
	default:
		return ExternalIdentity{}, serviceError(503, "authentication provider is not configured")
	}
}
func (s *Service) authJSON(ctx context.Context, request *http.Request, out any, provider string) error {
	response, err := s.client.Do(request.WithContext(ctx))
	if err != nil {
		return serviceError(503, "unavailable: "+provider+" authentication is unavailable")
	}
	defer response.Body.Close()
	if response.StatusCode == 401 || response.StatusCode == 403 {
		return serviceError(401, "unauthorized: "+provider+" token rejected")
	}
	if response.StatusCode < 200 || response.StatusCode >= 300 {
		return serviceError(503, "unavailable: "+provider+" authentication is unavailable")
	}
	body, err := io.ReadAll(io.LimitReader(response.Body, 65537))
	if err != nil || len(body) > 65536 {
		return serviceError(503, "unavailable: "+provider+" auth refresh response is too large")
	}
	if err = json.Unmarshal(body, out); err != nil {
		return serviceError(503, "unavailable: "+provider+" auth refresh response was invalid")
	}
	return nil
}
func (s *Service) rauthyIdentity(ctx context.Context, authorization string) (ExternalIdentity, error) {
	return s.rauthyIdentityWithCache(ctx, authorization, true)
}

func (s *Service) rauthyIdentityWithCache(ctx context.Context, authorization string, cache bool) (ExternalIdentity, error) {
	token, err := rauthyBearer(authorization)
	if err != nil {
		return ExternalIdentity{}, err
	}
	if token == "" {
		return ExternalIdentity{}, serviceError(401, "unauthorized: missing bearer token")
	}
	key := tokenCacheKey(token)
	if cache {
		if cached, ok := s.cachedIdentity(key); ok {
			return cached.Identity, nil
		}
	}
	cfg := s.cfg.Auth.Rauthy
	req, err := http.NewRequest("POST", strings.TrimRight(cfg.BaseURL, "/")+cfg.IntrospectPath, strings.NewReader(url.Values{"token": {token}}.Encode()))
	if err != nil {
		return ExternalIdentity{}, err
	}
	req.SetBasicAuth(cfg.ClientID, cfg.ClientSecret)
	req.Header.Set("Content-Type", "application/x-www-form-urlencoded")
	var info struct {
		Active   *bool  `json:"active"`
		Sub      string `json:"sub"`
		ClientID string `json:"client_id"`
		Exp      *int64 `json:"exp"`
	}
	if err = s.authJSON(ctx, req, &info, "Rauthy"); err != nil {
		return ExternalIdentity{}, err
	}
	if info.Active == nil {
		return ExternalIdentity{}, serviceError(503, "unavailable: Rauthy introspection response was invalid")
	}
	if !*info.Active {
		return ExternalIdentity{}, serviceError(401, "unauthorized: rauthy token is inactive")
	}
	if strings.TrimSpace(info.Sub) == "" || strings.TrimSpace(info.ClientID) == "" {
		return ExternalIdentity{}, serviceError(401, "unauthorized: rauthy introspection is missing subject or client id")
	}
	info.Sub = strings.TrimSpace(info.Sub)
	info.ClientID = strings.TrimSpace(info.ClientID)
	if info.Exp != nil && *info.Exp <= time.Now().Unix() {
		return ExternalIdentity{}, serviceError(401, "unauthorized: rauthy token is expired")
	}
	req, err = http.NewRequest("GET", strings.TrimRight(cfg.BaseURL, "/")+cfg.UserinfoPath, nil)
	if err != nil {
		return ExternalIdentity{}, err
	}
	req.Header.Set("Authorization", "Bearer "+token)
	var profile struct {
		Sub           string  `json:"sub"`
		Email         *string `json:"email"`
		EmailVerified bool    `json:"email_verified"`
		Name          string  `json:"name"`
		Username      string  `json:"preferred_username"`
		GivenName     string  `json:"given_name"`
		FamilyName    string  `json:"family_name"`
	}
	if err = s.authJSON(ctx, req, &profile, "Rauthy"); err != nil {
		return ExternalIdentity{}, err
	}
	if profile.Sub != info.Sub {
		return ExternalIdentity{}, serviceError(401, "unauthorized: rauthy userinfo subject mismatch")
	}
	email := profile.Email
	if !profile.EmailVerified {
		email = nil
	}
	name := ""
	for _, v := range []string{profile.Name, profile.Username, strings.TrimSpace(profile.GivenName + " " + profile.FamilyName), ptrValue(email), info.Sub} {
		if strings.TrimSpace(v) != "" {
			name = strings.TrimSpace(v)
			break
		}
	}
	identity := ExternalIdentity{Provider: "rauthy", ExternalUserID: strings.TrimSpace(info.Sub), ClientID: strings.TrimSpace(info.ClientID), Email: email, DisplayName: name, Active: true}
	if !cache {
		return identity, nil
	}
	if s.cfg.Auth.Rauthy.CacheTTLSeconds > uint64((1<<63-1)/int64(time.Second)) {
		return ExternalIdentity{}, serviceError(401, "unauthorized: invalid auth cache duration")
	}
	expires := time.Now().Add(time.Duration(s.cfg.Auth.Rauthy.CacheTTLSeconds) * time.Second)
	if info.Exp != nil && time.Unix(*info.Exp, 0).Before(expires) {
		expires = time.Unix(*info.Exp, 0)
	}
	s.cacheIdentity(key, cachedIdentity{Identity: identity, ExpiresAt: expires})
	return identity, nil
}
func ptrValue(value *string) string {
	if value == nil {
		return ""
	}
	return *value
}

type pocketBaseRecord struct {
	ID             string `json:"id"`
	CollectionID   string `json:"collectionId"`
	CollectionName string `json:"collectionName"`
	Email          string `json:"email"`
	Verified       bool   `json:"verified"`
	Name           string `json:"name"`
	Username       string `json:"username"`
}

func (p *pocketBaseRecord) UnmarshalJSON(data []byte) error {
	type record pocketBaseRecord
	d := json.NewDecoder(bytes.NewReader(data))
	t, err := d.Token()
	if err != nil || t != json.Delim('{') {
		return serviceError(503, "invalid PocketBase record")
	}
	seen := map[string]bool{}
	fields := map[string]json.RawMessage{}
	for d.More() {
		k, err := d.Token()
		if err != nil {
			return err
		}
		key := k.(string)
		var raw json.RawMessage
		if err = d.Decode(&raw); err != nil {
			return err
		}
		if seen[key] {
			return serviceError(503, "duplicate PocketBase record field")
		}
		seen[key] = true
		fields[key] = raw
	}
	if _, err = d.Token(); err != nil {
		return err
	}
	for _, k := range []string{"id", "collectionId", "collectionName"} {
		if !seen[k] || bytes.Equal(bytes.TrimSpace(fields[k]), []byte("null")) {
			return serviceError(503, "missing PocketBase identity field")
		}
	}
	var value record
	if err = json.Unmarshal(data, &value); err != nil {
		return err
	}
	*p = pocketBaseRecord(value)
	return nil
}

func pocketBaseID(value string) bool {
	if value == "" || len(value) > 255 {
		return false
	}
	for _, r := range value {
		if !(r >= 'a' && r <= 'z' || r >= 'A' && r <= 'Z' || r >= '0' && r <= '9' || r == '_') {
			return false
		}
	}
	return true
}
func canonicalPocketBaseOrigin(value string) (string, error) {
	u, err := url.Parse(value)
	if err != nil || u.Host == "" || u.User != nil || u.RawQuery != "" || u.ForceQuery || u.Fragment != "" || (u.Path != "" && u.Path != "/") || (u.Scheme != "http" && u.Scheme != "https") {
		return "", serviceError(400, "invalid PocketBase issuer origin")
	}
	parsed, err := whatwg.Parse(value)
	if err != nil {
		return "", err
	}
	return strings.TrimSuffix(parsed.Href(false), "/"), nil
}
func pocketBaseSubject(issuer, collection, id string) string {
	return "pocketbase:" + issuer + "/" + collection + "/" + id
}
func (s *Service) pocketBaseIdentity(ctx context.Context, authorization string) (ExternalIdentity, error) {
	cfg := s.cfg.Auth.PocketBase
	if cfg == nil {
		return ExternalIdentity{}, serviceError(503, "PocketBase is not configured")
	}
	issuer, err := canonicalPocketBaseOrigin(cfg.BaseURL)
	collection := cfg.Collection
	if collection == "" {
		collection = "users"
	}
	if err != nil || !pocketBaseID(collection) {
		return ExternalIdentity{}, serviceError(503, "invalid PocketBase configuration")
	}
	req, err := http.NewRequest("POST", issuer+"/api/collections/"+collection+"/auth-refresh", nil)
	if err != nil {
		return ExternalIdentity{}, err
	}
	req.Header.Set("Authorization", authorization)
	var payload struct {
		Token  string            `json:"token"`
		Record *pocketBaseRecord `json:"record"`
	}
	if err = s.authJSON(ctx, req, &payload, "PocketBase"); err != nil {
		return ExternalIdentity{}, err
	}
	r := payload.Record
	if strings.TrimSpace(payload.Token) == "" || r == nil || !pocketBaseID(r.ID) || !pocketBaseID(r.CollectionID) || r.CollectionName != collection {
		return ExternalIdentity{}, serviceError(503, "invalid PocketBase verified identity")
	}
	var email *string
	if r.Verified && strings.Contains(r.Email, "@") {
		v := strings.ToLower(strings.TrimSpace(r.Email))
		email = &v
	}
	name := strings.TrimSpace(r.Name)
	if name == "" {
		name = strings.TrimSpace(r.Username)
	}
	if name == "" {
		name = r.ID
	}
	return ExternalIdentity{Provider: "pocketbase", Issuer: issuer, CollectionID: r.CollectionID, ExternalUserID: r.ID, ClientID: cfg.ApplicationID, Email: email, DisplayName: name, Active: true}, nil
}

func (s *Service) externalCheckIdentity(ctx context.Context, authorization string) (ExternalIdentity, error) {
	cfg := s.cfg.Auth.ExternalCheck
	if cfg == nil || strings.TrimSpace(cfg.URL) == "" {
		return ExternalIdentity{}, serviceError(503, "external-check is not configured")
	}
	method := strings.ToUpper(strings.TrimSpace(cfg.Method))
	if method == "" {
		method = "POST"
	}
	req, err := http.NewRequest(method, cfg.URL, nil)
	if err != nil {
		return ExternalIdentity{}, err
	}
	req.Header.Set("Authorization", authorization)
	var payload map[string]any
	if err = s.authJSON(ctx, req, &payload, "external-check"); err != nil {
		return ExternalIdentity{}, err
	}
	if cfg.TokenPath != "" {
		if tok, ok := jsonPathString(payload, cfg.TokenPath); !ok || strings.TrimSpace(tok) == "" {
			return ExternalIdentity{}, serviceError(503, "invalid external-check verified token")
		}
	}
	if cfg.ExpectedField != "" {
		if got, ok := jsonPathString(payload, cfg.ExpectedField); !ok || got != cfg.ExpectedValue {
			return ExternalIdentity{}, serviceError(503, "invalid external-check assertion")
		}
	}
	subjectPath := cfg.SubjectPath
	if subjectPath == "" {
		subjectPath = "sub"
	}
	sub, ok := jsonPathString(payload, subjectPath)
	sub = strings.TrimSpace(sub)
	if !ok || sub == "" {
		return ExternalIdentity{}, serviceError(503, "invalid external-check verified subject")
	}
	var email *string
	emailPath := cfg.EmailPath
	if emailPath == "" {
		emailPath = "email"
	}
	verifiedPath := cfg.EmailVerifiedPath
	if verifiedPath == "" {
		verifiedPath = "email_verified"
	}
	if rawEmail, ok := jsonPathString(payload, emailPath); ok && strings.Contains(rawEmail, "@") {
		if verified, hasVerified := jsonPathBool(payload, verifiedPath); hasVerified && verified {
			v := strings.ToLower(strings.TrimSpace(rawEmail))
			email = &v
		}
	}
	namePath := cfg.NamePath
	if namePath == "" {
		namePath = "name"
	}
	name, _ := jsonPathString(payload, namePath)
	name = strings.TrimSpace(name)
	if name == "" && cfg.FallbackNamePath != "" {
		name, _ = jsonPathString(payload, cfg.FallbackNamePath)
		name = strings.TrimSpace(name)
	}
	if name == "" && email != nil {
		name = *email
	}
	if name == "" {
		name = sub
	}
	clientID := strings.TrimSpace(cfg.ApplicationID)
	if cfg.ClientIDPath != "" {
		if v, ok := jsonPathString(payload, cfg.ClientIDPath); ok && strings.TrimSpace(v) != "" {
			clientID = strings.TrimSpace(v)
		}
	}
	return ExternalIdentity{
		Provider:       "external-check",
		ExternalUserID: sub,
		ClientID:       clientID,
		Email:          email,
		DisplayName:    name,
		Active:         true,
	}, nil
}

func jsonPathValue(root map[string]any, path string) (any, bool) {
	if root == nil || path == "" {
		return nil, false
	}
	var cur any = root
	for _, seg := range strings.Split(path, ".") {
		m, ok := cur.(map[string]any)
		if !ok {
			return nil, false
		}
		v, exists := m[seg]
		if !exists || v == nil {
			return nil, false
		}
		cur = v
	}
	return cur, true
}

func jsonPathString(root map[string]any, path string) (string, bool) {
	v, ok := jsonPathValue(root, path)
	if !ok {
		return "", false
	}
	s, ok := v.(string)
	return s, ok
}

func jsonPathBool(root map[string]any, path string) (bool, bool) {
	v, ok := jsonPathValue(root, path)
	if !ok {
		return false, false
	}
	b, ok := v.(bool)
	return b, ok
}
