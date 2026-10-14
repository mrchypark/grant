package adminauth

import (
	"net/http"
	"net/http/httptest"
	"net/url"
	"strings"
	"testing"
	"time"
)

func TestLogoutEndsLocallyAndConfirmationDoesNotLogout(t *testing.T) {
	f := newIssuerFixture(t)
	var csrf string
	h := f.handler(t, http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) { csrf = CSRFToken(r.Context()); w.WriteHeader(200) }))
	login, transaction := beginLogin(t, h, f)
	cookie := completeLogin(t, h, f, login.Query().Get("state"), f.server.URL, transaction)
	request := func(method, path, token, origin string, c *http.Cookie) *httptest.ResponseRecorder {
		r := httptest.NewRequest(method, path, strings.NewReader(url.Values{"csrf_token": {token}}.Encode()))
		r.Header.Set("Content-Type", "application/x-www-form-urlencoded")
		if origin != "" {
			r.Header.Set("Origin", origin)
		}
		if c != nil {
			r.AddCookie(c)
		}
		w := httptest.NewRecorder()
		h.ServeHTTP(w, r)
		return w
	}
	if w := request("GET", "/", "", "", cookie); w.Code != 200 {
		t.Fatal(w.Code)
	} else {
		assertSecurityHeaders(t, w)
	}
	if w := request("GET", "/logged-out", "", "", cookie); w.Code != 303 || w.Header().Get("Location") != "/" || len(w.Result().Cookies()) != 0 {
		t.Fatal("confirmation changed live session")
	}
	if w := request("GET", "/", "", "", cookie); w.Code != 200 {
		t.Fatal("session lost")
	}
	for _, tc := range []struct{ token, origin string }{{"wrong", testPublicURL}, {"", testPublicURL}, {csrf, "https://attacker.example"}, {csrf, "null"}} {
		if w := request("POST", "/logout", tc.token, tc.origin, cookie); w.Code != 403 {
			t.Fatalf("invalid logout accepted: %d", w.Code)
		}
		if w := request("GET", "/", "", "", cookie); w.Code != 200 {
			t.Fatal("rejected logout invalidated session")
		}
	}
	w := request("POST", "/logout", csrf, testPublicURL, cookie)
	if w.Code != 303 || w.Header().Get("Location") != "/logged-out" {
		t.Fatalf("logout %d %q", w.Code, w.Header().Get("Location"))
	}
	cleared := false
	for _, c := range w.Result().Cookies() {
		if c.Name == sessionCookie && c.MaxAge < 0 {
			cleared = true
		}
	}
	if !cleared {
		t.Fatal("cookie not cleared")
	}
	if w := request("GET", "/", "", "", cookie); w.Code != 303 || w.Header().Get("Location") != "/login" {
		t.Fatal("old session valid")
	}
	for _, c := range []*http.Cookie{nil, cookie} {
		if w := request("POST", "/logout", "", "", c); w.Code != 303 || w.Header().Get("Location") != "/logged-out" {
			t.Fatal("missing session enters SSO")
		}
	}
	w = request("GET", "/logged-out", "", "", nil)
	if w.Code != 200 || !strings.Contains(w.Body.String(), `href="/login"`) || len(w.Result().Cookies()) != 0 {
		t.Fatal("invalid public confirmation")
	}
	for _, text := range []string{`lang="ko"`, "Grant에서 로그아웃했습니다", "Grant 관리 세션이 종료되었습니다. Goauthy 로그인 상태는 변경하지 않았습니다.", "다시 로그인"} {
		if !strings.Contains(w.Body.String(), text) {
			t.Errorf("confirmation missing Korean content %q", text)
		}
	}
	assertSecurityHeaders(t, w)
	if w := request("POST", "/logged-out", "", "", nil); w.Code != 405 {
		t.Fatal("confirmation accepts POST")
	}
}

func TestExpiredLogoutEndsLocally(t *testing.T) {
	s := &server{public: &url.URL{Scheme: "https"}, sessions: map[string]session{"expired": {expires: time.Now().Add(-time.Minute)}}, logins: map[string]login{}}
	next := http.HandlerFunc(func(http.ResponseWriter, *http.Request) { t.Fatal("protected handler reached") })
	r := httptest.NewRequest("POST", "/logout", nil)
	r.AddCookie(&http.Cookie{Name: sessionCookie, Value: "expired"})
	w := httptest.NewRecorder()
	s.serve(next, w, r)
	if w.Code != 303 || w.Header().Get("Location") != "/logged-out" {
		t.Fatalf("expired logout: %d %q", w.Code, w.Header().Get("Location"))
	}
}
