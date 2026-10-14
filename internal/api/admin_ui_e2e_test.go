package api

import (
	"context"
	"crypto/sha256"
	"crypto/tls"
	"crypto/x509"
	"encoding/base64"
	"encoding/json"
	"fmt"
	"html"
	"io"
	"net"
	"net/http"
	"net/http/cookiejar"
	"net/http/httptest"
	"net/url"
	"os"
	"regexp"
	"strings"
	"sync"
	"testing"
	"time"

	"github.com/Conalog/grant/internal/adminauth"
	"github.com/Conalog/grant/internal/adminui"
	"github.com/Conalog/grant/internal/grant"
)

const (
	adminE2EClientID     = "grant-admin-e2e"
	adminE2EClientSecret = "synthetic-admin-e2e-secret"
	adminE2EIssuerUser   = "owner"
	adminE2EAPIClient    = "grant-api-e2e-introspector"
	adminE2EAPISecret    = "synthetic-api-e2e-secret"
	adminE2EResource     = "https://grant.example.test/admin"
	adminE2EOrg          = "mgmt-org"
	adminE2ETenantOrg    = "owner"
)

type adminE2ECode struct {
	challenge, redirectURI, subject string
}

type adminE2EIssuer struct {
	t        *testing.T
	server   *httptest.Server
	mu       sync.Mutex
	codes    map[string]adminE2ECode
	subjects map[string]string
	nextSub  string
	nextTTL  int
}

func newAdminE2EIssuer(t *testing.T) *adminE2EIssuer {
	t.Helper()
	f := &adminE2EIssuer{t: t, codes: map[string]adminE2ECode{}, subjects: map[string]string{}, nextSub: adminE2EIssuerUser, nextTTL: 300}
	f.server = httptest.NewTLSServer(http.HandlerFunc(f.serveHTTP))
	t.Cleanup(f.server.Close)
	return f
}

func (f *adminE2EIssuer) serveHTTP(w http.ResponseWriter, r *http.Request) {
	switch r.URL.Path {
	case "/.well-known/openid-configuration":
		_ = json.NewEncoder(w).Encode(map[string]string{
			"issuer": f.server.URL, "authorization_endpoint": f.server.URL + "/authorize", "token_endpoint": f.server.URL + "/token",
		})
	case "/authorize":
		q := r.URL.Query()
		if q.Get("response_type") != "code" || q.Get("client_id") != adminE2EClientID || q.Get("resource") != adminE2EResource || q.Get("code_challenge_method") != "S256" || q.Get("code_challenge") == "" || q.Get("redirect_uri") == "" || q.Get("state") == "" {
			http.Error(w, "invalid authorization request", http.StatusBadRequest)
			return
		}
		f.mu.Lock()
		code := fmt.Sprintf("synthetic-code-%d", len(f.codes)+1)
		f.codes[code] = adminE2ECode{challenge: q.Get("code_challenge"), redirectURI: q.Get("redirect_uri"), subject: f.nextSub}
		f.nextSub = adminE2EIssuerUser
		f.mu.Unlock()
		callback, err := url.Parse(q.Get("redirect_uri"))
		if err != nil {
			http.Error(w, "invalid callback", http.StatusBadRequest)
			return
		}
		callbackQuery := callback.Query()
		callbackQuery.Set("code", code)
		callbackQuery.Set("state", q.Get("state"))
		callbackQuery.Set("iss", f.server.URL)
		callback.RawQuery = callbackQuery.Encode()
		http.Redirect(w, r, callback.String(), http.StatusFound)
	case "/token":
		if r.Method != http.MethodPost || !adminE2EClientAuth(r) || r.ParseForm() != nil || r.PostForm.Get("grant_type") != "authorization_code" {
			http.Error(w, "invalid token request", http.StatusBadRequest)
			return
		}
		f.mu.Lock()
		code, ok := f.codes[r.PostForm.Get("code")]
		delete(f.codes, r.PostForm.Get("code"))
		ttl := f.nextTTL
		f.mu.Unlock()
		verifierHash := sha256.Sum256([]byte(r.PostForm.Get("code_verifier")))
		if !ok || code.redirectURI != r.PostForm.Get("redirect_uri") || code.challenge != base64.RawURLEncoding.EncodeToString(verifierHash[:]) {
			http.Error(w, "invalid code or PKCE verifier", http.StatusBadRequest)
			return
		}
		token := "synthetic-access-" + code.subject
		f.mu.Lock()
		f.subjects[token] = code.subject
		f.nextTTL = 300
		f.mu.Unlock()
		f.t.Logf("synthetic issuer issued token for subject=%q ttl=%d", code.subject, ttl)
		w.Header().Set("Content-Type", "application/json")
		_ = json.NewEncoder(w).Encode(map[string]any{"access_token": token, "token_type": "Bearer", "expires_in": ttl})
	case "/oidc/introspect":
		user, secret, ok := r.BasicAuth()
		if r.Method != http.MethodPost || !ok || user != adminE2EAPIClient || secret != adminE2EAPISecret || r.ParseForm() != nil {
			http.Error(w, "invalid introspection request", http.StatusUnauthorized)
			return
		}
		f.mu.Lock()
		subject := f.subjects[r.PostForm.Get("token")]
		f.mu.Unlock()
		f.t.Logf("synthetic introspection active=%t subject=%q", subject != "", subject)
		if subject == "" {
			_ = json.NewEncoder(w).Encode(map[string]any{"active": false})
			return
		}
		_ = json.NewEncoder(w).Encode(map[string]any{
			"active": true, "sub": subject, "client_id": adminE2EClientID,
			"exp": time.Now().Add(5 * time.Minute).Unix(), "aud": []string{adminE2EResource},
			"scope": "openid profile email goauthy.read", "iss": f.server.URL, "token_type": "Bearer",
		})
	case "/oidc/userinfo":
		token := strings.TrimPrefix(r.Header.Get("Authorization"), "Bearer ")
		f.mu.Lock()
		subject := f.subjects[token]
		f.mu.Unlock()
		if subject == "" {
			http.Error(w, "inactive token", http.StatusUnauthorized)
			return
		}
		_ = json.NewEncoder(w).Encode(map[string]any{"sub": subject, "email": subject + "@example.test", "email_verified": true, "name": "Synthetic " + subject})
	default:
		http.NotFound(w, r)
	}
}

