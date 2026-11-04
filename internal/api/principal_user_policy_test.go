package api

import (
	"crypto/sha256"
	"encoding/json"
	"fmt"
	"net/http/httptest"
	"strings"
	"testing"

	"github.com/mrchypark/grant/internal/grant"
)

func pepPolicyCall(t *testing.T, s *Server, method, path, payload, match, none string, want int) *httptest.ResponseRecorder {
	t.Helper()
	r := httptest.NewRequest(method, path, strings.NewReader(payload))
	r.Header.Set("X-Grant-Master-Key", "master-secret")
	r.Header.Set("Content-Type", "application/json")
	if match != "" {
		r.Header.Set("If-Match", match)
	}
	if none != "" {
		r.Header.Set("If-None-Match", none)
	}
	w := httptest.NewRecorder()
	s.ServeHTTP(w, r)
	if w.Code != want {
		t.Fatalf("%s %s: got %d want %d: %s", method, path, w.Code, want, w.Body.String())
	}
	return w
}

func TestPEPPolicyHTTPConditionalWrites(t *testing.T) {
	s := managementTestServer(t)
	path := "/ops/providers/grant-management/enforcers/policy-pep"
	call := func(method, body, match, none string, want int) *httptest.ResponseRecorder {
		return pepPolicyCall(t, s, method, path, body, match, none, want)
	}
	call("GET", "", "", "", 404)
	call("PUT", `{"lifecycle":"active"}`, "", "", 428)
	call("PUT", `{"lifecycle":"active"}`, `"missing"`, "", 412)
	created := call("PUT", `{"lifecycle":"active","authorizationMode":"user"}`, "", "*", 200)
	expected := fmt.Sprintf(`"pep-policy-sha256:%x"`, sha256.Sum256([]byte("grant-management\npolicy-pep\n1")))
	tag := created.Header().Get("ETag")
	if tag != expected {
		t.Fatalf("ETag %q want %q", tag, expected)
	}
	call("PUT", `{"lifecycle":"active"}`, "", "*", 412)
	call("PUT", `{"lifecycle":"active"}`, "W/"+tag, "", 412)
	call("PUT", `{"lifecycle":"active"}`, "*", "", 412)
	call("PUT", `{"lifecycle":"active"}`, tag, "*", 400)
	call("PUT", `{"lifecycle":"active"}`, "", tag, 400)
	for _, body := range []string{
		`{"lifecycle":"active","authorizationMode":null}`,
		`{"lifecycle":"active","authorizationMode":"unknown"}`,
		`{"lifecycle":"active","authorizationMode":true}`,
		`{"lifecycle":"active","policyRevision":50}`,
		`{"lifecycle":"active","authorizationMode":"user","authorizationMode":"application"}`,
	} {
		call("PUT", body, tag, "", 422)
	}
	read := call("GET", "", "", "", 200)
	if read.Header().Get("ETag") != tag {
		t.Fatal("failed writes changed policy")
	}
	updated := call("PUT", `{"lifecycle":"disabled","validUntil":"2000-01-01T00:00:00Z"}`, tag, "", 200)
	var policy grant.ProviderPepClient
	if err := json.Unmarshal(updated.Body.Bytes(), &policy); err != nil {
		t.Fatal(err)
	}
	if policy.AuthorizationMode != "user" || policy.PolicyRevision != 2 {
		t.Fatalf("omitted mode not preserved: %+v", policy)
	}
	newer := updated.Header().Get("ETag")
	if newer == tag || newer == "" {
		t.Fatal("policy update did not change ETag")
	}
	call("PUT", `{"lifecycle":"active"}`, tag, "", 412)
	// No catalog is registered; disabled and expired policies must still be readable.
	read = call("GET", "", "", "", 200)
	if read.Header().Get("ETag") != newer {
		t.Fatal("management read did not return stored policy ETag")
	}
	other := pepPolicyCall(t, s, "PUT", "/ops/providers/grant-management/enforcers/another-pep", `{"lifecycle":"active"}`, "", "*", 200)
	if other.Header().Get("ETag") == tag {
		t.Fatal("ETag is not bound to PEP identity")
	}
	var legacy grant.ProviderPepClient
	if err := json.Unmarshal(other.Body.Bytes(), &legacy); err != nil || legacy.AuthorizationMode != "application" {
		t.Fatalf("create default: %+v, %v", legacy, err)
	}
}

