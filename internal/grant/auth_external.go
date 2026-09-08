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
	"unicode"
)

func (s *Service) externalIdentity(ctx context.Context, authorization, bootstrapOrganization string) (ExternalIdentity, error) {
	if strings.TrimSpace(authorization) == "" {
		return ExternalIdentity{}, serviceError(401, "unauthorized: missing bearer token")
	}
	if s.cfg.Auth.ProviderMode == "patch-data" {
		return s.patchDataIdentity(ctx, authorization, bootstrapOrganization)
	}
	return s.rauthyIdentity(ctx, authorization)
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
	token, err := rauthyBearer(authorization)
	if err != nil {
		return ExternalIdentity{}, err
	}
	if token == "" {
		return ExternalIdentity{}, serviceError(401, "unauthorized: missing bearer token")
	}
	key := tokenCacheKey(token)
	if cached, ok := s.cachedIdentity(key); ok {
		return cached.Identity, nil
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

type patchRecord struct {
	ID, Email, Name string
	Verified        bool
	Organizations   []json.RawMessage
}

func (p *patchRecord) UnmarshalJSON(data []byte) error {
	d := json.NewDecoder(bytes.NewReader(data))
	token, err := d.Token()
	if err != nil || token != json.Delim('{') {
		return serviceError(503, "invalid record")
	}
	seen := map[string]bool{}
	for d.More() {
		keyToken, err := d.Token()
		if err != nil {
			return err
		}
		key := keyToken.(string)
		if key == "name" {
			key = "displayName"
		}
		var raw json.RawMessage
		if err = d.Decode(&raw); err != nil {
			return err
		}
		if key == "organizations" {
			p.Organizations = append(p.Organizations, raw)
			continue
		}
		switch key {
		case "id", "email", "verified", "displayName":
			if key != "displayName" && bytes.Equal(bytes.TrimSpace(raw), []byte("null")) {
				return serviceError(503, "null required identity field")
			}
			if seen[key] {
				return serviceError(503, "duplicate identity field")
			}
			seen[key] = true
			var target any
			switch key {
			case "id":
				target = &p.ID
			case "email":
				target = &p.Email
			case "verified":
				target = &p.Verified
			case "displayName":
				target = &p.Name
			}
			if err = json.Unmarshal(raw, target); err != nil {
				return err
			}
		}
	}
	if _, err = d.Token(); err != nil {
		return err
	}
	for _, key := range []string{"id", "email", "verified"} {
		if !seen[key] {
			return serviceError(503, "missing identity field")
		}
	}
	return nil
}
func (s *Service) patchDataIdentity(ctx context.Context, authorization, bootstrapOrg string) (ExternalIdentity, error) {
	cfg := s.cfg.Auth.PatchData
	if cfg == nil {
		return ExternalIdentity{}, serviceError(503, "unavailable: Patch Data is not configured")
	}
	req, err := http.NewRequest("POST", strings.TrimRight(cfg.BaseURL, "/")+"/api/collections/users/auth-refresh", nil)
	if err != nil {
		return ExternalIdentity{}, err
	}
	req.Header.Set("Authorization", authorization)
	var payload struct {
		Token  string       `json:"token"`
		Record *patchRecord `json:"record"`
	}
	if err = s.authJSON(ctx, req, &payload, "Patch Data"); err != nil {
		return ExternalIdentity{}, err
	}
	if strings.TrimSpace(payload.Token) == "" || payload.Record == nil {
		return ExternalIdentity{}, serviceError(503, "unavailable: Patch Data auth refresh response was invalid")
	}
	r := payload.Record
	if !r.Verified {
		return ExternalIdentity{}, serviceError(403, "forbidden: Patch Data email is not verified")
	}
	id, err := required("Patch Data user id", r.ID)
	if err != nil {
		return ExternalIdentity{}, err
	}
	email := strings.ToLower(strings.TrimSpace(r.Email))
	if email == "" || !strings.Contains(email, "@") {
		return ExternalIdentity{}, serviceError(401, "unauthorized: invalid email")
	}
	name := strings.TrimSpace(r.Name)
	if name == "" {
		name = "Patch Data User"
	}
	identity := ExternalIdentity{Provider: "patch-data", ExternalUserID: id, ClientID: cfg.ExposureApplicationID, Email: &email, DisplayName: name, Active: true}
	if bootstrapOrg != "" {
		if len(r.Organizations) != 1 {
			return ExternalIdentity{}, serviceError(503, "unavailable: Patch Data auth refresh response was invalid")
		}
		var orgs []string
		if err = json.Unmarshal(r.Organizations[0], &orgs); err != nil || len(orgs) > 100 {
			return ExternalIdentity{}, serviceError(503, "unavailable: Patch Data auth refresh response was invalid")
		}
		found := false
		for _, org := range orgs {
			if org == "" || len(org) > 255 || org != strings.TrimSpace(org) || strings.IndexFunc(org, unicode.IsControl) >= 0 {
				return ExternalIdentity{}, serviceError(503, "unavailable: Patch Data auth refresh response was invalid")
			}
			if org == bootstrapOrg {
				found = true
			}
		}
		if !found {
			return ExternalIdentity{}, serviceError(403, "forbidden: Patch Data user is not a member of the requested organization")
		}
		identity.OrganizationID = &bootstrapOrg
	}
	return identity, nil
}
