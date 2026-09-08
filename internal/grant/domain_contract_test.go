package grant

import (
	"encoding/json"
	"strings"
	"testing"
)

func TestModelWireCompatibility(t *testing.T) {
	// Explicit JSON expectations follow crates/domain/tests/model_roundtrip.rs and API DTO contracts.
	cases := []struct {
		name  string
		value any
		want  string
	}{
		{"organization", Organization{ID: "org", Name: "Org"}, `{"id":"org","name":"Org"}`},
		{"user", User{ID: "u", DisplayName: "User", Active: true}, `{"id":"u","displayName":"User","active":true}`},
		{"membership", OrganizationMembership{ID: "m", OrganizationID: "o", UserID: "u", MembershipRole: "member", Status: "disabled"}, `{"id":"m","organizationId":"o","userId":"u","membershipRole":"member","status":"disabled"}`},
		{"current user nullable organization", CurrentUserView{Provider: "rauthy", ExternalUserID: "r", User: User{ID: "u"}, Organizations: []Organization{}}, `{"provider":"rauthy","externalUserId":"r","user":{"id":"u","displayName":"","active":false},"organizations":[],"activeOrganizationId":null}`},
	}
	for _, c := range cases {
		t.Run(c.name, func(t *testing.T) {
			b, err := json.Marshal(c.value)
			if err != nil || string(b) != c.want {
				t.Fatalf("got %s err=%v want=%s", b, err, c.want)
			}
		})
	}
}
func TestCanonicalRequestEvidenceRustNegativeCases(t *testing.T) {
	// Source: frozen crates/infra/src/service.rs canonical_path_and_query_extractors_are_exact_and_unambiguous.
	template := "/api/collections/plants/records/{plantId}"
	for _, path := range []string{"/api/collections/plants/records/%61bcdefghijklmn1", "/api/collections/plants//records/abcdefghijklmn1", "/api/collections/plants/records/..", "/api/collections/other/records/abcdefghijklmn1"} {
		if _, err := matchPath(template, path); err == nil {
			t.Errorf("accepted %s", path)
		}
	}
	captures, err := matchPath(template, "/api/collections/plants/records/abcdefghijklmn1")
	if err != nil || captures["plantId"] != "abcdefghijklmn1" {
		t.Fatal(captures, err)
	}
	policy := HttpQueryPolicy{Mode: "exact", Parameters: []HttpQueryParameter{{Name: "optional", Format: "slug"}, {Name: "plant", Format: "pocketbaseId", Required: true}}}
	if _, err = canonicalQuery(policy, "optional=site-a&plant=abcdefghijklmn1"); err != nil {
		t.Fatal(err)
	}
	for _, q := range []string{"plant=abcdefghijklmn1&unknown=x", "plant=abcdefghijklmn1&plant=abcdefghijklmn1", "plant=abcdefghijklmn1&optional=site-a", "plant=%61bcdefghijklmn1", "optional=site-a"} {
		if _, err := canonicalQuery(policy, q); err == nil {
			t.Errorf("accepted %s", q)
		}
	}
}
func TestPrincipalDigestLengthFraming(t *testing.T) {
	a := ExposurePrincipalDigest("ab", "c", "d")
	b := ExposurePrincipalDigest("a", "bc", "d")
	if a == b || len(a) != 64 || strings.Trim(a, "0123456789abcdef") != "" {
		t.Fatal(a, b)
	}
}
func TestRedirectSecurityContract(t *testing.T) {
	for _, uri := range []string{"http://localhost:8080/callback", "http://127.0.0.1/callback", "http://127.0.0.1:80/callback", "https://example.test/callback?x=1", "https://u:p@example.test/callback", "https://example.test/callback#", "https://example.test/*", "javascript:alert(1)"} {
		if _, err := CanonicalRedirectURI(uri); err == nil {
			t.Errorf("accepted %s", uri)
		}
	}
	for _, uri := range []string{"https://example.test/callback", "http://127.0.0.1:8080/callback", "http://[::1]:8080/callback"} {
		if _, err := CanonicalRedirectURI(uri); err != nil {
			t.Errorf("rejected %s: %v", uri, err)
		}
	}
}