func adminE2EClientAuth(r *http.Request) bool {
	user, secret, ok := r.BasicAuth()
	return ok && user == adminE2EClientID && secret == adminE2EClientSecret
}

type adminE2ESystem struct {
	issuer *adminE2EIssuer
	api    *httptest.Server
	ui     *httptest.Server
	url    string
}

func newAdminE2ESystem(t *testing.T, bindAddr string) *adminE2ESystem {
	return newAdminE2ESystemWithPreview(t, bindAddr, false)
}

func newAdminE2ESystemWithPreview(t *testing.T, bindAddr string, browserHTTP bool) *adminE2ESystem {
	t.Helper()
	ctx := context.Background()
	issuer := newAdminE2EIssuer(t)
	trustLocalIssuerForGrantAPI(t, issuer.server.Certificate())
	cfg := grant.Config{
		Security: grant.SecurityConfig{BootstrapUserID: adminE2EIssuerUser, BootstrapUserName: "Synthetic Owner", ManagementOrganizationID: adminE2EOrg, MasterKey: "synthetic-master-key", AppAuthKey: "synthetic-app-key"},
		Auth: grant.AuthConfig{ProviderMode: "goauthy", Goauthy: &grant.GoauthyConfig{
			Issuer: issuer.server.URL, IntrospectionClientID: adminE2EAPIClient, IntrospectionClientSecret: adminE2EAPISecret,
			ExpectedClientID: adminE2EClientID, Resource: adminE2EResource, ApplicationID: "grant-admin-e2e", RequiredScopes: []string{"openid", "profile", "email", "goauthy.read"},
		}},
		Database: grant.DatabaseConfig{DataDir: t.TempDir(), DefaultReadConsistency: grant.ReadStrong},
	}
	store, err := grant.OpenStore(ctx, cfg)
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() { _ = store.Close() })
	if err = store.EnsureBootstrapData(ctx); err != nil {
		t.Fatal(err)
	}
	seedAdminE2E(t, ctx, store, issuer.server.URL)
	api := New(grant.NewService(cfg, store), false)
	apiHandler := http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		t.Logf("synthetic Grant API request: %s %s organization=%q", r.Method, r.URL.Path, r.Header.Get("X-Organization-ID"))
		api.ServeHTTP(w, r)
	})
	apiServer := httptest.NewServer(apiHandler)
	t.Cleanup(apiServer.Close)
	uiHandler, err := adminui.New(adminui.Config{APIURL: apiServer.URL, ManagementOrganizationID: adminE2EOrg, HTTPClient: &http.Client{Timeout: 10 * time.Second}})
	if err != nil {
		t.Fatal(err)
	}
	ui := uiHandler
	uiHandler = http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		if r.URL.Path == "/operations" && adminauth.AccessToken(r.Context()) == "" {
			t.Error("admin auth middleware did not pass the browser access token to the UI")
		}
		ui.ServeHTTP(w, r)
	})
	uiServer := httptest.NewUnstartedServer(http.NotFoundHandler())
	if bindAddr != "" {
		if err := uiServer.Listener.Close(); err != nil {
			t.Fatal(err)
		}
		listener, listenErr := net.Listen("tcp", bindAddr)
		if listenErr != nil {
			t.Fatal(listenErr)
		}
		uiServer.Listener = listener
	}
	publicScheme := "https"
	if browserHTTP {
		if bindAddr == "" {
			t.Fatal("HTTP browser preview requires an explicit loopback address")
		}
		host, _, err := net.SplitHostPort(bindAddr)
		if err != nil || net.ParseIP(host) == nil || !net.ParseIP(host).IsLoopback() {
			t.Fatalf("HTTP browser preview must bind to a loopback IP address, got %q", bindAddr)
		}
		publicScheme = "http"
	}
	publicURL := publicScheme + "://" + uiServer.Listener.Addr().String()
	authHandler, err := adminauth.New(adminauth.Config{
		Issuer: issuer.server.URL, ClientID: adminE2EClientID, ClientSecret: adminE2EClientSecret,
		Resource: adminE2EResource, PublicURL: publicURL,
		Scopes: []string{"openid", "profile", "email", "goauthy.read"}, HTTPClient: issuer.server.Client(),
	}, uiHandler)
	if err != nil {
		t.Fatal(err)
	}
	if browserHTTP {
		uiServer.Config.Handler = http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
			if r.URL.Path == "/__preview/authorize" {
				previewRequest := r.Clone(r.Context())
				previewURL := *r.URL
				previewURL.Path = "/authorize"
				previewRequest.URL = &previewURL
				issuer.serveHTTP(w, previewRequest)
				return
			}
			authHandler.ServeHTTP(&adminE2ERedirectWriter{
				ResponseWriter: w,
				from:           issuer.server.URL + "/authorize",
				to:             publicURL + "/__preview/authorize",
			}, r)
		})
		uiServer.Start()
	} else {
		uiServer.Config.Handler = authHandler
		uiServer.StartTLS()
	}
	t.Cleanup(uiServer.Close)
	return &adminE2ESystem{issuer: issuer, api: apiServer, ui: uiServer, url: publicURL}
}

