package api

import (
	"bytes"
	"encoding/json"
	"net/http/httptest"
	"strings"
	"testing"

	"github.com/Conalog/grant/internal/grant"
)

func TestPrivateMigrationValidationStatus(t *testing.T) {
	s := managementTestServer(t)
	var manifest grant.PatchDataAuthorityImport
	if err := json.Unmarshal([]byte(`{"manifestVersion":"grant.patch-data-authority.v1","generation":"validation-test","users":[],"organizations":[],"memberships":[],"hierarchyEdges":[],"plantBindings":[],"applicationAssignments":[]}`), &manifest); err != nil {
		t.Fatal(err)
	}
	manifest.LedgerDigest = strings.Repeat("a", 64)
	manifest.ManifestDigest = grant.PatchDataManifestDigest(manifest)
	call := func(value grant.PatchDataAuthorityImport, want int) *httptest.ResponseRecorder {
		t.Helper()
		payload, err := json.Marshal(value)
		if err != nil {
			t.Fatal(err)
		}
		r := httptest.NewRequest("POST", "/ops/migrations/patch-data-authority", bytes.NewReader(payload))
		r.Header.Set("X-Grant-Master-Key", "master-secret")
		r.Header.Set("Content-Type", "application/json")
		w := httptest.NewRecorder()
		s.ServeHTTP(w, r)
		if w.Code != want {
			t.Fatalf("got %d want %d: %s", w.Code, want, w.Body.String())
		}
		return w
	}
	bad := manifest
	bad.ManifestVersion = "invalid"
	call(bad, 400)
	bad = manifest
	bad.ManifestDigest = strings.Repeat("b", 64)
	call(bad, 400)
	first := call(manifest, 200)
	if !strings.Contains(first.Body.String(), `"applied":true`) {
		t.Fatalf("valid import not applied after validation failures: %s", first.Body.String())
	}
	repeat := call(manifest, 200)
	if !strings.Contains(repeat.Body.String(), `"applied":false`) {
		t.Fatalf("valid import not idempotent: %s", repeat.Body.String())
	}
}

func TestProviderActionHTTPPreconditionsAndPatch(t *testing.T) {
	s := managementTestServer(t)
	request := func(method, path, body, match string, status int) *httptest.ResponseRecorder {
		t.Helper()
		r := httptest.NewRequest(method, path, strings.NewReader(body))
		r.Header.Set("X-Grant-Master-Key", "master-secret")
		r.Header.Set("Content-Type", "application/json")
		if match != "" {
			r.Header.Set("If-Match", match)
		}
		w := httptest.NewRecorder()
		s.ServeHTTP(w, r)
		if w.Code != status {
			t.Fatalf("%s %s: got %d want %d: %s", method, path, w.Code, status, w.Body.String())
		}
		return w
	}
	request("POST", "/api/v1/admin/providers", `{"id":"actions-test","name":"Actions","active":true}`, "", 201)
	path := "/api/v1/admin/providers/actions-test/actions"
	first := request("GET", path, "", "", 200)
	tag := first.Header().Get("ETag")
	if !strings.HasPrefix(tag, `"`) || strings.TrimSpace(first.Body.String()) != "[]" {
		t.Fatalf("invalid snapshot %q %s", tag, first.Body.String())
	}
	request("PUT", path, `{"actions":[]}`, "", 428)
	request("PUT", path, `{"actions":[]}`, `"stale"`, 412)
	request("PUT", path, `{"actions":[]}`, tag, 200)
	request("POST", path, `{"id":"read","code":"read","name":"Read","riskLevel":"low","visibilityGranting":false,"grantableToApplications":true}`, "", 201)
	request("PUT", path, `{"actions":[]}`, tag, 412)
	item := path + "/actions-test.read"
	response := request("PATCH", item, `{"name":"Read one","description":null}`, "", 200)
	var action map[string]any
	if err := json.Unmarshal(response.Body.Bytes(), &action); err != nil {
		t.Fatal(err)
	}
	if action["name"] != "Read one" || action["grantableToApplications"] != true {
		t.Fatalf("patch replaced omitted field: %v", action)
	}
	request("PATCH", item, `{"id":"different"}`, "", 422)
	request("PATCH", item, `{"name":"a","name":"b"}`, "", 422)
	request("PATCH", item, `{"grantableToApplications":null}`, "", 422)
	request("DELETE", item, "", "", 204)
	request("DELETE", item, "", "", 204)
}

