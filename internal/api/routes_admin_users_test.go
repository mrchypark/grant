package api

import (
	"context"
	"encoding/json"
	"fmt"
	"net/http/httptest"
	"net/url"
	"reflect"
	"strings"
	"testing"

	"github.com/Conalog/grant/internal/grant"
)

func directoryRequest(t *testing.T, s *Server, query, credential, org string, want int) *httptest.ResponseRecorder {
	t.Helper()
	r := httptest.NewRequest("GET", "/api/v1/admin/users"+query, nil)
	if credential == "master" {
		r.Header.Set("X-Grant-Master-Key", "master-secret")
	}
	if credential == "bearer" {
		r.Header.Set("Authorization", "Bearer tok")
	}
	if org != "" {
		r.Header.Set("X-Organization-ID", org)
	}
	w := httptest.NewRecorder()
	s.ServeHTTP(w, r)
	if w.Code != want {
		t.Fatalf("query=%q credential=%s org=%s: got %d, want %d: %s", query, credential, org, w.Code, want, w.Body.String())
	}
	return w
}

func decodeDirectory(t *testing.T, w *httptest.ResponseRecorder) grant.UserDirectory {
	t.Helper()
	var page grant.UserDirectory
	if err := json.Unmarshal(w.Body.Bytes(), &page); err != nil {
		t.Fatal(err)
	}
	if page.Items == nil {
		t.Fatal("items must be an array, not null")
	}
	if page.NextCursor == "" && strings.Contains(w.Body.String(), `"nextCursor"`) {
		t.Fatal("empty nextCursor must be omitted")
	}
	return page
}

func directoryTestServer(t *testing.T) (*Server, *grant.Store) {
	t.Helper()
	cfg := grant.Config{
		Security: grant.SecurityConfig{MasterKey: "master-secret"},
		Database: grant.DatabaseConfig{DataDir: t.TempDir(), DefaultReadConsistency: grant.ReadLocal},
	}
	cfg.Database.Rhiza.Local = true
	store, err := grant.OpenStore(context.Background(), cfg)
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() { _ = store.Close() })
	return New(grant.NewService(cfg, store), false), store
}

func TestAdminUsersAuthorization(t *testing.T) {
	s, store := managementTestServerWithStore(t)
	exec := func(sql string) {
		t.Helper()
		if _, err := store.Exec(context.Background(), sql); err != nil {
			t.Fatal(err)
		}
	}
	directoryRequest(t, s, "", "", "", 401)
	directoryRequest(t, s, "", "bearer", "wrong-org", 403)
	directoryRequest(t, s, "", "bearer", "mgmt-org", 403)
	decodeDirectory(t, directoryRequest(t, s, "", "master", "", 200))
	exec("INSERT INTO actions(id,provider_id,code,name,risk_level,visibility_granting,grantable_to_applications,created_at,updated_at) VALUES('grant-management.admin.users.read','grant-management','admin.users.read','Read users','high',0,1,'now','now')")
	exec("INSERT INTO role_actions(role_id,action_id) VALUES('role-mgmt','grant-management.admin.users.read')")
	directoryRequest(t, s, "", "bearer", "mgmt-org", 403)
	exec("INSERT INTO application_actions(application_id,action_id) VALUES('app-1','grant-management.admin.users.read')")
	directoryRequest(t, s, "", "bearer", "mgmt-org", 403)
	exec("INSERT INTO application_grant_actions(application_grant_id,action_id) VALUES('g1','grant-management.admin.users.read')")
	decodeDirectory(t, directoryRequest(t, s, "", "bearer", "mgmt-org", 200))
	exec("DELETE FROM role_actions WHERE role_id='role-mgmt' AND action_id='grant-management.admin.users.read'")
	directoryRequest(t, s, "", "bearer", "mgmt-org", 403)
	r := httptest.NewRequest("GET", "/api/v1/admin/users", nil)
	r.Header.Set("X-Grant-Master-Key", "wrong")
	r.Header.Set("Authorization", "Bearer tok")
	r.Header.Set("X-Organization-ID", "mgmt-org")
	w := httptest.NewRecorder()
	s.ServeHTTP(w, r)
	if w.Code != 401 {
		t.Fatalf("invalid master downgraded: %d %s", w.Code, w.Body.String())
	}
	s.learner = true
	directoryRequest(t, s, "", "master", "", 503)
}

