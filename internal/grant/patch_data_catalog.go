package grant

import (
	_ "embed"
	"encoding/json"
	"fmt"
)

//go:embed patch_data_collections.json
var patchDataCollectionManifest []byte

// PatchDataCollection defines independent, whole-record read authority. Relations
// and matching IDs in another collection never imply authority over this tuple.
type PatchDataCollection struct {
	Name          string   `json:"name"`
	ID            string   `json:"id"`
	Type          string   `json:"type"`
	ResourceType  string   `json:"resourceType"`
	AllowedFields []string `json:"allowedFields"`
}

func PatchDataCollections() ([]PatchDataCollection, error) {
	var collections []PatchDataCollection
	err := json.Unmarshal(patchDataCollectionManifest, &collections)
	return collections, err
}

// PatchDataReadCatalog produces a reviewable stage payload; it does not enroll
// applications, grant roles, or activate a generation. Every read needs an
// explicit matching action and record binding in the existing Grant evaluator.
func PatchDataReadCatalog(generation uint64, previous *uint64) (StageHttpCapabilityGeneration, []Action, error) {
	stage := StageHttpCapabilityGeneration{SchemaVersion: 1, Generation: generation, ExpectedLatestGeneration: previous}
	if generation == 0 || (previous == nil && generation != 1) || (previous != nil && (*previous == 0 || generation <= *previous || generation-*previous != 1)) {
		return stage, nil, fmt.Errorf("generation must be 1 without a previous generation, otherwise previous + 1")
	}
	collections, err := PatchDataCollections()
	if err != nil {
		return stage, nil, err
	}
	actions := make([]Action, 0, len(collections))
	for _, collection := range collections {
		action := "patch-data." + collection.ResourceType + ".read"
		actions = append(actions, Action{ID: action, ProviderID: "patch-data", Code: collection.ResourceType + ".read", Name: "Read " + collection.Name + " records", RiskLevel: "high", GrantableToApplications: true})
		for aliasIndex, alias := range []string{collection.Name, collection.ID} {
			for _, operation := range []string{"list", "get"} {
				c := HttpCapability{
					CapabilityID: fmt.Sprintf("%s.%s.%d", collection.Name, operation, aliasIndex), Version: 1, Enabled: true, AccessMode: "authorized", Method: "GET",
					PathTemplate: "/api/collections/" + alias + "/records", ActionID: &action,
					Resource: &HttpCapabilityResource{ResourceType: collection.ResourceType, ID: HttpResourceExtractor{Kind: "constant", Value: collection.Name}},
					Query:    HttpQueryPolicy{Mode: "none"},
					Request:  HttpRequestPolicy{Body: "none", ForwardHeaders: []string{"accept"}, Authorization: "service-token"},
					Response: HttpResponsePolicy{MaxBytes: 1 << 20, ForwardHeaders: []string{"content-type"}, Redirects: "deny", ErrorStatuses: []int{400, 403, 404}},
				}
				if operation == "get" {
					c.PathTemplate += "/{id}"
					c.Resource.ID = HttpResourceExtractor{Kind: "pathSegment", Name: "id", Format: "pocketbaseId"}
				} else {
					c.Query = HttpQueryPolicy{Mode: "exact", Parameters: []HttpQueryParameter{{Name: "filter", Format: "text", MaxBytes: 4096}, {Name: "page", Format: "uint"}, {Name: "perPage", Format: "uint"}, {Name: "skipTotal", Format: "bool"}, {Name: "sort", Format: "text", MaxBytes: 1024}}}
					c.RecordScope = &HttpRecordScopePolicy{FilterParameter: "filter", RowKey: "id", Grammar: "fexpr", MaxIDs: 100, MaxFilterBytes: 8192, AllowedFields: append([]string(nil), collection.AllowedFields...), SortParameter: "sort", Pagination: &HttpPaginationPolicy{PageParameter: "page", PageSizeParameter: "perPage", SkipTotalParameter: "skipTotal"}}
				}
				c.ContractDigest, err = HttpCapabilityContractDigest(c)
				if err != nil {
					return stage, nil, err
				}
				stage.Capabilities = append(stage.Capabilities, c)
			}
		}
	}
	stage.CatalogDigest, err = HttpCapabilityCatalogDigest("patch-data", stage)
	if err == nil {
		err = validateGeneration("patch-data", stage)
	}
	return stage, actions, err
}

// MergePatchDataReadActions preserves unrelated actions and the existing code of
// each action ID. Codes are unique provider keys and cannot be renamed by an
// action-set replacement without colliding with the existing primary key.
func MergePatchDataReadActions(existing, reads []Action) ([]Action, error) {
	merged := append([]Action(nil), existing...)
	seen := make(map[string]int, len(existing))
	for i, action := range existing {
		if action.ProviderID != "patch-data" {
			return nil, fmt.Errorf("existing action %q has a different provider", action.ID)
		}
		if _, ok := seen[action.ID]; ok {
			return nil, fmt.Errorf("duplicate existing action %q", action.ID)
		}
		seen[action.ID] = i
	}
	for _, read := range reads {
		if i, ok := seen[read.ID]; ok {
			// Preserve all existing action metadata and grants, not just its key.
			if merged[i].Code != read.Code && merged[i].Code != read.ID {
				return nil, fmt.Errorf("incompatible existing action code for %q", read.ID)
			}
			continue
		}
		seen[read.ID] = len(merged)
		merged = append(merged, read)
	}
	return merged, nil
}
