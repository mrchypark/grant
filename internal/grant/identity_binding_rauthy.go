package grant

import (
	"context"
	"encoding/json"
	"net/http"
	"net/url"
	"sort"
	"strings"
)

func (s *Service) verifyRauthyIdentityBinding(ctx context.Context, a Application, clientID string, uri *string) error {
	req, err := http.NewRequestWithContext(ctx, "GET", strings.TrimRight(s.RauthyBaseURL(), "/")+"/auth/v1/clients/"+url.PathEscape(clientID), nil)
	if err != nil {
		return err
	}
	req.Header.Set("Authorization", "API-Key "+s.cfg.Auth.Rauthy.ClientReadAPIKey)
	res, err := s.client.Do(req)
	if err != nil {
		return serviceError(503, "rauthy client fetch failed")
	}
	defer res.Body.Close()
	if res.StatusCode == 404 {
		return serviceError(409, "Rauthy client is not provisioned")
	}
	if res.StatusCode < 200 || res.StatusCode >= 300 {
		return serviceError(503, "rauthy client fetch failed")
	}
	var v struct {
		ID           string   `json:"id"`
		Name         *string  `json:"name"`
		Enabled      bool     `json:"enabled"`
		Confidential bool     `json:"confidential"`
		RedirectURIs []string `json:"redirect_uris"`
		FlowsEnabled []string `json:"flows_enabled"`
		Challenges   []string `json:"challenges"`
	}
	if err = json.NewDecoder(res.Body).Decode(&v); err != nil {
		return serviceError(503, "invalid Rauthy client response")
	}
	expected := []string{}
	if uri != nil {
		canonical, err := CanonicalRedirectURI(*uri)
		if err != nil || canonical != *uri {
			return serviceError(409, "local application redirect URI manifest is invalid")
		}
		expected = append(expected, *uri)
	}
	flows := []string{"authorization_code", "refresh_token"}
	if a.ApplicationType == "confidential" {
		flows = append(flows, "client_credentials")
	}
	same := func(a, b []string) bool {
		if len(a) != len(b) {
			return false
		}
		for i := range a {
			if a[i] != b[i] {
				return false
			}
		}
		return true
	}
	sort.Strings(flows)
	actual := map[string]bool{}
	for _, f := range v.FlowsEnabled {
		actual[f] = true
	}
	got := make([]string, 0, len(actual))
	for f := range actual {
		got = append(got, f)
	}
	sort.Strings(got)
	if v.ID != clientID || ptrValue(v.Name) != a.Name || v.Enabled != a.Active || v.Confidential != (a.ApplicationType == "confidential") || !same(v.RedirectURIs, expected) || !same(got, flows) || (a.ApplicationType == "public" && !same(v.Challenges, []string{"S256"})) {
		return serviceError(409, "Rauthy client configuration does not match the local application manifest")
	}
	return nil
}
