package grant

import (
	"bytes"
	"encoding/json"
	"fmt"
	"strings"
)

func (h HttpQueryPolicy) MarshalJSON() ([]byte, error) {
	if h.Mode == "none" {
		return []byte(`{"mode":"none"}`), nil
	}
	return json.Marshal(struct {
		Mode       string               `json:"mode"`
		Parameters []HttpQueryParameter `json:"parameters"`
	}{h.Mode, h.Parameters})
}
func (h *HttpQueryPolicy) UnmarshalJSON(b []byte) error {
	var obj map[string]json.RawMessage
	if e := json.Unmarshal(b, &obj); e != nil {
		return e
	}
	if e := json.Unmarshal(obj["mode"], &h.Mode); e != nil {
		return e
	}
	if h.Mode == "none" {
		if len(obj) != 1 {
			return fmt.Errorf("none query policy contains unknown fields")
		}
		h.Parameters = nil
		return nil
	}
	if h.Mode != "exact" || len(obj) != 2 || obj["parameters"] == nil {
		return fmt.Errorf("invalid query policy")
	}
	d := json.NewDecoder(bytes.NewReader(obj["parameters"]))
	d.DisallowUnknownFields()
	return d.Decode(&h.Parameters)
}
func (h HttpResourceExtractor) MarshalJSON() ([]byte, error) {
	switch h.Kind {
	case "pathSegment", "queryScalar":
		return json.Marshal(struct {
			Kind   string `json:"kind"`
			Name   string `json:"name"`
			Format string `json:"format"`
		}{h.Kind, h.Name, h.Format})
	case "jsonPointer":
		return json.Marshal(struct {
			Kind    string `json:"kind"`
			Pointer string `json:"pointer"`
			Format  string `json:"format"`
		}{h.Kind, h.Pointer, h.Format})
	case "constant":
		return json.Marshal(struct {
			Kind  string `json:"kind"`
			Value string `json:"value"`
		}{h.Kind, h.Value})
	}
	return nil, fmt.Errorf("invalid extractor")
}
func (h *HttpResourceExtractor) UnmarshalJSON(b []byte) error {
	type plain HttpResourceExtractor
	var p plain
	d := json.NewDecoder(bytes.NewReader(b))
	d.DisallowUnknownFields()
	if e := d.Decode(&p); e != nil {
		return e
	}
	var m map[string]json.RawMessage
	json.Unmarshal(b, &m)
	switch p.Kind {
	case "pathSegment", "queryScalar":
		if len(m) != 3 || m["name"] == nil || m["format"] == nil {
			return fmt.Errorf("invalid scalar extractor")
		}
	case "jsonPointer":
		if len(m) != 3 || m["pointer"] == nil || m["format"] == nil {
			return fmt.Errorf("invalid pointer extractor")
		}
	case "constant":
		if len(m) != 2 || m["value"] == nil {
			return fmt.Errorf("invalid constant extractor")
		}
	default:
		return fmt.Errorf("invalid extractor kind")
	}
	*h = HttpResourceExtractor(p)
	return nil
}
func HttpCapabilityContractDigest(c HttpCapability) (string, error) {
	v, e := canonicalJSON(c, "contractDigest")
	if e != nil {
		return "", e
	}
	return digestBytes([]byte(v)), nil
}

type canonicalGeneration struct {
	SchemaVersion uint64           `json:"schemaVersion"`
	ProviderID    string           `json:"providerId"`
	Generation    uint64           `json:"generation"`
	Capabilities  []HttpCapability `json:"capabilities"`
}

