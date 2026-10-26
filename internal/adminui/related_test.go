package adminui

import (
	"encoding/json"
	"io"
	"net/http"
	"net/http/httptest"
	"net/url"
	"strings"
	"testing"

	"github.com/mrchypark/grant/internal/adminauth"
)

type relatedRoundTripper func(*http.Request) (*http.Response, error)

func (f relatedRoundTripper) RoundTrip(r *http.Request) (*http.Response, error) { return f(r) }

func TestLoadRelatedGroupMembers(t *testing.T) {
	var apiRequest *http.Request
	tables, err := loadRelatedWithSession(t, func(r *http.Request) (*http.Response, error) {
		apiRequest = r
		return relatedResponse(r, http.StatusOK, `[{"groupId":"group-1","organizationMembershipId":"membership-1","userId":"user-1"}]`), nil
	}, "groups", "org-1", "provider-1", "group-1")
	if err != nil {
		t.Fatal(err)
	}
	if apiRequest.Method != http.MethodGet || apiRequest.URL.Path != "/api/v1/orgs/org-1/groups/group-1/members" {
		t.Fatalf("unexpected management request: %s %s", apiRequest.Method, apiRequest.URL.Path)
	}
	if apiRequest.Header.Get("Authorization") != "Bearer related-access-token" || apiRequest.Header.Get("X-Organization-ID") != "org-1" {
		t.Fatalf("management auth/scope headers: %v", apiRequest.Header)
	}
	if len(tables) != 1 || tables[0].Title != "그룹 구성원" || len(tables[0].Rows) != 1 {
		t.Fatalf("unexpected tables: %+v", tables)
	}
	row := tables[0].Rows[0]
	if row.Cells[0] != "user-1" || row.Cells[1] != "membership-1" {
		t.Fatalf("unexpected member row: %+v", row)
	}
	link, err := url.Parse(row.Link)
	if err != nil || link.Path != "/members" || link.Query().Get("org") != "org-1" || link.Query().Get("provider") != "provider-1" || link.Query().Get("id") != "user-1" {
		t.Fatalf("unexpected member link %q: %v", row.Link, err)
	}
}

func TestLoadRelatedApplicationGrantsFilterAndRevokeLink(t *testing.T) {
	var apiRequest *http.Request
	tables, err := loadRelatedWithSession(t, func(r *http.Request) (*http.Response, error) {
		apiRequest = r
		return relatedResponse(r, http.StatusOK, `[
			{"id":"grant-1","userId":"user-1","applicationId":"app-1","organizationId":"org-1","status":"active","consentedAt":"2026-10-01T00:00:00Z"},
			{"id":"grant-2","userId":"user-2","applicationId":"app-2","organizationId":"org-1","status":"revoked","consentedAt":"2026-10-02T00:00:00Z"}
		]`), nil
	}, "applications", "org-1", "provider-1", "app-2")
	if err != nil {
		t.Fatal(err)
	}
	if apiRequest.Method != http.MethodGet || apiRequest.URL.Path != "/api/v1/orgs/org-1/app-grants" {
		t.Fatalf("unexpected management request: %s %s", apiRequest.Method, apiRequest.URL.Path)
	}
	if len(tables) != 1 || tables[0].Title != "애플리케이션 권한 부여" || len(tables[0].Rows) != 1 {
		t.Fatalf("selected application grants were not filtered: %+v", tables)
	}
	row := tables[0].Rows[0]
	if row.Cells[0] != "grant-2" || row.Cells[1] != "user-2" || row.Cells[2] != "app-2" || row.Cells[3] != "revoked" {
		t.Fatalf("unexpected grant row: %+v", row)
	}
	link, err := url.Parse(row.Link)
	if err != nil || link.Path != "/applications" || link.Query().Get("org") != "org-1" || link.Query().Get("provider") != "provider-1" || link.Query().Get("id") != "app-2" || link.Query().Get("grant") != "grant-2" {
		t.Fatalf("grant ID not available in application link %q: %v", row.Link, err)
	}
}

func TestLoadRelatedReturnsManagementErrorsAndRejectsMalformedLists(t *testing.T) {
	for _, tc := range []struct {
		name   string
		status int
		body   string
		key    string
	}{
		{"forbidden", http.StatusForbidden, `{"error":"forbidden"}`, "groups"},
		{"invalid-json", http.StatusOK, `{`, "applications"},
		{"wrong-shape", http.StatusOK, `{"items":[]}`, "groups"},
		{"null-list", http.StatusOK, `null`, "applications"},
	} {
		t.Run(tc.name, func(t *testing.T) {
			tables, err := loadRelatedWithSession(t, func(r *http.Request) (*http.Response, error) {
				return relatedResponse(r, tc.status, tc.body), nil
			}, tc.key, "org-1", "provider-1", "selected-1")
			if err == nil || tables != nil {
				t.Fatalf("expected accurate management/decoding error, got tables=%+v err=%v", tables, err)
			}
		})
	}
}