func TestPEPPolicyHTTPManagementBoundary(t *testing.T) {
	s := managementTestServer(t)
	path := "/ops/providers/grant-management/enforcers/policy-pep"
	for _, method := range []string{"GET", "PUT"} {
		r := httptest.NewRequest(method, path, strings.NewReader(`{"lifecycle":"active"}`))
		r.Header.Set("Content-Type", "application/json")
		r.Header.Set("X-Grant-App-Auth-Key", "app-secret")
		r.Header.Set("Authorization", "Bearer tok")
		r.Header.Set("If-None-Match", "*")
		w := httptest.NewRecorder()
		s.ServeHTTP(w, r)
		if w.Code != 401 {
			t.Fatalf("runtime credential on %s: %d %s", method, w.Code, w.Body.String())
		}
		w = httptest.NewRecorder()
		New(nil, true).ServeHTTP(w, r)
		if w.Code != 503 {
			t.Fatalf("learner exposed %s: %d", method, w.Code)
		}
	}
	for _, path := range []string{
		"/ops/providers/grant%0amanagement/enforcers/pep",
		"/ops/providers/grant-management/enforcers/policy%0dpep",
	} {
		pepPolicyCall(t, s, "GET", path, "", "", "", 422)
	}
}

func TestHTTPDecisionPolicyDTO(t *testing.T) {
	for _, value := range []any{grant.HttpCapabilityDecisionInput{}, grant.HttpCapabilityDecision{}} {
		raw, err := json.Marshal(value)
		if err != nil {
			t.Fatal(err)
		}
		if strings.Contains(string(raw), "wireVersion") || strings.Contains(string(raw), "pepPolicyRevision") {
			t.Fatalf("legacy wire changed: %s", raw)
		}
	}
	input := grant.HttpCapabilityDecisionInput{WireVersion: 2, PEPPolicyRevision: 7}
	raw, err := json.Marshal(input)
	if err != nil {
		t.Fatal(err)
	}
	r := httptest.NewRequest("POST", "/", strings.NewReader(string(raw)))
	r.Header.Set("Content-Type", "application/json")
	decoded, err := decode[grant.HttpCapabilityDecisionInput](r, true)
	if err != nil || decoded.WireVersion != 2 || decoded.PEPPolicyRevision != 7 {
		t.Fatalf("versioned decode: %+v %v", decoded, err)
	}
	for _, field := range []string{`"wireVersion":-1`, `"wireVersion":2.5`, `"wireVersion":"2"`, `"wireVersion":null`, `"authorizationMode":"user"`, `"applicationId":"forged"`, `"userId":"forged"`} {
		payload := strings.TrimSuffix(string(raw), "}")
		// Replace wireVersion rather than adding a duplicate key.
		if strings.HasPrefix(field, `"wireVersion"`) {
			payload = strings.Replace(payload, `"wireVersion":2`, field, 1)
		} else {
			payload += "," + field
		}
		r := httptest.NewRequest("POST", "/", strings.NewReader(payload+"}"))
		r.Header.Set("Content-Type", "application/json")
		if _, err := decode[grant.HttpCapabilityDecisionInput](r, true); err == nil {
			t.Fatalf("accepted %s", field)
		}
	}
	decision, err := json.Marshal(grant.HttpCapabilityDecision{WireVersion: 2, PEPPolicyRevision: 7})
	if err != nil || !strings.Contains(string(decision), `"wireVersion":2`) || !strings.Contains(string(decision), `"pepPolicyRevision":7`) {
		t.Fatalf("versioned response: %s %v", decision, err)
	}
}

func TestHTTPDecisionRejectsNegativePolicyRevision(t *testing.T) {
	s := managementTestServer(t)
	input := grant.HttpCapabilityDecisionInput{WireVersion: 2, PEPPolicyRevision: -1}
	raw, err := json.Marshal(input)
	if err != nil {
		t.Fatal(err)
	}
	r := httptest.NewRequest("POST", "/api/v2/http-capabilities/decide", strings.NewReader(string(raw)))
	r.Header.Set("Content-Type", "application/json")
	r.Header.Set("X-Grant-App-Auth-Key", "app-secret")
	r.Header.Set("Authorization", "Bearer tok")
	w := httptest.NewRecorder()
	s.ServeHTTP(w, r)
	if w.Code != 422 {
		t.Fatalf("negative policy revision: %d %s", w.Code, w.Body.String())
	}
}
