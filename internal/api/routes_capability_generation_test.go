package api

import (
	"bytes"
	"context"
	"encoding/json"
	"net/http/httptest"
	"reflect"
	"testing"

	"github.com/mrchypark/grant/internal/grant"
)

const generationReadPath = "/api/v1/admin/providers/grant/http-capability-generations/"

func TestAdminHttpCapabilityGenerationRead(t *testing.T) {
	s, store := managementTestServerWithStore(t)
	cap := grant.HttpCapability{CapabilityID: "health", Version: 1, Enabled: true, AccessMode: "public_authn", Method: "GET", PathTemplate: "/health", Query: grant.HttpQueryPolicy{Mode: "none"}, Request: grant.HttpRequestPolicy{Body: "none", ForwardHeaders: []string{}}, Response: grant.HttpResponsePolicy{MaxBytes: 1024, ForwardHeaders: []string{}, Redirects: "deny"}}
	var err error
	cap.ContractDigest, err = grant.HttpCapabilityContractDigest(cap)
	if err != nil {
		t.Fatal(err)
	}
	stage := grant.StageHttpCapabilityGeneration{SchemaVersion: 1, Generation: 1, Capabilities: []grant.HttpCapability{cap}}
	stage.CatalogDigest, err = grant.HttpCapabilityCatalogDigest("grant", stage)
	if err != nil {
		t.Fatal(err)
	}
	raw, err := json.Marshal(stage)
	if err != nil {
		t.Fatal(err)
	}
	post := httptest.NewRequest("POST", generationReadPath[:len(generationReadPath)-1], bytes.NewReader(raw))
	post.Header.Set("X-Grant-Master-Key", "master-secret")
	post.Header.Set("Content-Type", "application/json")
	w := httptest.NewRecorder()
	s.ServeHTTP(w, post)
	if w.Code != 201 {
		t.Fatalf("stage: %d %s", w.Code, w.Body.String())
	}
	request := func(path, credential, org string, want int) *httptest.ResponseRecorder {
		t.Helper()
		r := httptest.NewRequest("GET", path, nil)
		if credential == "master" {
			r.Header.Set("X-Grant-Master-Key", "master-secret")
		}
		if credential == "bearer" || credential == "invalid-master" {
			r.Header.Set("Authorization", "Bearer tok")
		}
		if credential == "invalid-master" {
			r.Header.Set("X-Grant-Master-Key", "invalid")
		}
		if org != "" {
			r.Header.Set("X-Organization-ID", org)
		}
		w := httptest.NewRecorder()
		s.ServeHTTP(w, r)
		if w.Code != want {
			t.Fatalf("GET %s credential=%s: %d want %d: %s", path, credential, w.Code, want, w.Body.String())
		}
		return w
	}
	request(generationReadPath+"1", "", "", 401)
	request(generationReadPath+"1", "bearer", "wrong-org", 403)
	request(generationReadPath+"1", "bearer", "mgmt-org", 403)
	exec := func(sql string) {
		t.Helper()
		if _, err := store.Exec(context.Background(), sql); err != nil {
			t.Fatal(err)
		}
	}
	exec("INSERT INTO actions(id,provider_id,code,name,risk_level,visibility_granting,grantable_to_applications,created_at,updated_at) VALUES('grant-management.admin.catalog.read','grant-management','admin.catalog.read','Read catalogs','high',0,1,'now','now')")
	exec("INSERT INTO role_actions(role_id,action_id) VALUES('role-mgmt','grant-management.admin.catalog.read')")
	request(generationReadPath+"1", "bearer", "mgmt-org", 403)
	exec("INSERT INTO application_actions(application_id,action_id) VALUES('app-1','grant-management.admin.catalog.read')")
	request(generationReadPath+"1", "bearer", "mgmt-org", 403)
	exec("INSERT INTO application_grant_actions(application_grant_id,action_id) VALUES('g1','grant-management.admin.catalog.read')")
	for _, credential := range []string{"master", "bearer"} {
		w := request(generationReadPath+"1", credential, "mgmt-org", 200)
		var got grant.AdminHttpCapabilityGeneration
		if err := json.Unmarshal(w.Body.Bytes(), &got); err != nil {
			t.Fatal(err)
		}
		if got.ProviderID != "grant" || got.Generation != 1 || got.CatalogDigest != stage.CatalogDigest || got.CapabilityCount != 1 || got.ResponseResourceCount != 0 || !reflect.DeepEqual(got.Capabilities, stage.Capabilities) || got.ResponseResources == nil {
			t.Fatalf("generation response: %+v", got)
		}
		var fields map[string]json.RawMessage
		if err := json.Unmarshal(w.Body.Bytes(), &fields); err != nil {
			t.Fatal(err)
		}
		for _, name := range []string{"providerId", "generation", "catalogDigest", "capabilityCount", "responseResourceCount", "capabilities", "responseResources"} {
			if fields[name] == nil {
				t.Fatalf("missing %s", name)
			}
		}
		if len(fields) != 7 {
			t.Fatalf("unexpected activation/lease metadata: %v", fields)
		}
	}
	request("/api/v1/admin/providers/grant/http-capability-active", "master", "", 404)
	request(generationReadPath+"1", "invalid-master", "mgmt-org", 401)
	exec("DELETE FROM role_actions WHERE role_id='role-mgmt' AND action_id='grant-management.admin.catalog.read'")
	request(generationReadPath+"1", "bearer", "mgmt-org", 403)
	for _, generation := range []string{"0", "-1", "+1", "abc", "1.0", "1e2", "%201", "9223372036854775808", "18446744073709551615", "18446744073709551616"} {
		request(generationReadPath+generation, "master", "", 400)
	}
	for _, generation := range []string{"2", "9223372036854775807"} {
		request(generationReadPath+generation, "master", "", 404)
	}
	request("/api/v1/admin/providers/missing/http-capability-generations/1", "master", "", 404)
	s.learner = true
	request(generationReadPath+"1", "master", "", 503)
}