func TestAdminUsersDirectory(t *testing.T) {
	s, store := directoryTestServer(t)
	ctx := context.Background()
	users := []grant.User{
		{ID: "dir-a", DisplayName: "100% literal", Active: true},
		{ID: "dir-b", DisplayName: "under_score", Active: false},
		{ID: "dir-c", DisplayName: `back\slash`, Active: true},
		{ID: "dir-d", Email: stringPointer("needle@example.test"), DisplayName: "Ordinary", Active: true},
		{ID: "dir-e", DisplayName: "100X literal underXscore backXslash", Active: true},
		{ID: "dir-f", DisplayName: "' OR 1=1 --", Active: true},
	}
	// Insert out of ID order to prove pagination does not use insertion order.
	for i := len(users) - 1; i >= 0; i-- {
		u := users[i]
		if _, err := store.Exec(ctx, "INSERT INTO users(id,email,display_name,active,created_at) VALUES(?,?,?,?,?)", u.ID, u.Email, u.DisplayName, u.Active, "now"); err != nil {
			t.Fatal(err)
		}
	}
	for _, tc := range []struct {
		q   string
		ids []string
	}{
		{"%", []string{"dir-a"}}, {"_", []string{"dir-b"}}, {`\`, []string{"dir-c"}},
		{"needle@", []string{"dir-d"}}, {"dir-c", []string{"dir-c"}},
		{"100%", []string{"dir-a"}}, {"' OR 1=1 --", []string{"dir-f"}},
		{"'; DROP TABLE users; --", []string{}}, {"not-found", []string{}},
	} {
		t.Run(tc.q, func(t *testing.T) {
			page := decodeDirectory(t, directoryRequest(t, s, "?q="+url.QueryEscape(tc.q), "master", "", 200))
			ids := make([]string, 0, len(page.Items))
			for _, u := range page.Items {
				ids = append(ids, u.ID)
			}
			if !reflect.DeepEqual(ids, tc.ids) {
				t.Fatalf("got %v, want %v", ids, tc.ids)
			}
		})
	}
	first := decodeDirectory(t, directoryRequest(t, s, "?q=dir-&limit=2", "master", "", 200))
	if !reflect.DeepEqual(first.Items, users[:2]) || first.NextCursor != "dir-b" {
		t.Fatalf("first page: %+v", first)
	}
	second := decodeDirectory(t, directoryRequest(t, s, "?q=dir-&limit=2&cursor="+first.NextCursor, "master", "", 200))
	if !reflect.DeepEqual(second.Items, users[2:4]) || second.NextCursor != "dir-d" {
		t.Fatalf("second page: %+v", second)
	}
	last := decodeDirectory(t, directoryRequest(t, s, "?q=dir-&limit=2&cursor="+second.NextCursor, "master", "", 200))
	if !reflect.DeepEqual(last.Items, users[4:]) || last.NextCursor != "" {
		t.Fatalf("last page: %+v", last)
	}
	empty := decodeDirectory(t, directoryRequest(t, s, "?q=dir-&cursor=zzzz", "master", "", 200))
	if len(empty.Items) != 0 {
		t.Fatalf("past-end cursor: %+v", empty)
	}
	afterMissing := decodeDirectory(t, directoryRequest(t, s, "?q=dir-&cursor=dir-bb", "master", "", 200))
	if !reflect.DeepEqual(afterMissing.Items, users[2:]) {
		t.Fatalf("cursor should be a strict ID bound: %+v", afterMissing)
	}
	injectedCursor := decodeDirectory(t, directoryRequest(t, s, "?q=dir-&cursor="+url.QueryEscape("zz' OR 1=1 --"), "master", "", 200))
	if len(injectedCursor.Items) != 0 {
		t.Fatalf("cursor injection: %+v", injectedCursor)
	}
	if _, err := store.Exec(ctx, "INSERT INTO user_identities(id,user_id,provider,external_user_id,created_at,updated_at) VALUES('dir-identity','dir-a','test','private-external-subject','now','now')"); err != nil {
		t.Fatal(err)
	}
	w := directoryRequest(t, s, "?q=dir-", "master", "", 200)
	var wire struct {
		Items []map[string]json.RawMessage `json:"items"`
	}
	if err := json.Unmarshal(w.Body.Bytes(), &wire); err != nil {
		t.Fatal(err)
	}
	for i, fields := range wire.Items {
		want := 3
		if users[i].Email != nil {
			want = 4
		}
		if len(fields) != want || fields["id"] == nil || fields["displayName"] == nil || fields["active"] == nil {
			t.Fatalf("unexpected public fields: %v", fields)
		}
		for name := range fields {
			if name != "id" && name != "email" && name != "displayName" && name != "active" {
				t.Fatalf("leaked field %s", name)
			}
		}
	}
	if strings.Contains(w.Body.String(), "private-external-subject") {
		t.Fatal("external subject leaked")
	}
	for i := 0; i < 105; i++ {
		id := fmt.Sprintf("page-%03d", i)
		if _, err := store.Exec(ctx, "INSERT INTO users(id,display_name,active,created_at) VALUES(?,?,1,'now')", id, id); err != nil {
			t.Fatal(err)
		}
	}
	for _, tc := range []struct {
		query  string
		count  int
		cursor string
	}{
		{"?q=page-", 50, "page-049"}, {"?q=page-&limit=1", 1, "page-000"},
		{"?q=page-&limit=100", 100, "page-099"}, {"?q=page-&limit=101", 100, "page-099"},
		{"?q=page-&limit=999&cursor=page-099", 5, ""},
	} {
		page := decodeDirectory(t, directoryRequest(t, s, tc.query, "master", "", 200))
		if len(page.Items) != tc.count || page.NextCursor != tc.cursor {
			t.Fatalf("%s: count=%d cursor=%s", tc.query, len(page.Items), page.NextCursor)
		}
	}
}

func stringPointer(s string) *string { return &s }

func TestAdminUsersInvalidQueryContract(t *testing.T) {
	s := testServer()
	for _, query := range []string{
		"?unknown=1", "?q=x&q=y", "?q=x&%71=y", "?cursor=x&cursor=y", "?limit=1&limit=2", "?consistency=strong&consistency=strong",
		"?q=%zz", "?q=x;limit=1", "?bad%zz=x", "?q=%ff", "?cursor=%ff", "?q=%00", "?cursor=%0A", "?q=%7F",
		"?limit=", "?limit=0", "?limit=-1", "?limit=%2B1", "?limit=1.5", "?limit=abc", "?limit=99999999999999999999",
		"?limit=" + url.QueryEscape("1 OR 1=1"), "?limit=" + strings.Repeat("1", 21),
		"?q=" + strings.Repeat("x", 1025), "?cursor=" + strings.Repeat("x", 1025), "?consistency=local",
		"?consistency=" + url.QueryEscape(strings.Repeat(" ", 33)+"strong"),
	} {
		t.Run(query, func(t *testing.T) { directoryRequest(t, s, query, "master", "", 400) })
	}
	r := httptest.NewRequest("GET", "/api/v1/admin/users", strings.NewReader(`{}`))
	r.Header.Set("X-Grant-Master-Key", "master-secret")
	w := httptest.NewRecorder()
	s.ServeHTTP(w, r)
	if w.Code != 400 {
		t.Fatalf("body accepted: %d %s", w.Code, w.Body.String())
	}
}

func TestAdminUsersValidQueryContract(t *testing.T) {
	s, _ := directoryTestServer(t)
	for _, query := range []string{"?q=&cursor=", "?consistency=%20Strong%20", "?q=" + strings.Repeat("x", 1024), "?cursor=" + strings.Repeat("x", 1024)} {
		directoryRequest(t, s, query, "master", "", 200)
	}
}
