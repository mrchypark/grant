package grant

import (
	"crypto/sha256"
	"fmt"
	"strings"
)

func sha256Text(v string) string { return fmt.Sprintf("sha256:%x", sha256.Sum256([]byte(v))) }
func validateSHA256(field, v string) error {
	if len(v) != 71 || !strings.HasPrefix(v, "sha256:") || strings.Trim(v[7:], "0123456789abcdef") != "" {
		return serviceError(400, field+" must be a sha256 digest")
	}
	return nil
}
func exactValue(field, v string, max int) error {
	if v == "" || v != strings.TrimSpace(v) || len(v) > max {
		return serviceError(400, field+" must be a bounded exact non-empty value")
	}
	return nil
}
func scalarFormat(format, v string) error {
	valid := false
	alpha := "abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789"
	switch format {
	case "pocketbaseId":
		valid = len(v) == 15 && strings.Trim(v, alpha) == ""
	case "uuid":
		x := strings.TrimPrefix(v, "urn:uuid:")
		if len(x) == 38 && x[0] == '{' && x[37] == '}' {
			x = x[1:37]
		}
		if len(x) == 36 && x[8] == '-' && x[13] == '-' && x[18] == '-' && x[23] == '-' {
			x = strings.ReplaceAll(x, "-", "")
		}
		valid = len(x) == 32 && strings.Trim(x, "0123456789abcdefABCDEF") == ""
	case "xid":
		valid = len(v) == 20 && strings.Trim(v, "abcdefghijklmnopqrstuvwxyz0123456789") == ""
	case "slug":
		valid = len(v) > 0 && len(v) <= 128 && strings.Trim(v, "abcdefghijklmnopqrstuvwxyz0123456789-") == ""
	case "asciiToken":
		valid = len(v) > 0 && len(v) <= 256 && strings.Trim(v, alpha+".-_:") == ""
	}
	if !valid {
		return serviceError(400, "resource scalar does not match capability format")
	}
	return nil
}
func canonicalQuery(policy HttpQueryPolicy, query string) (map[string]string, error) {
	values := map[string]string{}
	if policy.Mode == "none" {
		if query != "" {
			return nil, serviceError(403, "forbidden: query is not allowed")
		}
		return values, nil
	}
	prior := ""
	if query != "" {
		for _, pair := range strings.Split(query, "&") {
			key, v, ok := strings.Cut(pair, "=")
			if !ok {
				return nil, serviceError(400, "canonicalQuery contains an invalid pair")
			}
			if key == "" || strings.ContainsAny(pair, "%;") {
				return nil, serviceError(400, "canonicalQuery is not canonical")
			}
			if prior != "" && prior >= key {
				return nil, serviceError(400, "canonicalQuery keys must be unique and sorted")
			}
			prior = key
			found := false
			for _, p := range policy.Parameters {
				if p.Name == key {
					found = true
					if err := scalarFormat(p.Format, v); err != nil {
						return nil, err
					}
					break
				}
			}
			if !found {
				return nil, serviceError(403, "forbidden: query key is not allowed")
			}
			values[key] = v
		}
	}
	for _, p := range policy.Parameters {
		if _, ok := values[p.Name]; p.Required && !ok {
			return nil, serviceError(403, "forbidden: required query key is missing")
		}
	}
	return values, nil
}
func matchPath(template, path string) (map[string]string, error) {
	if !strings.HasPrefix(path, "/") || strings.ContainsAny(path, "%\\") || strings.Contains(path, "//") {
		return nil, serviceError(400, "canonicalPath is not canonical")
	}
	for _, b := range []byte(path) {
		if b < 32 || b == 127 {
			return nil, serviceError(400, "canonicalPath is not canonical")
		}
	}
	want, got := strings.Split(template, "/"), strings.Split(path, "/")
	if len(want) != len(got) {
		return nil, serviceError(403, "forbidden: HTTP path does not match capability")
	}
	out := map[string]string{}
	for i, w := range want {
		v := got[i]
		if strings.HasPrefix(w, "{") && strings.HasSuffix(w, "}") {
			if v == "" || v == "." || v == ".." {
				return nil, serviceError(400, "canonicalPath contains an invalid segment")
			}
			if err := scalarFormat("asciiToken", v); err != nil {
				return nil, err
			}
			name := w[1 : len(w)-1]
			if _, duplicate := out[name]; duplicate {
				return nil, serviceError(400, "duplicate path variable")
			}
			out[name] = v
		} else if w != v {
			return nil, serviceError(403, "forbidden: HTTP path does not match capability")
		}
	}
	return out, nil
}
func validateHTTPRequestEvidence(c HttpCapability, r HttpCapabilityDecisionInput) error {
	if r.ContractDigest != c.ContractDigest {
		return serviceError(403, "forbidden: HTTP capability contract digest mismatch")
	}
	if r.Method != c.Method {
		return serviceError(403, "forbidden: HTTP method mismatch")
	}
	for k, v := range map[string]string{"canonicalQueryDigest": r.CanonicalQueryDigest, "forwardedHeaderDigest": r.ForwardedHeaderDigest, "bodyDigest": r.BodyDigest, "providerCredentialDigest": r.ProviderCredentialDigest} {
		if err := validateSHA256(k, v); err != nil {
			return err
		}
	}
	if sha256Text(r.CanonicalQuery) != r.CanonicalQueryDigest {
		return serviceError(403, "forbidden: canonical query digest mismatch")
	}
	query, err := canonicalQuery(c.Query, r.CanonicalQuery)
	if err != nil {
		return err
	}
	if c.Request.Body == "none" && (r.BodyLength != 0 || r.BodyDigest != sha256Text("")) {
		return serviceError(403, "forbidden: bodyless request policy mismatch")
	}
	if c.Request.Body == "json" && r.BodyLength > c.Request.MaxBytes {
		return serviceError(403, "forbidden: request body exceeds capability limit")
	}
	if len(c.Request.ForwardHeaders) == 0 && r.ForwardedHeaderDigest != sha256Text("") {
		return serviceError(403, "forbidden: forwarded header policy mismatch")
	}
	if err = exactValue("requestBinding", r.RequestBinding, 256); err != nil {
		return err
	}
	if len(r.RequestBinding) < 22 || strings.Trim(r.RequestBinding, "abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789-_") != "" {
		return serviceError(400, "requestBinding must contain at least 128 bits of base64url entropy")
	}
	if err = exactValue("providerCredential.subject", r.ProviderCredential.Subject, 512); err != nil {
		return err
	}
	if err = exactValue("providerCredential.audience", r.ProviderCredential.Audience, 512); err != nil {
		return err
	}
	if c.Resource == nil {
		return serviceError(503, "capability resource missing")
	}
	if r.ResourceType != c.Resource.ResourceType {
		return serviceError(403, "forbidden: resource type mismatch")
	}
	captures, err := matchPath(c.PathTemplate, r.CanonicalPath)
	if err != nil {
		return err
	}
	extractor := c.Resource.ID
	value := ""
	name := ptrValue(r.ResourceEvidence.Name)
	if r.ResourceEvidence.Kind != extractor.Kind {
		return serviceError(403, "forbidden: resource evidence mismatch")
	}
	switch extractor.Kind {
	case "pathSegment":
		if name != extractor.Name {
			return serviceError(403, "forbidden: path resource evidence mismatch")
		}
		var ok bool
		value, ok = captures[name]
		if !ok {
			return serviceError(403, "forbidden: path resource was not captured")
		}
	case "queryScalar":
		if name != extractor.Name {
			return serviceError(403, "forbidden: query resource evidence mismatch")
		}
		var ok bool
		value, ok = query[name]
		if !ok {
			return serviceError(403, "forbidden: required query resource is missing")
		}
	case "jsonPointer":
		if name != extractor.Pointer {
			return serviceError(403, "forbidden: body resource evidence mismatch")
		}
		value = r.ResourceEvidence.CanonicalValue
	case "constant":
		if r.ResourceEvidence.Name != nil {
			return serviceError(403, "forbidden: constant resource evidence mismatch")
		}
		value = extractor.Value
	default:
		return serviceError(503, "capability resource extractor invalid")
	}
	if extractor.Kind != "constant" {
		if err = scalarFormat(extractor.Format, value); err != nil {
			return err
		}
	}
	if value != r.ResourceEvidence.CanonicalValue || value != r.ResourceID {
		return serviceError(403, "forbidden: resource evidence mismatch")
	}
	return nil
}
