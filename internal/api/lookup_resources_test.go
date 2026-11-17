package api

import (
	"net/http/httptest"
	"strings"
	"testing"
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
}