func generationJSON(provider string, c StageHttpCapabilityGeneration) (string, error) {
	var b bytes.Buffer
	enc := json.NewEncoder(&b)
	enc.SetEscapeHTML(false)
	e := enc.Encode(canonicalGeneration{c.SchemaVersion, provider, c.Generation, c.Capabilities})
	return strings.TrimSuffix(b.String(), "\n"), e
}
func HttpCapabilityCatalogDigest(provider string, c StageHttpCapabilityGeneration) (string, error) {
	b, e := generationJSON(provider, c)
	return digestBytes([]byte(b)), e
}
func capabilityToken(s string) bool {
	if s == "" || len(s) > 256 {
		return false
	}
	for _, c := range s {
		if !((c >= 'a' && c <= 'z') || (c >= 'A' && c <= 'Z') || (c >= '0' && c <= '9') || strings.ContainsRune(".-_:/", c)) {
			return false
		}
	}
	return true
}
func capabilityFormat(s string) bool {
	switch s {
	case "pocketbaseId", "uuid", "xid", "slug", "asciiToken":
		return true
	}
	return false
}
func pathVariables(s string) ([]string, error) {
	if !strings.HasPrefix(s, "/") || strings.ContainsAny(s, "\\\r\n\t") {
		return nil, fmt.Errorf("invalid pathTemplate")
	}
	var v []string
	for _, part := range strings.Split(s, "/")[1:] {
		if part == "" || part == "." || part == ".." {
			return nil, fmt.Errorf("invalid path segment")
		}
		if strings.ContainsAny(part, "{}") {
			if !strings.HasPrefix(part, "{") || !strings.HasSuffix(part, "}") || !capabilityToken(part[1:len(part)-1]) {
				return nil, fmt.Errorf("invalid variable")
			}
			v = append(v, part[1:len(part)-1])
		}
	}
	return v, nil
}
func validateCapability(c HttpCapability) error {
	if !capabilityToken(c.CapabilityID) || c.Version == 0 || c.Method == "" {
		return fmt.Errorf("invalid capability identity")
	}
	for _, r := range c.Method {
		if r < 'A' || r > 'Z' {
			return fmt.Errorf("invalid method")
		}
	}
	vars, e := pathVariables(c.PathTemplate)
	if e != nil {
		return e
	}
	if c.Query.Mode == "exact" {
		if len(c.Query.Parameters) == 0 {
			return fmt.Errorf("empty exact query policy")
		}
		prev := ""
		for _, p := range c.Query.Parameters {
			if p.Name <= prev || !capabilityToken(p.Name) || !capabilityFormat(p.Format) {
				return fmt.Errorf("invalid query parameter")
			}
			prev = p.Name
		}
	} else if c.Query.Mode != "none" || len(c.Query.Parameters) != 0 {
		return fmt.Errorf("invalid query policy")
	}
	if (c.Request.Body == "none" && c.Request.MaxBytes != 0) || (c.Request.Body == "json" && c.Request.MaxBytes == 0) || (c.Request.Body != "none" && c.Request.Body != "json") || c.Response.MaxBytes == 0 || c.Response.Redirects != "deny" {
		return fmt.Errorf("invalid body/response policy")
	}
	for _, item := range []struct {
		headers []string
		allow   string
	}{{c.Request.ForwardHeaders, "|accept|content-type|if-match|if-none-match|"}, {c.Response.ForwardHeaders, "|cache-control|content-type|etag|last-modified|"}} {
		prev := ""
		for _, h := range item.headers {
			if h <= prev || !strings.Contains(item.allow, "|"+h+"|") {
				return fmt.Errorf("invalid header policy")
			}
			prev = h
		}
	}
	switch c.AccessMode {
	case "public_authn":
		if c.ActionID != nil || c.Resource != nil || len(vars) > 0 {
			return fmt.Errorf("public_authn cannot bind action/resource")
		}
	case "authorized":
		if c.ActionID == nil || !capabilityToken(*c.ActionID) || c.Resource == nil || !capabilityToken(c.Resource.ResourceType) {
			return fmt.Errorf("authorized capability needs action/resource")
		}
		r := c.Resource.ID
		if r.Kind != "constant" && !capabilityFormat(r.Format) {
			return fmt.Errorf("invalid extractor format")
		}
		switch r.Kind {
		case "pathSegment":
			if len(vars) != 1 || vars[0] != r.Name {
				return fmt.Errorf("path extractor mismatch")
			}
		case "queryScalar":
			if len(vars) > 0 || c.Query.Mode != "exact" {
				return fmt.Errorf("query extractor mismatch")
			}
			found := false
			for _, p := range c.Query.Parameters {
				if p.Name == r.Name && p.Format == r.Format && p.Required {
					found = true
				}
			}
			if !found {
				return fmt.Errorf("query extractor missing required parameter")
			}
		case "jsonPointer":
			if len(vars) > 0 || c.Request.Body != "json" || !strings.HasPrefix(r.Pointer, "/") || strings.Contains(r.Pointer, "//") {
				return fmt.Errorf("invalid JSON extractor")
			}
		case "constant":
			if len(vars) > 0 || !capabilityToken(r.Value) {
				return fmt.Errorf("invalid constant extractor")
			}
		default:
			return fmt.Errorf("unknown extractor")
		}
	default:
		return fmt.Errorf("invalid access mode")
	}
	digest, e := HttpCapabilityContractDigest(c)
	if e != nil {
		return e
	}
	if !validDigest(c.ContractDigest) || digest != c.ContractDigest {
		return fmt.Errorf("contractDigest mismatch")
	}
	return nil
}
func validateGeneration(provider string, c StageHttpCapabilityGeneration) error {
	if !capabilityToken(provider) || c.SchemaVersion != 1 || c.Generation == 0 {
		return fmt.Errorf("invalid generation")
	}
	seen := map[string]bool{}
	for _, v := range c.Capabilities {
		if e := validateCapability(v); e != nil {
			return e
		}
		key := fmt.Sprint(v.CapabilityID, "/", v.Version)
		if seen[key] {
			return fmt.Errorf("duplicate capability")
		}
		seen[key] = true
	}
	for i, a := range c.Capabilities {
		if !a.Enabled {
			continue
		}
		for _, b := range c.Capabilities[i+1:] {
			if !b.Enabled || a.Method != b.Method {
				continue
			}
			aa, bb := strings.Split(a.PathTemplate, "/"), strings.Split(b.PathTemplate, "/")
			overlap := len(aa) == len(bb)
			if overlap {
				for i := range aa {
					if aa[i] != bb[i] && !strings.HasPrefix(aa[i], "{") && !strings.HasPrefix(bb[i], "{") {
						overlap = false
					}
				}
			}
			if overlap {
				return fmt.Errorf("overlapping enabled HTTP capabilities")
			}
		}
	}
	d, e := HttpCapabilityCatalogDigest(provider, c)
	if e != nil {
		return e
	}
	if !validDigest(c.CatalogDigest) || d != c.CatalogDigest {
		return fmt.Errorf("catalogDigest mismatch")
	}
	return nil
}
