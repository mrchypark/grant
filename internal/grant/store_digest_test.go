package grant

import "testing"

func TestCanonicalJSONUnicodeSeparators(t *testing.T) {
	// Shared backup and capability digests preserve UTF-8 separators and literal escapes.
	got, err := canonicalJSON(map[string]string{"value": "a\u2028b\u2029c\\u2028"}, "")
	want := "{\"value\":\"a\u2028b\u2029c\\\\u2028\"}"
	if err != nil || got != want {
		t.Fatalf("got %q want %q err=%v", got, want, err)
	}
}
