package api

import (
	"context"
	"encoding/json"
	"net/http/httptest"
	"slices"
	"strings"
	"testing"

	"github.com/mrchypark/grant/internal/grant"
)

func TestUnifiedFilterValidationAndLearnerRoutes(t *testing.T) {
	for _, learner := range []bool{false, true} {
		s := testServer()
		s.learner = learner
		cases := []struct {
			name, path, body string
			want             int
		}{
			{
				name: "v1 both resources and resource",
				path: "/api/v1/authz/filter",
				body: `{"subject":{"type":"user","id":"u","org_id":"o"},"action":"read","resources":[],"resource":{"provider":"p","type":"t"}}`,
				want: 400,
			},
			{
				name: "v1 neither resources nor resource",
				path: "/api/v1/authz/filter",
				body: `{"subject":{"type":"user","id":"u","org_id":"o"},"action":"read"}`,
				want: 422,
			},
			{
				name: "v1 limit with candidate resources",
				path: "/api/v1/authz/filter",
				body: `{"subject":{"type":"user","id":"u","org_id":"o"},"action":"read","resources":[],"limit":10}`,
				want: 400,
			},
			{
				name: "v1 limit out of bounds",
				path: "/api/v1/authz/filter",
				body: `{"subject":{"type":"user","id":"u","org_id":"o"},"action":"read","resource":{"provider":"p","type":"t"},"limit":0}`,
				want: 400,
			},
			{
				name: "v1 missing resource type",
				path: "/api/v1/authz/filter",
				body: `{"subject":{"type":"user","id":"u","org_id":"o"},"action":"read","resource":{"provider":"p","type":""}}`,
				want: 422,
			},
			{
				name: "v1 blank action with empty resources",
				path: "/api/v1/authz/filter",
				body: `{"subject":{"type":"user","id":"u","org_id":"o"},"action":"","resources":[]}`,
				want: 422,
			},
			{
				name: "v1 valid lookup reaches auth",
				path: "/api/v1/authz/filter",
				body: `{"subject":{"type":"user","id":"u","org_id":"o"},"action":"read","resource":{"provider":"p","type":"t"},"limit":50}`,
				want: 401,
			},
			{
				name: "app authz both resources and resource",
				path: "/api/v1/application-authz/filter",
				body: `{"action":"read","resources":[],"resource":{"provider":"p","type":"t"}}`,
				want: 400,
			},
			{
				name: "app authz limit out of bounds",
				path: "/api/v1/application-authz/filter",
				body: `{"action":"read","resource":{"provider":"p","type":"t"},"limit":1001}`,
				want: 400,
			},
			{
				name: "app authz valid lookup reaches auth",
				path: "/api/v1/application-authz/filter",
				body: `{"action":"read","resource":{"provider":"p","type":"t"},"limit":50,"cursor":"abc"}`,
				want: 401,
			},
			{
				name: "v2 provider mismatch",
				path: "/api/v2/providers/compos/authz/filter",
				body: `{"action":"read","resource":{"provider":"other","type":"t"}}`,
				want: 400,
			},
			{
				name: "v2 limit out of bounds",
				path: "/api/v2/providers/compos/authz/filter",
				body: `{"action":"read","resource":{"provider":"compos","type":"t"},"limit":0}`,
				want: 400,
			},
			{
				name: "v2 valid lookup reaches auth",
				path: "/api/v2/providers/compos/authz/filter",
				body: `{"action":"read","resource":{"provider":"compos","type":"t"},"limit":20}`,
				want: 401,
			},
		}
		for _, tc := range cases {
			t.Run(tc.name, func(t *testing.T) {
				r := httptest.NewRequest("POST", tc.path, strings.NewReader(tc.body))
				r.Header.Set("Content-Type", "application/json")
				if !strings.Contains(tc.name, "reaches auth") {
					if strings.HasPrefix(tc.path, "/api/v1/authz/") {
						r.Header.Set("X-Grant-Master-Key", "master-secret")
					} else if strings.HasPrefix(tc.path, "/api/v1/application-authz/") {
						r.Header.Set("X-Grant-App-Auth-Key", "app-secret")
					}
				}
				w := httptest.NewRecorder()
				s.ServeHTTP(w, r)
				if w.Code != tc.want {
					t.Fatalf("learner=%v %s got %d want %d: %s", learner, tc.path, w.Code, tc.want, w.Body.String())
				}
			})
		}
	}

	// Learner enforces closed-input decoding and rejects duplicate X-Grant-Master-Key headers on /api/v1/authz/*.
	ls := New(grant.NewService(grant.Config{Security: grant.SecurityConfig{MasterKey: "master-secret", AppAuthKey: "app-secret"}, Auth: grant.AuthConfig{ProviderMode: "pocketbase"}}, nil), true)
	r := httptest.NewRequest("POST", "/api/v1/authz/filter", strings.NewReader(`{"subject":{"type":"user","id":"u","org_id":"o"},"action":"read","resources":[],"extra":1}`))
	r.Header.Set("Content-Type", "application/json")
	r.Header.Set("X-Grant-Master-Key", "master-secret")
	w := httptest.NewRecorder()
	ls.ServeHTTP(w, r)
	if w.Code != 422 {
		t.Fatalf("learner unknown field got %d want 422: %s", w.Code, w.Body.String())
	}

	r = httptest.NewRequest("POST", "/api/v1/authz/check", strings.NewReader(`{"subject":{"type":"user","id":"u","org_id":"o"},"action":"read","resource":{"provider":"p","type":"t","id":"1"}}`))
	r.Header.Set("Content-Type", "application/json")
	r.Header.Add("X-Grant-Master-Key", "master-secret")
	r.Header.Add("X-Grant-Master-Key", "master-secret")
	w = httptest.NewRecorder()
	ls.ServeHTTP(w, r)
	if w.Code != 400 {
		t.Fatalf("learner duplicate master key got %d want 400: %s", w.Code, w.Body.String())
	}
}

