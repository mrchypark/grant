package grant

import (
	"bytes"
	"crypto/sha256"
	"encoding/hex"
	"encoding/json"
	"fmt"
	"strings"
)

func digestBytes(b []byte) string { h := sha256.Sum256(b); return "sha256:" + hex.EncodeToString(h[:]) }
func canonicalJSON(v any, omit string) (string, error) {
	b, e := json.Marshal(v)
	if e != nil {
		return "", e
	}
	d := json.NewDecoder(bytes.NewReader(b))
	d.UseNumber()
	var data any
	if e = d.Decode(&data); e != nil {
		return "", e
	}
	if omit != "" {
		delete(data.(map[string]any), omit)
	}
	var out bytes.Buffer
	enc := json.NewEncoder(&out)
	enc.SetEscapeHTML(false)
	if e = enc.Encode(data); e != nil {
		return "", e
	}
	return jsonUTF8Separators(strings.TrimSuffix(out.String(), "\n")), nil
}

// encoding/json escapes these two JavaScript separators even with HTML escaping
// disabled. serde_json does not. Consume escape pairs so literal \\u2028 text
// remains escaped text rather than being confused with a Unicode separator.
func jsonUTF8Separators(s string) string {
	var out strings.Builder
	for i := 0; i < len(s); {
		if s[i] == '\\' && i+1 < len(s) {
			if strings.HasPrefix(s[i:], `\u2028`) || strings.HasPrefix(s[i:], `\u2029`) {
				if s[i+5] == '8' {
					out.WriteRune('\u2028')
				} else {
					out.WriteRune('\u2029')
				}
				i += 6
			} else {
				out.WriteString(s[i : i+2])
				i += 2
			}
		} else {
			out.WriteByte(s[i])
			i++
		}
	}
	return out.String()
}
func validDigest(v string) bool {
	if len(v) != 71 || !strings.HasPrefix(v, "sha256:") {
		return false
	}
	for _, r := range v[7:] {
		if !strings.ContainsRune("0123456789abcdef", r) {
			return false
		}
	}
	return true
}
func strictToken(v string, max int) error {
	if v == "" || len(v) > max || strings.TrimSpace(v) != v || strings.ContainsAny(v, "\x00\n\r\t") {
		return fmt.Errorf("invalid token")
	}
	return nil
}
