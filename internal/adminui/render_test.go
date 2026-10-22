package adminui

import (
	"net/http"
	"net/http/httptest"
	"strings"
	"testing"
)

func TestRenderPageAndWorkspace(t *testing.T) {
	r, err := newRenderer()
	if err != nil {
		t.Fatal(err)
	}
	p := Page{
		Title: "제공자", Description: "등록된 제공자", Section: "providers",
		OrganizationID: "org-a", ProviderID: "provider-a", Search: "clinic",
		CSRF: "csrf-<token>", Notice: "저장 완료", Error: "검토 필요",
		Navigation:    []NavItem{{ID: "providers", Label: "제공자", Group: "서비스 정의", Href: "/admin/providers"}},
		Organizations: []Option{{Value: "org-a", Label: "기관 A"}},
		Providers:     []Option{{Value: "provider-a", Label: "제공자 A"}},
		Columns:       []string{"이름", "설명"},
		Rows:          []Row{{Cells: []string{"<script>alert(1)</script>", "항목"}, Link: "/admin/providers/1"}},
		Tables:        []Table{{Title: "연결된 그룹", Columns: []string{"이름", "역할"}, Rows: []Row{{Cells: []string{"<img src=x>", "멤버"}, Link: "/groups/group-1/members"}}}},
		Forms: []Form{{
			ID: "provider-edit", Title: "제공자 수정", Action: "/admin/providers/save", SubmitLabel: "저장",
			Hidden: map[string]string{"operation": "update", "id": "1"},
			Fields: []Field{
				{Name: "name", Label: "이름", Value: "<b>값</b>", Required: true},
				{Name: "config", Label: "설정", Type: "json", Value: `{"enabled":true}`},
				{Name: "enabled", Label: "사용", Type: "boolean", Value: "true"},
			},
		}},
		Detail: "{\"safe\": \"<script>\"}", NextURL: "/admin/providers?cursor=next&filter=active",
	}
	for _, partial := range []bool{false, true} {
		var out strings.Builder
		if err := r.render(&out, p, partial); err != nil {
			t.Fatal(err)
		}
		body := out.String()
		if strings.Count(body, `id="admin-content"`) != 1 || !strings.Contains(body, `aria-label="조직 선택"`) || !strings.Contains(body, `hx-target="#admin-content"`) {
			t.Errorf("scope selectors and content must refresh together (partial=%t)", partial)
		}
		if !strings.Contains(body, `<main id="workspace"`) || !strings.Contains(body, `name="csrf_token" value="csrf-&lt;token&gt;"`) {
			t.Fatalf("workspace or escaped CSRF token missing (partial=%t): %s", partial, body)
		}
		for _, want := range []string{`name="operation" value="update"`, `name="section" value="providers"`, `name="org" value="org-a"`, `name="provider" value="provider-a"`, `name="id" value="1"`, `cursor=next&amp;filter=active&amp;org=org-a&amp;provider=provider-a&amp;q=clinic`, "&lt;script&gt;alert(1)&lt;/script&gt;", "&lt;b&gt;값&lt;/b&gt;", "&lt;script&gt;", `name="config"`, `<textarea`, `name="enabled"`, `type="checkbox"`, `checked`, `<caption class="sr-only">연결된 그룹 목록</caption>`, `<a href="/groups/group-1/members">&lt;img src=x&gt;</a>`} {
			if !strings.Contains(body, want) {
				t.Errorf("partial=%t missing %q in %s", partial, want, body)
			}
		}
		if strings.Contains(body, "<script>alert(1)") || strings.Contains(body, "<b>값</b>") {
			t.Errorf("untrusted view data was not escaped (partial=%t)", partial)
		}
		if partial && strings.Contains(body, "<!doctype html>") {
			t.Error("partial render returned a full document")
		}
		if !partial && (!strings.Contains(body, "<!doctype html>") || !strings.Contains(body, "/logout")) {
			t.Error("full render missing document shell or logout form")
		}
		if !partial && (!strings.Contains(body, `href="/" aria-label="Grant 관리 홈"`) || !strings.Contains(body, "조직 선택") || !strings.Contains(body, "프로바이더 선택")) {
			t.Error("full render missing root navigation or domain selector labels")
		}
		if strings.Contains(body, "사용자 검색") {
			t.Error("users-only search appeared outside the users section")
		}
		if strings.Contains(body, "<script>") {
			t.Errorf("inline script violates the self-only CSP contract (partial=%t)", partial)
		}
	}
}

