package api

import (
	"context"
	"crypto/sha256"
	"encoding/json"
	"fmt"
	"net/http/httptest"
	"strings"
	"testing"

	"github.com/mrchypark/grant/internal/grant"
)

func TestOwnerCustomActionEligibilityHTTP(t *testing.T) {
	for _, provider := range []string{"operable", "independent-authz"} {
		t.Run(provider, func(t *testing.T) {
			s, store := managementTestServerWithStore(t)
			if _, err := s.service.UpsertServiceProviderAdmin(context.Background(), grant.ServiceProvider{ID: provider, Name: provider, Active: true}); err != nil {
				t.Fatal(err)
			}
			path := "/api/v1/orgs/mgmt-org/providers/" + provider + "/actions"
			call := func(method, path, body, match string, want int) *httptest.ResponseRecorder {
				t.Helper()
				r := httptest.NewRequest(method, path, strings.NewReader(body))
				r.Header.Set("Authorization", "Bearer tok")
				r.Header.Set("X-Organization-ID", "mgmt-org")
				if body != "" {
					r.Header.Set("Content-Type", "application/json")
				}
				if match != "" {
					r.Header.Set("If-Match", match)
				}
				w := httptest.NewRecorder()
				s.ServeHTTP(w, r)
				if w.Code != want {
					t.Fatalf("%s %s: got %d want %d: %s", method, path, w.Code, want, w.Body.String())
				}
				return w
			}
			digest := sha256.Sum256([]byte("mgmt-org"))
			created := call("POST", path, fmt.Sprintf(`{"code":"custom.org-%x.calendar.event.create","name":"Create","riskLevel":"low"}`, digest), "", 201)
			var a grant.Action
			if err := json.Unmarshal(created.Body.Bytes(), &a); err != nil {
				t.Fatal(err)
			}
			if a.ID != provider+"."+a.Code || a.GrantableToApplications || a.VisibilityGranting {
				t.Fatalf("unexpected custom action: %+v", a)
			}
			item := path + "/" + a.ID
			read := call("GET", path, "", "", 200)
			etag := read.Header().Get("ETag")
			if etag == "" || !strings.Contains(read.Body.String(), a.ID) {
				t.Fatal("owner cannot snapshot ineligible action", read.Body.String())
			}
			for _, body := range []string{`{"grantableToApplications":null}`, `{"grantableToApplications":"true"}`, `{"grantableToApplications":1}`} {
				call("PATCH", item, body, etag, 422)
			}
			call("PATCH", item, `{"grantableToApplications":true}`, "", 428)
			call("PATCH", item, `{"grantableToApplications":true}`, `"stale"`, 412)
			call("PATCH", item, `{"grantableToApplications":true}`, etag, 200)
			// Idempotent booleans still require a current catalog precondition.
			call("PATCH", item, `{"grantableToApplications":true}`, "", 428)
			call("PATCH", item, `{"grantableToApplications":true}`, etag, 412)
			currentETag := call("GET", path, "", "", 200).Header().Get("ETag")
			call("PATCH", item, `{"grantableToApplications":true}`, currentETag, 200)
			for _, table := range []string{"application_actions", "application_grant_actions", "role_actions"} {
				rows, err := store.Query(context.Background(), grant.ReadStrong, "SELECT 1 FROM "+table+" WHERE action_id=?", a.ID)
				if err != nil || len(rows) != 0 {
					t.Fatalf("eligibility implicitly changed %s: %v %v", table, rows, err)
				}
			}
			call("PATCH", item, `{"grantableToApplications":false}`, etag, 412)
			renamed := call("PATCH", item, `{"name":"Renamed"}`, "", 200)
			if !strings.Contains(renamed.Body.String(), `"grantableToApplications":true`) {
				t.Fatal("omission cleared eligibility")
			}
			etag = call("GET", path, "", "", 200).Header().Get("ETag")
			// The existing fixture app is explicitly ready, non-first-party, and uses
			// explicit consent. This establishes no live Goauthy client mapping.
			scopes := "/api/v1/orgs/mgmt-org/apps/app-1/actions"
			appETag := call("GET", scopes, "", "", 200).Header().Get("ETag")
			admission := `{"actionIds":["grant-management.admin.provider.create","` + a.ID + `"]}`
			call("PUT", scopes, admission, "", 428)
			call("PUT", scopes, admission, appETag, 200)
			call("PUT", scopes, admission, appETag, 412)
			rows, err := store.Query(context.Background(), grant.ReadStrong, "SELECT 1 FROM application_grant_actions WHERE action_id=?", a.ID)
			if err != nil || len(rows) != 0 {
				t.Fatalf("admission implicitly consented: %v %v", rows, err)
			}
			call("PATCH", item, `{"grantableToApplications":false}`, etag, 409)
			appETag = call("GET", scopes, "", "", 200).Header().Get("ETag")
			call("PUT", scopes, `{"actionIds":["grant-management.admin.provider.create"]}`, appETag, 200)
			call("PATCH", item, `{"grantableToApplications":false}`, etag, 200)
			call("PATCH", path+"/organization.profile.read", `{"grantableToApplications":true}`, etag, 412)
			etag = call("GET", path, "", "", 200).Header().Get("ETag")
			call("PATCH", path+"/organization.profile.read", `{"grantableToApplications":true}`, etag, 409)
			call("DELETE", item, "", "", 204)
		})
	}
}

