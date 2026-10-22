package adminui

import (
	"encoding/json"
	"net/http"
	"net/http/httptest"
	"net/url"
	"strings"
	"testing"

	"github.com/mrchypark/grant/internal/adminauth"
)

func TestAdminPagesSendOrganizationThroughExposureBoundary(t *testing.T) {
	for _, tc := range []struct{ name, page, target, organization string }{
		{"overview", "/", "/api/v1/providers", "management-org"},
		{"providers", "/providers", "/api/v1/providers", "management-org"},
		{"selected providers", "/providers?org=selected-org", "/api/v1/providers", "selected-org"},
		{"actions", "/actions?org=selected-org", "/api/v1/admin/providers/grant-management/actions", "management-org"},
		{"HTTP catalog", "/http?org=selected-org", "/api/v1/admin/providers/grant-management/http-capability-active", "management-org"},
		{"selected members", "/members?org=selected-org", "/api/v1/orgs/selected-org/members", "selected-org"},
	} {
		t.Run(tc.name, func(t *testing.T) {
			matched := false
			proxy := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
				if r.Header.Get("Authorization") != "Bearer header-test-token" {
					http.Error(w, "missing credential", http.StatusUnauthorized)
					return
				}
				if r.URL.Path != "/api/v1/orgs" && r.Header.Get("X-Organization-ID") == "" {
					http.Error(w, "missing organization", http.StatusUnauthorized)
					return
				}
				if r.URL.Path == tc.target && !matched {
					matched = true
					if got := r.Header.Get("X-Organization-ID"); got != tc.organization {
						t.Errorf("organization=%q want %q", got, tc.organization)
					}
				}
				w.Header().Set("Content-Type", "application/json")
				_, _ = w.Write([]byte(`[]`))
			}))
			t.Cleanup(proxy.Close)
			ui, err := New(Config{APIURL: proxy.URL, ManagementOrganizationID: "management-org"})
			if err != nil {
				t.Fatal(err)
			}
			response := requestWithHeaderTestSession(t, ui, tc.page)
			if response.Code != http.StatusOK || strings.Contains(response.Body.String(), "HTTP 401") || !matched {
				t.Fatalf("page status=%d target reached=%v body=%s", response.Code, matched, response.Body.String())
			}
		})
	}
}

func TestAdminClientKeepsUnscopedReadsUnscoped(t *testing.T) {
	for _, route := range []string{"/api/v1/me", "/api/v1/orgs"} {
		t.Run(route, func(t *testing.T) {
			proxy := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
				if got := r.Header.Get("X-Organization-ID"); got != "" {
					t.Errorf("unscoped read acquired organization %q", got)
				}
				_, _ = w.Write([]byte(`[]`))
			}))
			t.Cleanup(proxy.Close)
			client, err := newAPIClient(proxy.URL, nil)
			if err != nil {
				t.Fatal(err)
			}
			requestWithHeaderTestSession(t, http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
				if _, _, err := client.call(r.Context(), http.MethodGet, route, nil, nil, nil, "", "management-org"); err != nil {
					t.Error(err)
				}
			}), "/test")
		})
	}
}

func requestWithHeaderTestSession(t *testing.T, next http.Handler, path string) *httptest.ResponseRecorder {
	t.Helper()
	issuer := "https://issuer.example.test"
	transport := relatedRoundTripper(func(r *http.Request) (*http.Response, error) {
		if r.URL.Path == "/.well-known/openid-configuration" {
			body, _ := json.Marshal(map[string]string{"issuer": issuer, "authorization_endpoint": issuer + "/authorize", "token_endpoint": issuer + "/token"})
			return relatedResponse(r, http.StatusOK, string(body)), nil
		}
		return relatedResponse(r, http.StatusOK, `{"access_token":"header-test-token","token_type":"Bearer","expires_in":300}`), nil
	})
	auth, err := adminauth.New(adminauth.Config{Issuer: issuer, ClientID: "admin-ui", ClientSecret: "test-secret", PublicURL: "https://admin.example.test", HTTPClient: &http.Client{Transport: transport}}, next)
	if err != nil {
		t.Fatal(err)
	}
	login := httptest.NewRecorder()
	auth.ServeHTTP(login, httptest.NewRequest(http.MethodGet, "/login", nil))
	location, err := url.Parse(login.Header().Get("Location"))
	if err != nil || login.Code != http.StatusFound {
		t.Fatalf("login status=%d error=%v", login.Code, err)
	}
	callback := httptest.NewRequest(http.MethodGet, "/oauth/callback?"+url.Values{"code": {"test-code"}, "state": {location.Query().Get("state")}}.Encode(), nil)
	for _, cookie := range login.Result().Cookies() {
		callback.AddCookie(cookie)
	}
	result := httptest.NewRecorder()
	auth.ServeHTTP(result, callback)
	if result.Code != http.StatusSeeOther {
		t.Fatalf("callback status=%d", result.Code)
	}
	request := httptest.NewRequest(http.MethodGet, path, nil)
	for _, cookie := range result.Result().Cookies() {
		request.AddCookie(cookie)
	}
	response := httptest.NewRecorder()
	auth.ServeHTTP(response, request)
	return response
}
