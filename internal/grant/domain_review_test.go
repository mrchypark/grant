package grant

import (
	"encoding/json"
	"testing"
)

func TestRedirectURLFrozenNormalization(t *testing.T) {
	// Expected output produced by frozen Cargo.lock url 2.5.8 (reqwest::Url),
	// using the exact parser underlying c716877 canonical_redirect_uri.
	for _, tc := range [][2]string{
		{"https://EXAMPLE.com:443/a/../b", "https://example.com/b"},
		{"https://example.com/a/%2E%2e/b", "https://example.com/b"},
		{"https://example.com/a/.", "https://example.com/a/"},
		{"https://example.com/a//b", "https://example.com/a//b"},
		{"https://example.com/a%2Fb", "https://example.com/a%2Fb"},
		{"https://bücher.example/callback", "https://xn--bcher-kva.example/callback"},
		{"http://127.1:8080/callback", "http://127.0.0.1:8080/callback"},
		{"http://0x7f000001:8080/", "http://127.0.0.1:8080/"},
		{" https://example.com ", "https://example.com/"},
		{"https:example.com", "https://example.com/"},
	} {
		got, err := CanonicalRedirectURI(tc[0])
		if err != nil || got != tc[1] {
			t.Fatalf("%q => %q, %v; want %q", tc[0], got, err, tc[1])
		}
	}
	for _, value := range []string{"https://example.com:65536", "http://127.0.0.1:00080"} {
		if _, err := CanonicalRedirectURI(value); err == nil {
			t.Fatal("accepted invalid redirect", value)
		}
	}
}

func TestPocketBaseRequiredIdentityNullIsUpstreamFailure(t *testing.T) {
	for _, raw := range []string{`{"id":null,"collectionId":"users_id","collectionName":"users"}`, `{"id":"u","collectionId":null,"collectionName":"users"}`, `{"id":"u","collectionId":"users_id","collectionName":null}`} {
		var record pocketBaseRecord
		if err := json.Unmarshal([]byte(raw), &record); err == nil || ErrorStatus(err) != 503 {
			t.Fatal(raw, err)
		}
	}
}