type adminE2ERedirectWriter struct {
	http.ResponseWriter
	from string
	to   string
	code bool
}

func (w *adminE2ERedirectWriter) WriteHeader(status int) {
	if !w.code {
		if status >= http.StatusMultipleChoices && status < http.StatusBadRequest {
			location := w.Header().Get("Location")
			if strings.HasPrefix(location, w.from+"?") {
				w.Header().Set("Location", w.to+strings.TrimPrefix(location, w.from))
			}
		}
		w.code = true
	}
	w.ResponseWriter.WriteHeader(status)
}

func (w *adminE2ERedirectWriter) Write(body []byte) (int, error) {
	if !w.code {
		w.WriteHeader(http.StatusOK)
	}
	return w.ResponseWriter.Write(body)
}

func trustLocalIssuerForGrantAPI(t *testing.T, cert *x509.Certificate) {
	t.Helper()
	original, ok := http.DefaultTransport.(*http.Transport)
	if !ok {
		t.Fatal("default HTTP transport has unexpected type")
	}
	transport := original.Clone()
	roots := x509.NewCertPool()
	roots.AddCert(cert)
	transport.TLSClientConfig = &tls.Config{RootCAs: roots, MinVersion: tls.VersionTLS12}
	http.DefaultTransport = transport
	t.Cleanup(func() {
		http.DefaultTransport = original
		transport.CloseIdleConnections()
	})
}

