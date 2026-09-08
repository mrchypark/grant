package grant

import "sort"

func patchDataBatchCapability(caps []HttpCapability) (HttpCapability, Action, error) {
	policy := &HttpBatchPolicy{Encoding: "request-array", RequestsPointer: "/requests", MethodField: "method", URLField: "url", BodyField: "body", HeadersField: "headers", MaxOperations: 20, Multipart: &HttpBatchMultipart{PayloadField: "@jsonPayload", FileFieldPatterns: []string{"requests.{index}.{field}", "requests[{index}].{field}"}}}
	byPath := map[string]HttpCapability{}
	for _, cap := range caps {
		if cap.Method == "PATCH" {
			byPath[cap.PathTemplate] = cap
		}
		if cap.Method == "POST" || cap.Method == "PATCH" || cap.Method == "DELETE" {
			policy.AllowedCapabilityIDs = append(policy.AllowedCapabilityIDs, cap.CapabilityID)
		}
	}
	for _, cap := range caps {
		if cap.Method != "POST" {
			continue
		}
		update, ok := byPath[cap.PathTemplate+"/{id}"]
		if ok {
			policy.Upserts = append(policy.Upserts, HttpBatchUpsert{Method: "PUT", PathTemplate: cap.PathTemplate, CreateCapabilityID: cap.CapabilityID, UpdateCapabilityID: update.CapabilityID, IDPointer: "/id"})
		}
	}
	sort.Strings(policy.AllowedCapabilityIDs)
	sort.Slice(policy.Upserts, func(i, j int) bool { return policy.Upserts[i].PathTemplate < policy.Upserts[j].PathTemplate })
	action := Action{ID: "patch-data.batch.execute", ProviderID: "patch-data", Code: "batch.execute", Name: "Execute an authorized native batch", RiskLevel: "high", GrantableToApplications: true}
	cap := HttpCapability{CapabilityID: "patch-data.batch", Version: 1, Enabled: true, AccessMode: "authorized", Method: "POST", PathTemplate: "/api/batch", ActionID: &action.ID, Resource: &HttpCapabilityResource{ResourceType: "batch", ID: HttpResourceExtractor{Kind: "constant", Value: "patch-data"}}, Query: HttpQueryPolicy{Mode: "none"}, Request: HttpRequestPolicy{Body: "opaque", MaxBytes: 128 << 20, Authorization: "service-token", ForwardHeaders: []string{"content-type"}, ContentTypes: []string{"application/json", "multipart/form-data"}}, Response: HttpResponsePolicy{MaxBytes: 128 << 20, ForwardHeaders: []string{"content-type"}, Redirects: "deny", ErrorStatuses: []int{400, 403, 404, 409, 422}}, Batch: policy}
	var err error
	cap.ContractDigest, err = HttpCapabilityContractDigest(cap)
	return cap, action, err
}