func TestOwnerCustomActionCatalogVisibilityHTTP(t *testing.T) {
	for _, provider := range []string{"operable", "independent-authz"} {
		t.Run(provider, func(t *testing.T) {
			s, store := managementTestServerWithStore(t)
			ctx := context.Background()
			if _, err := s.service.UpsertServiceProviderAdmin(ctx, grant.ServiceProvider{ID: provider, Name: provider, Active: true}); err != nil {
				t.Fatal(err)
			}
			own := fmt.Sprintf("custom.org-%x.calendar.event.create", sha256.Sum256([]byte("mgmt-org")))
			foreign := fmt.Sprintf("custom.org-%x.calendar.event.create", sha256.Sum256([]byte("other-org")))
			if _, err := store.Exec(ctx, "INSERT INTO organizations(id,name,created_at,updated_at) VALUES('other-org','Other organization','now','now')"); err != nil {
				t.Fatal(err)
			}
			if _, err := store.Exec(ctx, "INSERT INTO organization_memberships(id,organization_id,user_id,membership_role,status,created_at,updated_at) VALUES('mem-other-owner','other-org','owner','owner','active','now','now')"); err != nil {
				t.Fatal(err)
			}
			// Two independent organization namespaces, plus the legacy eligible catalog.
			for _, a := range []grant.Action{
				{ID: provider + "." + own, ProviderID: provider, Code: own, Name: "Own private metadata", RiskLevel: "low"},
				{ID: provider + "." + foreign, ProviderID: provider, Code: foreign, Name: "Foreign private metadata", RiskLevel: "low"},
				{ID: provider + ".eligible", ProviderID: provider, Code: "eligible", Name: "Eligible", RiskLevel: "low", GrantableToApplications: true},
			} {
				if _, err := s.service.CreateProviderAction(ctx, provider, a); err != nil {
					t.Fatal(err)
				}
			}
			call := func(method, path, body, etag string) *httptest.ResponseRecorder {
				t.Helper()
				r := httptest.NewRequest(method, path, strings.NewReader(body))
				r.Header.Set("Authorization", "Bearer tok")
				r.Header.Set("X-Organization-ID", "mgmt-org")
				if strings.Contains(path, "/orgs/other-org/") {
					r.Header.Set("X-Organization-ID", "other-org")
				}
				r.Header.Set("Content-Type", "application/json")
				if etag != "" {
					r.Header.Set("If-Match", etag)
				}
				w := httptest.NewRecorder()
				s.ServeHTTP(w, r)
				return w
			}
			path := "/api/v1/orgs/mgmt-org/providers/" + provider + "/actions"
			read := call("GET", path, "", "")
			if read.Code != 200 || !strings.Contains(read.Body.String(), own) || !strings.Contains(read.Body.String(), provider+".eligible") || strings.Contains(read.Body.String(), foreign) || strings.Contains(read.Body.String(), "Foreign private metadata") {
				t.Fatalf("owner catalog leak: %d %s", read.Code, read.Body.String())
			}
			etag := read.Header().Get("ETag")
			if etag == "" {
				t.Fatal("missing catalog ETag")
			}
			// Foreign ineligible metadata changes must neither leak nor invalidate this view.
			if _, err := store.Exec(ctx, "UPDATE actions SET name='Other hidden metadata' WHERE id=?", provider+"."+foreign); err != nil {
				t.Fatal(err)
			}
			patched := call("PATCH", path+"/"+provider+"."+own, `{"grantableToApplications":true}`, etag)
			if patched.Code != 200 {
				t.Fatalf("GET/PATCH validator mismatch: %d %s", patched.Code, patched.Body.String())
			}
			otherPath := "/api/v1/orgs/other-org/providers/" + provider + "/actions"
			other := call("GET", otherPath, "", "")
			if other.Code != 200 || strings.Contains(other.Body.String(), own) || strings.Contains(other.Body.String(), "Own private metadata") || !strings.Contains(other.Body.String(), foreign) {
				t.Fatalf("A opt-in exposed custom metadata to B: %d %s", other.Code, other.Body.String())
			}
			if _, err := store.Exec(ctx, "UPDATE actions SET grantable_to_applications=1 WHERE id=?", provider+"."+foreign); err != nil {
				t.Fatal(err)
			}
			read = call("GET", path, "", "")
			if read.Code != 200 || strings.Contains(read.Body.String(), foreign) || strings.Contains(read.Body.String(), "Other hidden metadata") {
				t.Fatalf("B opt-in exposed custom metadata to A: %d %s", read.Code, read.Body.String())
			}
			// Retain actual owner membership and app.manage while removing permission.manage.
			if _, err := store.Exec(ctx, "DELETE FROM role_actions WHERE action_id='organization.permission.manage'"); err != nil {
				t.Fatal(err)
			}
			// Make own action ineligible again to distinguish the legacy candidate view.
			if _, err := store.Exec(ctx, "UPDATE actions SET grantable_to_applications=0 WHERE id=?", provider+"."+own); err != nil {
				t.Fatal(err)
			}
			read = call("GET", path, "", "")
			if read.Code != 200 || !strings.Contains(read.Body.String(), provider+".eligible") || strings.Contains(read.Body.String(), own) || strings.Contains(read.Body.String(), foreign) {
				t.Fatalf("app-manager owner fallback regressed: %d %s", read.Code, read.Body.String())
			}
			other = call("GET", otherPath, "", "")
			if other.Code != 200 || !strings.Contains(other.Body.String(), foreign) || strings.Contains(other.Body.String(), own) {
				t.Fatalf("B fallback lost own eligible action or leaked A: %d %s", other.Code, other.Body.String())
			}
			candidates, err := s.service.ListProviderActionsAuthorized(ctx, "owner", "mgmt-org", provider)
			if err != nil || len(candidates) != 1 || candidates[0].ID != provider+".eligible" {
				t.Fatalf("service candidate catalog differs from public view: %+v %v", candidates, err)
			}
		})
	}
}
