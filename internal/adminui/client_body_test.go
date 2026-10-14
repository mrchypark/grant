package adminui

import (
	"io"
	"net/http"
	"net/http/httptest"
	"net/url"

	"strings"
	"testing"

	"github.com/Conalog/grant/internal/grant"
)

func TestClientUsesCatalogBodyContracts(t *testing.T) {
	catalog, _, err := grant.ManagementCatalog(1, nil)
	if err != nil {
		t.Fatal(err)
	}
	for _, capability := range catalog.Capabilities {
		t.Run(capability.Method+" "+capability.PathTemplate, func(t *testing.T) {
			calls := 0
			proxy := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
				calls++
				body, _ := io.ReadAll(r.Body)
				if capability.Request.Body == "none" {
					if len(body) != 0 || r.Header.Get("Content-Type") != "" {
						t.Errorf("bodyless request: body=%q content-type=%q", body, r.Header.Get("Content-Type"))
					}
				} else if string(body) != `{"name":"preserved"}` || r.Header.Get("Content-Type") != "application/json" {
					t.Errorf("JSON payload changed: %q", body)
				}
				w.WriteHeader(http.StatusNoContent)
			}))
			defer proxy.Close()
			client, err := newAPIClient(proxy.URL, nil)
			if err != nil {
				t.Fatal(err)
			}
			vars := map[string]string{}
			for _, part := range strings.Split(capability.PathTemplate, "/") {
				if strings.HasPrefix(part, "{") {
					vars[strings.Trim(part, "{}")] = "test"
				}
			}
			requestWithHeaderTestSession(t, http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
				body := map[string]any{"name": "preserved"}
				if capability.Request.Body == "none" {
					body = map[string]any{}
				}
				if _, _, err := client.call(r.Context(), capability.Method, capability.PathTemplate, vars, nil, body, "org", "org"); err != nil {
					t.Fatal(err)
				}
				if capability.Request.Body == "none" {
					for _, unexpected := range []any{map[string]any{"unexpected": true}, []string{}, "payload"} {
						if _, _, err := client.call(r.Context(), capability.Method, capability.PathTemplate, vars, nil, unexpected, "org", "org"); err == nil {
							t.Error("unexpected payload accepted")
						}
					}
				}
			}), "/test")
			if calls != 1 {
				t.Fatalf("upstream calls=%d", calls)
			}
		})
	}
}

func TestUIBodylessOperationsThroughProxy(t *testing.T) {
	for _, op := range []string{opActionDelete, opOrganizationDeactivate, opOrganizationReactivate, opMemberEnroll, opMemberRemove, opGroupDelete, opGroupMemberAdd, opGroupMemberRemove, opRoleDelete, opBindingDelete, opApplicationDelete, opApplicationConsentRevoke, opGrantRevoke} {
		t.Run(op, func(t *testing.T) {
			calls := 0
			proxy := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
				calls++
				b, _ := io.ReadAll(r.Body)
				if len(b) > 0 || r.Header.Get("Content-Type") != "" {
					http.Error(w, "invalid_request", 422)
					return
				}
				w.WriteHeader(204)
			}))
			defer proxy.Close()
			client, err := newAPIClient(proxy.URL, nil)
			if err != nil {
				t.Fatal(err)
			}
			method, route, vars, body, headers, err := operationRequest("", "provider", "org", url.Values{"operation": {op}, "id": {"test"}, "userId": {"user"}})
			if err != nil {
				t.Fatal(err)
			}
			requestWithHeaderTestSession(t, http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
				if _, status, err := client.call(r.Context(), method, route, vars, nil, body, "org", "org", headers); err != nil || status != 204 {
					t.Fatalf("status=%d err=%v", status, err)
				}
			}), "/test")
			if calls != 1 {
				t.Fatal("mutation not sent")
			}
		})
	}
}