func seedAdminE2E(t *testing.T, ctx context.Context, store *grant.Store, issuer string) {
	t.Helper()
	now := "2026-01-01T00:00:00Z"
	exec := func(query string, args ...any) {
		t.Helper()
		if _, err := store.Exec(ctx, query, args...); err != nil {
			t.Fatalf("seed Grant admin UI fixture: %v", err)
		}
	}
	exec("INSERT INTO user_identities(id,user_id,provider,issuer,collection_id,external_user_id,email_snapshot,display_name_snapshot,last_seen_at,created_at,updated_at) VALUES('e2e-owner-identity','owner','goauthy',?,'','owner','owner@example.test','Synthetic owner',?,?,?)", issuer, now, now, now)
	exec("INSERT INTO users(id,email,display_name,active,created_at) VALUES('target-user','target@example.test','Target user',1,?)", now)
	exec("INSERT INTO user_identities(id,user_id,provider,issuer,collection_id,external_user_id,email_snapshot,display_name_snapshot,last_seen_at,created_at,updated_at) VALUES('e2e-target-identity','target-user','goauthy',?,'','target-user','target@example.test','Target user',?,?,?)", issuer, now, now, now)
	exec("INSERT INTO organizations(id,name,created_at,updated_at) VALUES('mgmt-org','Management',?,?)", now, now)
	exec("INSERT INTO organization_memberships(id,organization_id,user_id,membership_role,status,created_at,updated_at) VALUES('e2e-owner-membership','mgmt-org','owner','owner','active',?,?)", now, now)
	exec("INSERT INTO organizations(id,name,created_at,updated_at) VALUES('foreign-org','Foreign organization',?,?)", now, now)
	exec("INSERT INTO service_providers(id,name,active,created_at,updated_at) VALUES('grant-management','Grant Management',1,?,?)", now, now)
	for _, action := range []struct{ code, name string }{
		{"admin.provider.create", "Create provider"},
		{"admin.provider.actions.read", "Read provider actions"},
		{"admin.provider.actions.create", "Create provider action"},
		{"admin.users.read", "Read users"},
		{"admin.user.active", "Change user access"},
	} {
		id := "grant-management." + action.code
		exec("INSERT INTO actions(id,provider_id,code,name,risk_level,visibility_granting,grantable_to_applications,created_at,updated_at) VALUES(?,'grant-management',? ,?,'medium',0,1,?,?)", id, action.code, action.name, now, now)
	}
	exec("INSERT INTO roles(id,organization_id,code,name,created_at,updated_at) VALUES('e2e-management-role',NULL,'e2e-management','E2E management',?,?)", now, now)
	for _, code := range []string{"admin.provider.create", "admin.provider.actions.read", "admin.provider.actions.create", "admin.users.read", "admin.user.active"} {
		exec("INSERT INTO role_actions(role_id,action_id) VALUES('e2e-management-role',?)", "grant-management."+code)
	}
	exec("INSERT INTO role_bindings(id,subject_type,subject_id,subject_organization_id,provider_id,resource_type,resource_id,role_id,state,created_by_user_id,created_at,updated_at) VALUES('e2e-management-binding','user','owner','mgmt-org','grant-management','platform','grant','e2e-management-role','active','owner',?,?)", now, now)
	exec("INSERT INTO applications(id,organization_id,slug,name,application_type,app_type,first_party,consent_policy,active,provisioning_status,created_by_user_id,created_at,updated_at) VALUES('grant-admin-e2e','mgmt-org','admin-e2e','Admin E2E','confidential','server',0,'explicit',1,'ready','owner',?,?)", now, now)
	for _, code := range []string{"admin.provider.create", "admin.provider.actions.read", "admin.provider.actions.create", "admin.users.read", "admin.user.active"} {
		exec("INSERT INTO application_actions(application_id,action_id) VALUES('grant-admin-e2e',?)", "grant-management."+code)
	}
	exec("INSERT INTO application_grants(id,user_id,application_id,organization_id,status,consented_at,created_at,updated_at) VALUES('e2e-management-grant','owner','grant-admin-e2e','mgmt-org','active',?,?,?)", now, now, now)
	for _, code := range []string{"admin.provider.create", "admin.provider.actions.read", "admin.provider.actions.create", "admin.users.read", "admin.user.active"} {
		exec("INSERT INTO application_grant_actions(application_grant_id,action_id) VALUES('e2e-management-grant',?)", "grant-management."+code)
	}
}