func TestLoadRelatedSkipsUnselectedGroupAndUnknownKeys(t *testing.T) {
	calls := 0
	transport := func(r *http.Request) (*http.Response, error) {
		calls++
		return relatedResponse(r, http.StatusOK, `[]`), nil
	}
	for _, tc := range []struct{ key, selectedID string }{{"groups", ""}, {"members", "user-1"}} {
		tables, err := loadRelatedWithSession(t, transport, tc.key, "org-1", "provider-1", tc.selectedID)
		if err != nil || len(tables) != 0 {
			t.Fatalf("key=%q returned tables=%+v err=%v", tc.key, tables, err)
		}
	}
	if calls != 0 {
		t.Fatalf("unexpected management API calls: %d", calls)
	}
}

func loadRelatedWithSession(t *testing.T, apiTransport relatedRoundTripper, key, org, provider, selectedID string) ([]Table, error) {
	t.Helper()
	api, err := newAPIClient("https://grant.example.test", &http.Client{Transport: apiTransport})
	if err != nil {
		t.Fatal(err)
	}
	s := &server{cfg: Config{ManagementOrganizationID: "management-org"}, api: api}
	var tables []Table
	var loadErr error
	issuer := "https://issuer.example.test"
	issuerTransport := relatedRoundTripper(func(r *http.Request) (*http.Response, error) {
		switch r.URL.Path {
		case "/.well-known/openid-configuration":
			body, _ := json.Marshal(map[string]string{
				"issuer": issuer, "authorization_endpoint": issuer + "/authorize", "token_endpoint": issuer + "/token", "userinfo_endpoint": issuer + "/userinfo",
			})
			return relatedResponse(r, http.StatusOK, string(body)), nil
		case "/token":
			return relatedResponse(r, http.StatusOK, `{"access_token":"related-access-token","token_type":"Bearer","expires_in":300}`), nil
		case "/userinfo":
			return relatedResponse(r, http.StatusOK, `{"sub":"user-1","email":"admin@example.test","name":"Admin User"}`), nil
		default:
			return relatedResponse(r, http.StatusNotFound, `{}`), nil
		}
	})
	auth, err := adminauth.New(adminauth.Config{
		Issuer: issuer, ClientID: "admin-ui", ClientSecret: "test-secret", PublicURL: "https://admin.example.test",
		HTTPClient: &http.Client{Transport: issuerTransport},
	}, http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		tables, loadErr = s.loadRelated(r, key, org, provider, selectedID)
	}))
	if err != nil {
		t.Fatal(err)
	}
	login := httptest.NewRecorder()
	auth.ServeHTTP(login, httptest.NewRequest(http.MethodGet, "/login", nil))
	if login.Code != http.StatusFound {
		t.Fatalf("login status=%d body=%s", login.Code, login.Body.String())
	}
	loginURL, err := url.Parse(login.Header().Get("Location"))
	if err != nil {
		t.Fatal(err)
	}
	var loginCookie *http.Cookie
	for _, cookie := range login.Result().Cookies() {
		if cookie.Name == "grant_admin_login" {
			loginCookie = cookie
		}
	}
	if loginCookie == nil {
		t.Fatal("login transaction cookie missing")
	}
	callbackValues := url.Values{"code": {"one-time-code"}, "state": {loginURL.Query().Get("state")}}
	callback := httptest.NewRequest(http.MethodGet, "/oauth/callback?"+callbackValues.Encode(), nil)
	callback.AddCookie(loginCookie)
	callbackResponse := httptest.NewRecorder()
	auth.ServeHTTP(callbackResponse, callback)
	if callbackResponse.Code != http.StatusSeeOther {
		t.Fatalf("OAuth callback status=%d body=%s", callbackResponse.Code, callbackResponse.Body.String())
	}
	var sessionCookie *http.Cookie
	for _, cookie := range callbackResponse.Result().Cookies() {
		if cookie.Name == "grant_admin_session" {
			sessionCookie = cookie
		}
	}
	if sessionCookie == nil {
		t.Fatal("session cookie missing")
	}
	request := httptest.NewRequest(http.MethodGet, "/test", nil)
	request.AddCookie(sessionCookie)
	response := httptest.NewRecorder()
	auth.ServeHTTP(response, request)
	if response.Code != http.StatusOK {
		t.Fatalf("related request status=%d body=%s", response.Code, response.Body.String())
	}
	return tables, loadErr
}

func relatedResponse(request *http.Request, status int, body string) *http.Response {
	header := make(http.Header)
	header.Set("Content-Type", "application/json")
	return &http.Response{StatusCode: status, Header: header, Body: io.NopCloser(strings.NewReader(body)), Request: request}
}