func TestRenderPaginationPreservesExistingFilters(t *testing.T) {
	r, err := newRenderer()
	if err != nil {
		t.Fatal(err)
	}
	var out strings.Builder
	if err := r.render(&out, Page{Title: "사용자", Section: "users", Search: "kim", NextURL: "?cursor=next&status=active"}, false); err != nil {
		t.Fatal(err)
	}
	if !strings.Contains(out.String(), `href="?cursor=next&amp;q=kim&amp;status=active"`) {
		t.Fatalf("next link did not preserve filters: %s", out.String())
	}
	if !strings.Contains(out.String(), `aria-label="사용자 검색"`) {
		t.Fatal("users page did not identify the search as user search")
	}
}

func TestFormHiddenDefaultsAndOverrides(t *testing.T) {
	p := Page{Section: "users", OrganizationID: "page-org", ProviderID: "page-provider", CSRF: "page-csrf"}
	got := formHidden(p, Form{Hidden: map[string]string{
		"operation": "update", "section": "form-section", "org": "form-org", "provider": "form-provider", "csrf_token": "old-token",
	}})
	for key, want := range map[string]string{
		"operation": "update", "section": "form-section", "org": "form-org", "provider": "form-provider", "csrf_token": "page-csrf",
	} {
		if got[key] != want {
			t.Errorf("form hidden %s = %q, want %q", key, got[key], want)
		}
	}
}

func TestLocalizedResourceLabelsKeepJSONFieldMapping(t *testing.T) {
	res := resources["providers"]
	if len(res.columns) != len(res.jsonFields) {
		t.Fatalf("provider labels/JSON fields differ in length: %d/%d", len(res.columns), len(res.jsonFields))
	}
	if strings.Join(res.columns, ",") != "ID,이름,설명,활성" {
		t.Fatalf("provider columns = %v", res.columns)
	}
	if strings.Join(res.jsonFields, ",") != "id,name,description,active" {
		t.Fatalf("provider JSON fields changed: %v", res.jsonFields)
	}
	for key, resource := range resources {
		if len(resource.columns) != len(resource.jsonFields) {
			t.Errorf("resource %q labels/JSON fields differ in length: %d/%d", key, len(resource.columns), len(resource.jsonFields))
		}
	}
	rows, err := rowsFromJSON([]byte(`[{"id":"provider-1","name":"예시 프로바이더","description":"권한 서비스","active":true}]`), "providers")
	if err != nil {
		t.Fatal(err)
	}
	if len(rows) != 1 || strings.Join(rows[0].Cells, ",") != "provider-1,예시 프로바이더,권한 서비스,true" {
		t.Fatalf("provider fields were not read from stable JSON keys: %+v", rows)
	}
}

func TestBindAssets(t *testing.T) {
	mux := http.NewServeMux()
	if err := bindAssets(mux); err != nil {
		t.Fatal(err)
	}
	for path, wantType := range map[string]string{
		"/assets/grant-admin.css": "text/css; charset=utf-8",
		"/assets/grant-admin.js":  "text/javascript; charset=utf-8",
		"/assets/htmx.min.js":     "text/javascript",
	} {
		w := httptest.NewRecorder()
		mux.ServeHTTP(w, httptest.NewRequest(http.MethodGet, path, nil))
		if w.Code != http.StatusOK || !strings.HasPrefix(w.Header().Get("Content-Type"), wantType) || w.Body.Len() == 0 {
			t.Errorf("GET %s: status=%d content-type=%q body=%d", path, w.Code, w.Header().Get("Content-Type"), w.Body.Len())
		}
		if path == "/assets/grant-admin.js" && strings.Contains(w.Body.String(), "htmx:before:swap") {
			t.Error("admin script must not rely on mutating the already-computed HTMX v4 swap from before:swap")
		}
	}
}