func TestAdminUIBrowserE2EAgainstGrantAPI(t *testing.T) {
	system := newAdminE2ESystem(t, "")
	health := adminE2ERequest(t, http.DefaultClient, http.MethodGet, system.api.URL+"/healthz", nil, "")
	if health.StatusCode != http.StatusOK {
		adminE2EFail(t, health)
	}
	_ = adminE2EBody(t, health)
	client := adminE2EBrowser(t, system.issuer.server.Certificate(), system.ui.Certificate())
	response := adminE2ERequest(t, client, http.MethodGet, system.url+"/login", nil, "")
	if response.Request.URL.Path != "/" || response.StatusCode != http.StatusOK {
		adminE2EFail(t, response)
	}
	client.CheckRedirect = func(*http.Request, []*http.Request) error { return http.ErrUseLastResponse }
	root := adminE2EBody(t, response)
	for _, want := range []string{`href="/?`, `href="/providers?`, `href="/organizations?`, `href="/groups?`, `href="/roles?`, `href="/users?`} {
		if !strings.Contains(root, want) {
			t.Fatalf("authenticated navigation omitted %q", want)
		}
	}
	providers := adminE2EGet(t, client, system.url+"/providers")
	csrf := adminE2ECSRF(t, providers)
	for _, want := range []string{`name="operation" value="provider.create"`, `name="id"`, `id="provider-create"`} {
		if !strings.Contains(providers, want) {
			t.Fatalf("provider form omitted stable field/action %q", want)
		}
	}

	adminE2EPost(t, client, system.url, "/operations", csrf, "providers", "", "grant-management", url.Values{
		"operation": {"provider.create"}, "id": {"e2e-provider"}, "name": {"E2E Provider"}, "active": {"true"},
	})
	providers = adminE2EGet(t, client, system.url+"/providers")
	if !strings.Contains(providers, "e2e-provider") {
		t.Fatal("created provider missing from Grant API readback")
	}

	actionsURL := system.url + "/actions?provider=e2e-provider"
	actions := adminE2EGet(t, client, actionsURL)
	csrf = adminE2ECSRF(t, actions)
	adminE2EPost(t, client, system.url, "/operations", csrf, "actions", "", "e2e-provider", url.Values{
		"operation": {"action.create"}, "id": {"e2e-provider.read.widget"}, "code": {"read.widget"}, "name": {"Read widget"},
		"riskLevel": {"low"}, "visibilityGranting": {"false"}, "grantableToApplications": {"true"},
	})
	actions = adminE2EGet(t, client, actionsURL)
	if !strings.Contains(actions, "e2e-provider.read.widget") {
		t.Fatal("created action missing from Grant API readback")
	}

	orgQuery := "?org=" + adminE2ETenantOrg
	members := adminE2EGet(t, client, system.url+"/members"+orgQuery)
	csrf = adminE2ECSRF(t, members)
	adminE2EPost(t, client, system.url, "/operations", csrf, "members", adminE2ETenantOrg, "grant-management", url.Values{
		"operation": {"member.enroll"}, "userId": {"target-user"},
	})
	members = adminE2EGet(t, client, system.url+"/members"+orgQuery)
	if !strings.Contains(members, "target-user") {
		t.Fatal("enrolled member missing from Grant API readback")
	}

	groups := adminE2EGet(t, client, system.url+"/groups"+orgQuery)
	csrf = adminE2ECSRF(t, groups)
	adminE2EPost(t, client, system.url, "/operations", csrf, "groups", adminE2ETenantOrg, "grant-management", url.Values{
		"operation": {"group.create"}, "id": {"e2e-group"}, "name": {"E2E Group"}, "description": {"created through admin UI"},
	})
	groups = adminE2EGet(t, client, system.url+"/groups"+orgQuery)
	if !strings.Contains(groups, "e2e-group") {
		t.Fatal("created group missing from Grant API readback")
	}
	csrf = adminE2ECSRF(t, groups)
	adminE2EPost(t, client, system.url, "/operations", csrf, "groups", adminE2ETenantOrg, "grant-management", url.Values{
		"operation": {"group.member.add"}, "id": {"e2e-group"}, "userId": {"target-user"},
	})
	groupMembers := adminE2EAPIGet(t, system.api.URL+"/api/v1/orgs/owner/groups/e2e-group/members", "synthetic-access-owner", adminE2ETenantOrg)
	if !strings.Contains(groupMembers, "target-user") {
		t.Fatal("created group membership missing from Grant API readback")
	}

	roles := adminE2EGet(t, client, system.url+"/roles"+orgQuery)
	csrf = adminE2ECSRF(t, roles)
	adminE2EPost(t, client, system.url, "/operations", csrf, "roles", adminE2ETenantOrg, "grant-management", url.Values{
		"operation": {"role.create"}, "id": {"e2e-role"}, "code": {"e2e"}, "name": {"E2E Role"}, "description": {"created through admin UI"},
	})
	roles = adminE2EGet(t, client, system.url+"/roles"+orgQuery)
	if !strings.Contains(roles, "e2e-role") {
		t.Fatal("created role missing from Grant API readback")
	}
	denied := adminE2EPostResponse(t, client, system.url, "/operations", adminE2ECSRF(t, providers), "providers", "", "e2e-provider", url.Values{
		"operation": {"provider.update"}, "id": {"e2e-provider"}, "name": {"Must remain unchanged"},
	})
	if denied.StatusCode != http.StatusForbidden {
		adminE2EFail(t, denied)
	}
	_ = denied.Body.Close()
	foreign := adminE2ERequest(t, client, http.MethodGet, system.url+"/groups?org=foreign-org", nil, "")
	if foreign.StatusCode != http.StatusForbidden {
		adminE2EFail(t, foreign)
	}
	_ = foreign.Body.Close()

	system.issuer.mu.Lock()
	system.issuer.nextSub = "target-user"
	system.issuer.nextTTL = 1
	system.issuer.mu.Unlock()
	targetBrowser := adminE2EBrowser(t, system.issuer.server.Certificate(), system.ui.Certificate())
	targetHome := adminE2ERequest(t, targetBrowser, http.MethodGet, system.url+"/login", nil, "")
	if targetHome.StatusCode != http.StatusOK || targetHome.Request.URL.Path != "/" {
		adminE2EFail(t, targetHome)
	}
	_ = adminE2EBody(t, targetHome)
	time.Sleep(1100 * time.Millisecond)
	expiredRequest, err := http.NewRequest(http.MethodGet, system.url+"/groups?org="+adminE2ETenantOrg, nil)
	if err != nil {
		t.Fatal(err)
	}
	expiredRequest.Header.Set("HX-Request", "true")
	expired, err := targetBrowser.Do(expiredRequest)
	if err != nil {
		t.Fatal(err)
	}
	if expired.StatusCode != http.StatusOK || expired.Header.Get("HX-Redirect") != "/login" || expired.Header.Get("Location") != "" {
		adminE2EFail(t, expired)
	}
	_ = expired.Body.Close()
	system.issuer.mu.Lock()
	system.issuer.nextSub = "target-user"
	system.issuer.nextTTL = 300
	system.issuer.mu.Unlock()
	targetHome = adminE2ERequest(t, targetBrowser, http.MethodGet, system.url+"/login", nil, "")
	if targetHome.StatusCode != http.StatusOK || targetHome.Request.URL.Path != "/" {
		adminE2EFail(t, targetHome)
	}
	_ = adminE2EBody(t, targetHome)
	if got := adminE2EAPIStatus(t, system.api.URL+"/api/v1/me", "synthetic-access-target-user"); got != http.StatusOK {
		t.Fatalf("active target user API status=%d before disable", got)
	}
	users := adminE2EGet(t, client, system.url+"/users")
	csrf = adminE2ECSRF(t, users)
	adminE2EPost(t, client, system.url, "/operations", csrf, "users", "", "grant-management", url.Values{
		"operation": {"user.active"}, "id": {"target-user"}, "active": {"false"},
	})
	users = adminE2EGet(t, client, system.url+"/users")
	if !strings.Contains(users, "target-user") || !strings.Contains(users, "false") {
		t.Fatal("disabled target user missing from admin UI readback")
	}
	if got := adminE2EAPIStatus(t, system.api.URL+"/api/v1/me", "synthetic-access-target-user"); got != http.StatusForbidden {
		t.Fatalf("disabled target user's active GoAuthy token still authenticated: status=%d", got)
	}
	selectedRoleURL := system.url + "/roles?org=" + adminE2ETenantOrg + "&id=e2e-role"
	selectedRoleResponse := adminE2ERequest(t, client, http.MethodGet, selectedRoleURL, nil, "")
	selectedRole := adminE2EBody(t, selectedRoleResponse)
	if selectedRoleResponse.StatusCode != http.StatusOK {
		t.Errorf("selected role detail GET %s returned HTTP %d: %s", selectedRoleURL, selectedRoleResponse.StatusCode, selectedRole)
	} else {
		csrf = adminE2ECSRF(t, selectedRole)
		oldETag := adminE2EEtag(t, selectedRole)
		if !strings.Contains(selectedRole, `name="operation" value="role.actions"`) {
			t.Error("selected role detail omitted the role actions form")
		} else {
			adminE2EPost(t, client, system.url, "/operations", csrf, "roles", adminE2ETenantOrg, "grant-management", url.Values{
				"operation": {"role.actions"}, "id": {"e2e-role"}, "actionIds": {"organization.group.manage"}, "etag": {oldETag},
			})
			selectedRole = adminE2EGet(t, client, selectedRoleURL)
			if !strings.Contains(html.UnescapeString(selectedRole), "organization.group.manage") {
				t.Error("role action replacement missing from selected-role readback")
			}
			stale := adminE2EPostResponse(t, client, system.url, "/operations", adminE2ECSRF(t, selectedRole), "roles", adminE2ETenantOrg, "grant-management", url.Values{
				"operation": {"role.actions"}, "id": {"e2e-role"}, "actionIds": {"organization.permission.manage"}, "etag": {oldETag},
			})
			if stale.StatusCode != http.StatusPreconditionFailed {
				t.Errorf("stale role ETag returned HTTP %d, want %d", stale.StatusCode, http.StatusPreconditionFailed)
			}
			_ = stale.Body.Close()
		}
	}
	ownerDetailResponse := adminE2ERequest(t, client, http.MethodGet, system.url+"/organizations?id="+adminE2ETenantOrg, nil, "")
	ownerDetail := adminE2EBody(t, ownerDetailResponse)
	if ownerDetailResponse.StatusCode != http.StatusOK {
		t.Errorf("selected organization detail returned HTTP %d: %s", ownerDetailResponse.StatusCode, ownerDetail)
	} else if !strings.Contains(html.UnescapeString(ownerDetail), `"ownerUserId": "owner"`) {
		t.Errorf("selected organization detail omitted its owner: %s", ownerDetail)
	}
}