func TestPrivateOpsCannotUseManagementBearer(t *testing.T) {
	s := managementTestServer(t)
	for _, endpoint := range []struct{ method, path, body string }{
		{"GET", "/ops/backups", ""},
		{"POST", "/ops/backups", ""},
		{"POST", "/ops/migrations/patch-data-authority", `{"manifestVersion":"grant.patch-data-authority.v1","generation":"1","ledgerDigest":"x","manifestDigest":"x","users":[],"organizations":[],"memberships":[],"hierarchyEdges":[],"plantBindings":[],"applicationAssignments":[]}`},
		{"PUT", "/ops/providers/grant-management/enforcers/pep", `{"applicationId":"app-1","lifecycle":"active"}`},
	} {
		r := httptest.NewRequest(endpoint.method, endpoint.path, strings.NewReader(endpoint.body))
		r.Header.Set("Authorization", "Bearer tok")
		r.Header.Set("X-Organization-ID", "mgmt-org")
		r.Header.Set("Content-Type", "application/json")
		w := httptest.NewRecorder()
		s.ServeHTTP(w, r)
		if w.Code != 401 {
			t.Errorf("%s %s: %d %s", endpoint.method, endpoint.path, w.Code, w.Body.String())
		}
	}
	for _, path := range []string{"/api/v1/admin/bootstrap/rauthy-owner", "/api/v1/admin/bootstrap/external-owner", "/api/v1/admin/backups", "/api/v1/admin/backups/s3", "/api/v1/admin/imports/patch-data-authority", "/api/v1/admin/providers/p/pep-clients/pep"} {
		r := httptest.NewRequest("POST", path, nil)
		r.Header.Set("X-Grant-Master-Key", "master-secret")
		w := httptest.NewRecorder()
		s.ServeHTTP(w, r)
		if w.Code != 404 {
			t.Errorf("retired route %s: %d", path, w.Code)
		}
	}
}

func TestOwnerBootstrapAndSeparateApplicationRegistration(t *testing.T) {
	s := managementTestServer(t)
	call := func(method, path, body string, status int) *httptest.ResponseRecorder {
		t.Helper()
		r := httptest.NewRequest(method, path, strings.NewReader(body))
		r.Header.Set("X-Grant-Master-Key", "master-secret")
		r.Header.Set("Authorization", "Bearer tok")
		r.Header.Set("X-Organization-ID", "fresh-org")
		r.Header.Set("Content-Type", "application/json")
		w := httptest.NewRecorder()
		s.ServeHTTP(w, r)
		if w.Code != status {
			t.Fatalf("%s %s: %d %s", method, path, w.Code, w.Body.String())
		}
		return w
	}
	call("POST", "/api/v1/admin/bootstrap/owner", "", 201)
	apps := call("GET", "/api/v1/orgs/fresh-org/apps?consistency=strong", "", 200)
	if strings.TrimSpace(apps.Body.String()) != "[]" {
		t.Fatalf("bootstrap created application: %s", apps.Body.String())
	}
	call("POST", "/api/v1/orgs/fresh-org/apps", `{"id":"explicit-app","slug":"explicit-app","name":"Explicit","applicationType":"confidential","appType":"internal","active":true}`, 201)
	call("POST", "/api/v1/admin/bootstrap/owner", "", 201)
	apps = call("GET", "/api/v1/orgs/fresh-org/apps?consistency=strong", "", 200)
	var values []json.RawMessage
	if err := json.Unmarshal(apps.Body.Bytes(), &values); err != nil || len(values) != 1 {
		t.Fatalf("explicit registration: %s %v", apps.Body.String(), err)
	}
}
