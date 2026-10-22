package grant

import (
	"fmt"
	"strings"
)

func HttpBatchChildrenDigest(children []HttpBatchChild) (string, error) {
	value, err := canonicalJSON(children, "")
	if err != nil {
		return "", err
	}
	return digestBytes([]byte(value)), nil
}

func validateBatchPolicy(cap HttpCapability) error {
	b := cap.Batch
	if b == nil {
		return nil
	}
	if cap.AccessMode != "authorized" || cap.Method != "POST" || cap.RecordScope != nil || cap.Resource == nil || cap.Resource.ID.Kind != "constant" || cap.Query.Mode != "none" || cap.Request.Body != "opaque" || b.Encoding != "request-array" || b.MaxOperations == 0 || b.MaxOperations > 20 || len(b.AllowedCapabilityIDs) == 0 {
		return fmt.Errorf("invalid batch policy")
	}
	if !strings.HasPrefix(b.RequestsPointer, "/") || !fieldIdentifier(strings.TrimPrefix(b.RequestsPointer, "/")) {
		return fmt.Errorf("batch requestsPointer must select one root field")
	}
	seen := map[string]bool{}
	for _, field := range []string{b.MethodField, b.URLField, b.BodyField, b.HeadersField} {
		if !fieldIdentifier(field) || seen[field] {
			return fmt.Errorf("ambiguous batch metadata field")
		}
		seen[field] = true
	}
	previous := ""
	for _, id := range b.AllowedCapabilityIDs {
		if !capabilityToken(id) || id <= previous {
			return fmt.Errorf("invalid batch capability allowlist")
		}
		previous = id
	}
	previous = ""
	for _, u := range b.Upserts {
		key := u.Method + " " + u.PathTemplate
		vars, err := pathVariables(u.PathTemplate)
		if u.Method != "PUT" || err != nil || len(vars) != 0 || key <= previous || !strings.HasPrefix(u.IDPointer, "/") || !fieldIdentifier(strings.TrimPrefix(u.IDPointer, "/")) || u.CreateCapabilityID == u.UpdateCapabilityID {
			return fmt.Errorf("invalid batch upsert recipe")
		}
		previous = key
	}
	if b.Multipart != nil {
		m := b.Multipart
		if cap.Request.Body != "opaque" || m.PayloadField == "" || len(m.PayloadField) > 64 || strings.ContainsAny(m.PayloadField, "\r\n\x00") || len(m.FileFieldPatterns) == 0 || len(m.FileFieldPatterns) > 2 {
			return fmt.Errorf("invalid batch multipart policy")
		}
		previous = ""
		for _, pattern := range m.FileFieldPatterns {
			rest := strings.ReplaceAll(strings.ReplaceAll(pattern, "{index}", ""), "{field}", "")
			if len(pattern) > 128 || pattern <= previous || strings.Count(pattern, "{index}") != 1 || strings.Count(pattern, "{field}") != 1 || !strings.HasSuffix(pattern, "{field}") || strings.ContainsAny(rest, "{}\r\n\x00/") {
				return fmt.Errorf("invalid batch file field pattern")
			}
			previous = pattern
		}
	}
	return nil
}

func validateBatchReferences(caps []HttpCapability) error {
	for _, root := range caps {
		if root.Batch == nil {
			continue
		}
		allowed := map[string]HttpCapability{}
		for _, id := range root.Batch.AllowedCapabilityIDs {
			found := false
			for _, child := range caps {
				if child.CapabilityID != id || !child.Enabled {
					continue
				}
				if found || child.Batch != nil || child.RecordScope != nil || child.AccessMode != "authorized" || child.Resource == nil || (child.Method != "POST" && child.Method != "PATCH" && child.Method != "DELETE") || child.Query.Mode != "none" || (child.Resource.ID.Kind != "constant" && child.Resource.ID.Kind != "pathSegment") {
					return fmt.Errorf("invalid batch child reference %s", id)
				}
				if child.Request.Authorization != root.Request.Authorization || (child.Method == "DELETE" && child.Request.Body != "none") || (child.Method != "DELETE" && child.Request.Body != "json" && child.Request.Body != "opaque") {
					return fmt.Errorf("incompatible batch child transport")
				}
				found = true
				allowed[id] = child
			}
			if !found {
				return fmt.Errorf("batch child reference missing %s", id)
			}
		}
		for _, u := range root.Batch.Upserts {
			create, ok := allowed[u.CreateCapabilityID]
			update, ok2 := allowed[u.UpdateCapabilityID]
			if !ok || !ok2 || create.Method != "POST" || update.Method != "PATCH" || create.PathTemplate != u.PathTemplate || create.Resource.ID.Kind != "constant" || update.Resource.ID.Kind != "pathSegment" {
				return fmt.Errorf("invalid batch upsert references")
			}
			vars, err := pathVariables(update.PathTemplate)
			if err != nil || len(vars) != 1 || update.PathTemplate != u.PathTemplate+"/{"+update.Resource.ID.Name+"}" {
				return fmt.Errorf("ambiguous batch upsert update path")
			}
		}
	}
	return nil
}