func TestGrantAdminBrowserPreview(t *testing.T) {
	addr := strings.TrimSpace(os.Getenv("GRANT_ADMIN_PREVIEW_ADDR"))
	if addr == "" {
		t.Skip("set GRANT_ADMIN_PREVIEW_ADDR=127.0.0.1:port to hold the synthetic admin UI preview for up to 10 minutes")
	}
	system := newAdminE2ESystemWithPreview(t, addr, true)
	t.Logf("Grant admin UI preview: %s", system.url)
	timer := time.NewTimer(10 * time.Minute)
	defer timer.Stop()
	<-timer.C
}

func adminE2EBrowser(t *testing.T, certs ...*x509.Certificate) *http.Client {
	t.Helper()
	roots := x509.NewCertPool()
	for _, cert := range certs {
		roots.AddCert(cert)
	}
	jar, err := cookiejar.New(nil)
	if err != nil {
		t.Fatal(err)
	}
	return &http.Client{Jar: jar, Transport: &http.Transport{TLSClientConfig: &tls.Config{RootCAs: roots, MinVersion: tls.VersionTLS12}, Proxy: nil}, Timeout: 15 * time.Second}
}

var adminE2ECSRFPattern = regexp.MustCompile(`name="csrf_token" value="([^"]+)"`)

