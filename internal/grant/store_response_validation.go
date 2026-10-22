package grant

import (
	"fmt"
	"strings"
)

func HttpResponseReferencesDigest(refs []HttpResponseReference) (string, error) {
	if refs == nil {
		refs = []HttpResponseReference{}
	}
	value, err := canonicalJSON(refs, "")
	if err != nil {
		return "", err
	}
	return digestBytes([]byte(value)), nil
}

func validateResponsePolicy(cap HttpCapability) error {
	p := cap.ResponseAuthorization
	if p == nil {
		return nil
	}
	if cap.AccessMode != "authorized" || cap.Method != "GET" || cap.Batch != nil || cap.Resource == nil || cap.Request.Body != "none" || cap.Query.Mode != "exact" || !capabilityToken(p.RootKey) || !fieldIdentifier(p.ExpansionField) || !fieldIdentifier(p.QueryParameter) || p.MaxDepth == 0 || p.MaxDepth > 6 || p.MaxRecords == 0 || p.MaxRecords > 200 {
		return fmt.Errorf("invalid response authorization policy")
	}
	if p.RecordsPointer != "" && (!strings.HasPrefix(p.RecordsPointer, "/") || !fieldIdentifier(strings.TrimPrefix(p.RecordsPointer, "/"))) {
		return fmt.Errorf("response recordsPointer must select a root field")
	}
	if (p.RecordsPointer == "" && cap.Resource.ID.Kind != "pathSegment") || (p.RecordsPointer != "" && cap.RecordScope == nil) {
		return fmt.Errorf("response roots require detail path or scoped list")
	}
	if cap.RecordScope != nil && (p.RecordsPointer == "" || p.QueryParameter == cap.RecordScope.FilterParameter || p.QueryParameter == cap.RecordScope.SortParameter) {
		return fmt.Errorf("ambiguous response query role")
	}
	found := false
	for _, q := range cap.Query.Parameters {
		if q.Name == p.QueryParameter {
			found = q.Format == "text" && !q.Required && !q.Repeated && q.MaxBytes > 0 && q.MaxBytes <= 4096
		}
	}
	if !found {
		return fmt.Errorf("missing response expansion query")
	}
	return nil
}

func validateResponseResources(stage StageHttpCapabilityGeneration) error {
	if len(stage.ResponseResources) > 256 {
		return fmt.Errorf("response resource graph too large")
	}
	nodes := map[string]HttpResponseResource{}
	previous := ""
	for _, node := range stage.ResponseResources {
		if node.Key <= previous || !capabilityToken(node.Key) || !capabilityToken(node.ActionID) || !capabilityToken(node.ResourceType) || !fieldIdentifier(node.IDField) || (node.IDFormat != "asciiToken" && node.IDFormat != "pocketbaseId") || !fieldIdentifier(node.IdentityField) || !capabilityToken(node.IdentityValue) {
			return fmt.Errorf("invalid response resource node")
		}
		previous = node.Key
		nodes[node.Key] = node
		field := ""
		for _, edge := range node.Relations {
			if !fieldIdentifier(edge.Field) || edge.Field <= field || !capabilityToken(edge.TargetKey) {
				return fmt.Errorf("invalid response relation edge")
			}
			field = edge.Field
		}
	}
	for _, node := range stage.ResponseResources {
		for _, edge := range node.Relations {
			if _, ok := nodes[edge.TargetKey]; !ok {
				return fmt.Errorf("response relation target missing")
			}
		}
	}
	for _, cap := range stage.Capabilities {
		p := cap.ResponseAuthorization
		if p == nil {
			continue
		}
		node, ok := nodes[p.RootKey]
		if !ok || cap.ActionID == nil || cap.Resource == nil || node.ActionID != *cap.ActionID || node.ResourceType != cap.Resource.ResourceType {
			return fmt.Errorf("response root authority mismatch")
		}
	}
	return nil
}

func responseExpansionPaths(cap HttpCapability, resources []HttpResponseResource, query string) ([]string, error) {
	p := cap.ResponseAuthorization
	if p == nil {
		return nil, nil
	}
	parameters, err := canonicalQuery(cap.Query, query)
	if err != nil {
		return nil, err
	}
	value := parameters[p.QueryParameter]
	if value == "" {
		return nil, nil
	}
	paths := strings.Split(value, ",")
	if len(paths) > int(p.MaxRecords) {
		return nil, serviceError(403, "forbidden: too many expansion paths")
	}
	nodes := map[string]HttpResponseResource{}
	for _, node := range resources {
		nodes[node.Key] = node
	}
	seen := map[string]bool{}
	for _, path := range paths {
		fields := strings.Split(path, ".")
		if uint64(len(fields)) > p.MaxDepth || seen[path] {
			return nil, serviceError(403, "forbidden: invalid expansion depth or duplicate path")
		}
		seen[path] = true
		key := p.RootKey
		for _, field := range fields {
			if !fieldIdentifier(field) {
				return nil, serviceError(403, "forbidden: invalid expansion field")
			}
			target := ""
			for _, edge := range nodes[key].Relations {
				if edge.Field == field {
					target = edge.TargetKey
					break
				}
			}
			if target == "" {
				return nil, serviceError(403, "forbidden: unmapped expansion relation")
			}
			key = target
		}
	}
	return paths, nil
}
