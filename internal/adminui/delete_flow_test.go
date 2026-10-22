package adminui

import (
	"net/http"
	"net/http/httptest"
	"net/url"
	"strings"
	"testing"
)

func TestDeleteClearsBrowserSelectionAndReloadsList(t *testing.T) {
	for _, tc := range []struct{ section, operation string }{{"groups", opGroupDelete}, {"roles", opRoleDelete}, {"bindings", opBindingDelete}} {
		for _, failed := range []bool{false, true} {
			name := tc.section + "/success"
			if failed {
				name = tc.section + "/failed"
			}
			t.Run(name, func(t *testing.T) {
				deletes, deletedReads := 0, 0
				proxy := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
					if r.Method == http.MethodDelete {
						deletes++
						if failed {
							http.Error(w, `{"error":"delete rejected"}`, http.StatusConflict)
						} else {
							w.WriteHeader(http.StatusNoContent)
						}
						return
					}
					if strings.Contains(r.URL.Path, "/deleted-record") {
						deletedReads++
						http.NotFound(w, r)
						return
					}
					w.Header().Set("Content-Type", "application/json")
					_, _ = w.Write([]byte(`[]`))
				}))
				defer proxy.Close()
				ui, err := New(Config{APIURL: proxy.URL, ManagementOrganizationID: "management-org"})
				if err != nil {
					t.Fatal(err)
				}
				requestWithHeaderTestSession(t, http.HandlerFunc(func(w http.ResponseWriter, sessionRequest *http.Request) {
					form := url.Values{"section": {tc.section}, "operation": {tc.operation}, "org": {"selected-org"}, "provider": {"selected-provider"}, "id": {"deleted-record"}}
					mutation := httptest.NewRequest(http.MethodPost, "/operations", strings.NewReader(form.Encode())).WithContext(sessionRequest.Context())
					mutation.Header.Set("Content-Type", "application/x-www-form-urlencoded")
					mutation.Header.Set("HX-Request", "true")
					mutation.Header.Set("HX-Current-URL", "https://admin.example.test/"+tc.section+"?id=deleted-record&org=selected-org&provider=selected-provider")
					result := httptest.NewRecorder()
					ui.ServeHTTP(result, mutation)
					target := result.Header().Get("HX-Replace-Url")
					if failed {
						if target != "" || result.Header().Get("HX-Redirect") != "" {
							t.Fatalf("failure changed URL: %q", target)
						}
						if !strings.Contains(result.Body.String(), "delete rejected") {
							t.Fatal("failure not rendered")
						}
						return
					}
					want := "/" + tc.section + "?org=selected-org&provider=selected-provider"
					if result.Code != 200 || target != want {
						t.Fatalf("status=%d target=%q want=%q", result.Code, target, want)
					}
					if !strings.Contains(result.Body.String(), `id="admin-content"`) {
						t.Fatal("missing HTMX list fragment")
					}
					reload := httptest.NewRequest(http.MethodGet, target, nil).WithContext(sessionRequest.Context())
					reloaded := httptest.NewRecorder()
					ui.ServeHTTP(reloaded, reload)
					if reloaded.Code != 200 {
						t.Fatalf("reload=%d", reloaded.Code)
					}
				}), "/test")
				if deletes != 1 || deletedReads != 0 {
					t.Fatalf("deletes=%d deleted entity reads=%d", deletes, deletedReads)
				}
			})
		}
	}
}