func adminE2ECSRF(t *testing.T, page string) string {
	t.Helper()
	match := adminE2ECSRFPattern.FindStringSubmatch(page)
	if len(match) != 2 || match[1] == "" {
		t.Fatal("page has no CSRF token")
	}
	return match[1]
}

func adminE2EGet(t *testing.T, client *http.Client, target string) string {
	t.Helper()
	response := adminE2ERequest(t, client, http.MethodGet, target, nil, "")
	if response.StatusCode != http.StatusOK {
		adminE2EFail(t, response)
	}
	return adminE2EBody(t, response)
}

func adminE2EPost(t *testing.T, client *http.Client, baseURL, path, csrf, section, org, provider string, values url.Values) {
	t.Helper()
	response := adminE2EPostResponse(t, client, baseURL, path, csrf, section, org, provider, values)
	defer response.Body.Close()
	if response.StatusCode != http.StatusSeeOther {
		adminE2EFail(t, response)
	}
}

func adminE2EPostResponse(t *testing.T, client *http.Client, baseURL, path, csrf, section, org, provider string, values url.Values) *http.Response {
	t.Helper()
	values.Set("csrf_token", csrf)
	values.Set("section", section)
	values.Set("org", org)
	values.Set("provider", provider)
	req, err := http.NewRequest(http.MethodPost, baseURL+path, strings.NewReader(values.Encode()))
	if err != nil {
		t.Fatal(err)
	}
	req.Header.Set("Content-Type", "application/x-www-form-urlencoded")
	req.Header.Set("Origin", baseURL)
	response, err := client.Do(req)
	if err != nil {
		t.Fatal(err)
	}
	return response
}

