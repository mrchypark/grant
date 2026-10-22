package grant

import (
	"context"
	"encoding/json"
	"math"
	"strings"
)

// AdminHttpCapabilityGeneration is an immutable stored catalog, whether staged,
// active, or historical. It deliberately contains no activation or PEP metadata.
type AdminHttpCapabilityGeneration struct {
	ProviderID            string                 `json:"providerId"`
	Generation            uint64                 `json:"generation"`
	CatalogDigest         string                 `json:"catalogDigest"`
	CapabilityCount       int                    `json:"capabilityCount"`
	ResponseResourceCount int                    `json:"responseResourceCount"`
	Capabilities          []HttpCapability       `json:"capabilities"`
	ResponseResources     []HttpResponseResource `json:"responseResources"`
}

func (s *Store) HttpCapabilityGenerationAdmin(ctx context.Context, provider string, generation uint64) (AdminHttpCapabilityGeneration, error) {
	var out AdminHttpCapabilityGeneration
	if generation == 0 || generation > math.MaxInt64 {
		return out, serviceError(400, "generation must be a positive 64-bit database integer")
	}
	// Read all chunks in one strong snapshot, keeping each returned row below
	// the catalog row limit, as in ActiveHttpCapabilityCatalogAdmin.
	rows, err := s.Query(ctx, ReadStrong, `WITH RECURSIVE catalog AS (
 SELECT canonical_json,catalog_digest FROM provider_http_capability_sets
 WHERE provider_id=? AND generation=?), chunks(payload,remaining,catalog_digest,ordinal) AS (
 SELECT substr(canonical_json,1,60000),substr(canonical_json,60001),catalog_digest,0 FROM catalog
 UNION ALL SELECT substr(remaining,1,60000),substr(remaining,60001),catalog_digest,ordinal+1 FROM chunks WHERE remaining<>'')
 SELECT payload,catalog_digest FROM chunks ORDER BY ordinal`, provider, generation)
	if err != nil {
		return out, err
	}
	if len(rows) == 0 {
		return out, serviceError(404, "HTTP capability generation not found")
	}
	var canonical strings.Builder
	for _, row := range rows {
		canonical.WriteString(rowString(row, "payload"))
	}
	var stored canonicalGeneration
	if err := json.Unmarshal([]byte(canonical.String()), &stored); err != nil {
		return out, err
	}
	if stored.Capabilities == nil {
		stored.Capabilities = []HttpCapability{}
	}
	if stored.ResponseResources == nil {
		stored.ResponseResources = []HttpResponseResource{}
	}
	return AdminHttpCapabilityGeneration{
		ProviderID: provider, Generation: stored.Generation, CatalogDigest: rowString(rows[0], "catalog_digest"),
		CapabilityCount: len(stored.Capabilities), ResponseResourceCount: len(stored.ResponseResources),
		Capabilities: stored.Capabilities, ResponseResources: stored.ResponseResources,
	}, nil
}