func TestUnifiedFilterHTTP200WithStore(t *testing.T) {
	ctx := context.Background()
	cfg := grant.Config{
		Security: grant.SecurityConfig{MasterKey: "master-secret", AppAuthKey: "app-secret"},
		Auth:     grant.AuthConfig{ProviderMode: "pocketbase"},
		Database: grant.DatabaseConfig{DataDir: t.TempDir(), DefaultReadConsistency: grant.ReadStrong},
	}
	store, err := grant.OpenStore(ctx, cfg)
	if err != nil {
		t.Fatal(err)
	}
	defer store.Close()
	if err := store.EnsureBootstrapData(ctx); err != nil {
		t.Fatal(err)
	}
	for _, sql := range []string{
		"INSERT INTO users(id,display_name,active,created_at) VALUES('u-1','User 1',1,'now')",
		"INSERT INTO organization_memberships(id,organization_id,user_id,membership_role,status,created_at,updated_at) VALUES('om-1','conalog-grant','u-1','member','active','now','now')",
		"INSERT INTO service_providers(id,name,active,created_at,updated_at) VALUES('svc','Svc',1,'now','now')",
		"INSERT INTO actions(id,provider_id,code,name,risk_level,visibility_granting,grantable_to_applications,created_at,updated_at) VALUES('svc.read','svc','read','Read','low',0,0,'now','now')",
		"INSERT INTO roles(id,organization_id,code,name,created_at,updated_at) VALUES('r-1','conalog-grant','r-1','R1','now','now')",
		"INSERT INTO role_actions(role_id,action_id) VALUES('r-1','svc.read')",
		"INSERT INTO role_bindings(id,subject_type,subject_id,subject_organization_id,provider_id,resource_type,resource_id,role_id,state,created_by_user_id,created_at,updated_at) VALUES" +
			"('rb-1','user','u-1','conalog-grant','svc','ws','ws-1','r-1','active','conalog-grant','now','now')," +
			"('rb-2','user','u-1','conalog-grant','svc','ws','ws-2','r-1','active','conalog-grant','now','now')",
	} {
		if _, err := store.Exec(ctx, sql); err != nil {
			t.Fatal(err)
		}
	}

	s := New(grant.NewService(cfg, store), false)
	post := func(body string) map[string]any {
		t.Helper()
		req := httptest.NewRequest("POST", "/api/v1/authz/filter", strings.NewReader(body))
		req.Header.Set("Content-Type", "application/json")
		req.Header.Set("X-Grant-Master-Key", "master-secret")
		rec := httptest.NewRecorder()
		s.ServeHTTP(rec, req)
		if rec.Code != 200 {
			t.Fatalf("got %d want 200: %s", rec.Code, rec.Body.String())
		}
		var out map[string]any
		if err := json.Unmarshal(rec.Body.Bytes(), &out); err != nil {
			t.Fatal(err)
		}
		return out
	}

	// Page 1 in reverse-lookup mode (limit=1).
	p1 := post(`{"subject":{"type":"user","id":"u-1","org_id":"conalog-grant"},"action":"svc.read","resource":{"provider":"svc","type":"ws"},"limit":1}`)
	if ids := p1["resource_ids"].([]any); len(ids) != 1 || ids[0] != "ws-1" || p1["next_cursor"] != "ws-1" {
		t.Fatalf("unexpected page1: %v", p1)
	}

	// Page 2 using cursor.
	p2 := post(`{"subject":{"type":"user","id":"u-1","org_id":"conalog-grant"},"action":"svc.read","resource":{"provider":"svc","type":"ws"},"limit":1,"cursor":"ws-1"}`)
	if ids := p2["resource_ids"].([]any); len(ids) != 1 || ids[0] != "ws-2" || p2["next_cursor"] != nil {
		t.Fatalf("unexpected page2: %v", p2)
	}

	// Candidate-filter mode still works identically.
	cand := post(`{"subject":{"type":"user","id":"u-1","org_id":"conalog-grant"},"action":"svc.read","resources":[{"provider":"svc","type":"ws","id":"ws-2"},{"provider":"svc","type":"ws","id":"ws-missing"}]}`)
	rawIDs := cand["resource_ids"].([]any)
	got := []string{}
	for _, v := range rawIDs {
		got = append(got, v.(string))
	}
	if !slices.Equal(got, []string{"ws-2"}) {
		t.Fatalf("unexpected candidate filter: %v", cand)
	}
}