func adminE2EAPIGet(t *testing.T, target, token, org string) string {
	t.Helper()
	response := adminE2EAPIRequest(t, target, token, org)
	if response.StatusCode != http.StatusOK {
		adminE2EFail(t, response)
	}
	return adminE2EBody(t, response)
}

func adminE2EAPIStatus(t *testing.T, target, token string) int {
	t.Helper()
	response := adminE2EAPIRequest(t, target, token, adminE2ETenantOrg)
	if response.StatusCode >= 400 {
		body, _ := io.ReadAll(response.Body)
		t.Logf("synthetic Grant API returned HTTP %d: %s", response.StatusCode, body)
	} else {
		_ = response.Body.Close()
	}
	return response.StatusCode
}

func adminE2EAPIRequest(t *testing.T, target, token, org string) *http.Response {
	t.Helper()
	req, err := http.NewRequest(http.MethodGet, target, nil)
	if err != nil {
		t.Fatal(err)
	}
	req.Header.Set("Authorization", "Bearer "+token)
	req.Header.Set("X-Organization-ID", org)
	response, err := http.DefaultClient.Do(req)
	if err != nil {
		t.Fatal(err)
	}
	return response
}

var adminE2EEtagPattern = regexp.MustCompile(`name="etag" value="([^"]*)"`)

func adminE2EEtag(t *testing.T, page string) string {
	t.Helper()
	match := adminE2EEtagPattern.FindStringSubmatch(page)
	if len(match) != 2 || match[1] == "" {
		t.Fatal("selected detail has no ETag field")
	}
	return html.UnescapeString(match[1])
}

func adminE2ERequest(t *testing.T, client *http.Client, method, target string, body io.Reader, contentType string) *http.Response {
	t.Helper()
	req, err := http.NewRequest(method, target, body)
	if err != nil {
		t.Fatal(err)
	}
	if contentType != "" {
		req.Header.Set("Content-Type", contentType)
	}
	response, err := client.Do(req)
	if err != nil {
		t.Fatal(err)
	}
	return response
}

func adminE2EBody(t *testing.T, response *http.Response) string {
	t.Helper()
	defer response.Body.Close()
	body, err := io.ReadAll(response.Body)
	if err != nil {
		t.Fatal(err)
	}
	return string(body)
}

func adminE2EFail(t *testing.T, response *http.Response) {
	t.Helper()
	body, _ := io.ReadAll(response.Body)
	_ = response.Body.Close()
	t.Fatalf("HTTP %d %s: %s", response.StatusCode, response.Request.URL, body)
}
