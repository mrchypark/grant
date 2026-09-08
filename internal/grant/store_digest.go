package grant

import (
	"bytes"
	"crypto/sha256"
	"encoding/binary"
	"encoding/hex"
	"encoding/json"
	"fmt"
	"hash"
	"strings"
)

func digestBytes(b []byte) string { h := sha256.Sum256(b); return "sha256:" + hex.EncodeToString(h[:]) }
func frame(h hash.Hash, b []byte) {
	var size [8]byte
	binary.BigEndian.PutUint64(size[:], uint64(len(b)))
	h.Write(size[:])
	h.Write(b)
}
func PatchDataManifestDigest(c PatchDataAuthorityImport) string {
	h := sha256.New()
	f := func(ss ...string) {
		for _, s := range ss {
			frame(h, []byte(s))
		}
	}
	bo := func(v bool) {
		b := byte(0)
		if v {
			b = 1
		}
		frame(h, []byte{b})
	}
	f(c.ManifestVersion, c.Generation, c.LedgerDigest)
	for _, u := range c.Users {
		f("user", u.UserID, u.ExternalUserID)
		if u.Email == nil {
			h.Write([]byte{0})
		} else {
			h.Write([]byte{1})
			f(*u.Email)
		}
		f(u.DisplayName)
		bo(u.Active)
	}
	for _, o := range c.Organizations {
		f("organization", o.OrganizationID, o.Name)
		bo(o.Active)
	}
	for _, m := range c.Memberships {
		f("membership", m.MembershipID, m.OrganizationID, m.UserID, string(m.Role), string(m.Status))
	}
	for _, e := range c.HierarchyEdges {
		f("hierarchy", e.ParentOrganizationID, e.ChildOrganizationID)
	}
	for _, b := range c.PlantBindings {
		f("plant_binding", b.BindingID, string(b.SubjectType), b.SubjectID, b.SubjectOrganizationID, b.PlantID, string(b.State))
	}
	for _, a := range c.ApplicationAssignments {
		f("application_assignment", a.ApplicationID, a.OrganizationID)
	}
	return hex.EncodeToString(h.Sum(nil))
}
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
func SnapshotDigest(s AuthoritySnapshotV2) (string, error) {
	v, e := canonicalJSON(s, "snapshotDigest")
	if e != nil {
		return "", e
	}
	return digestBytes([]byte(v)), nil
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
