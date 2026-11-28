package grant

import (
	"context"
	"encoding/json"
	"fmt"
	"math"
	"regexp"
	"strings"
)

const (
	maxGRPCCapabilities = 256
	// Keep the ASCII canonical JSON and its encoded SQL argument below the
	// runtime's existing 60,000-byte staging chunk budget.
	maxGRPCCatalogBytes = 48 * 1024
	maxGRPCMessageBytes = 16 * 1024 * 1024
)

type GRPCCapability struct {
	CapabilityID     string `json:"capabilityId"`
	Version          uint64 `json:"version"`
	Enabled          bool   `json:"enabled"`
	FullMethod       string `json:"fullMethod"`
	DescriptorDigest string `json:"descriptorDigest"`
	ActionID         string `json:"actionId"`
	ResourceType     string `json:"resourceType"`
	ResourceID       string `json:"resourceId"`
	MaxMessageBytes  uint64 `json:"maxMessageBytes"`
	ContractDigest   string `json:"contractDigest"`
}

type GRPCCatalog struct {
	SchemaVersion uint64           `json:"schemaVersion"`
	ProviderID    string           `json:"providerId"`
	Generation    uint64           `json:"generation"`
	CatalogDigest string           `json:"catalogDigest"`
	Capabilities  []GRPCCapability `json:"capabilities"`
}

type StageGRPCCapabilityGeneration struct {
	SchemaVersion            uint64           `json:"schemaVersion"`
	Generation               uint64           `json:"generation"`
	ExpectedLatestGeneration *uint64          `json:"expectedLatestGeneration"`
	CatalogDigest            string           `json:"catalogDigest"`
	Capabilities             []GRPCCapability `json:"capabilities"`
}

type ActivateGRPCCapabilityGeneration struct {
	ExpectedGeneration    *uint64 `json:"expectedGeneration"`
	ExpectedCatalogDigest *string `json:"expectedCatalogDigest"`
	TargetGeneration      uint64  `json:"targetGeneration"`
	TargetCatalogDigest   string  `json:"targetCatalogDigest"`
}

var grpcFullMethod = regexp.MustCompile(`^/(?:[A-Za-z_][A-Za-z0-9_]*\.)*[A-Za-z_][A-Za-z0-9_]*/[A-Za-z_][A-Za-z0-9_]*$`)

func GRPCCapabilityContractDigest(c GRPCCapability) (string, error) {
	payload, err := canonicalJSON(c, "contractDigest")
	if err != nil {
		return "", err
	}
	return digestBytes([]byte(payload)), nil
}

func grpcCatalogJSON(c GRPCCatalog) (string, error) {
	if len(c.Capabilities) == 0 {
		c.Capabilities = []GRPCCapability{}
	}
	return canonicalJSON(c, "catalogDigest")
}

func GRPCCapabilityCatalogDigest(provider string, in StageGRPCCapabilityGeneration) (string, error) {
	payload, err := grpcCatalogJSON(GRPCCatalog{SchemaVersion: in.SchemaVersion, ProviderID: provider, Generation: in.Generation, Capabilities: in.Capabilities})
	if err != nil {
		return "", err
	}
	return digestBytes([]byte(payload)), nil
}

func grpcGenerationNumber(n uint64) bool { return n > 0 && n <= math.MaxInt64 }

func validateGRPCCatalog(c GRPCCatalog) (string, error) {
	if c.SchemaVersion != 1 || !capabilityToken(c.ProviderID) || !grpcGenerationNumber(c.Generation) ||
		!validDigest(c.CatalogDigest) || len(c.Capabilities) > maxGRPCCapabilities {
		return "", fmt.Errorf("invalid gRPC catalog")
	}
	type identity struct {
		id      string
		version uint64
	}
	seen := make(map[identity]bool, len(c.Capabilities))
	methods := make(map[string]bool, len(c.Capabilities))
	for _, v := range c.Capabilities {
		if !capabilityToken(v.CapabilityID) || !grpcGenerationNumber(v.Version) ||
			len(v.FullMethod) > 256 || !grpcFullMethod.MatchString(v.FullMethod) ||
			!validDigest(v.DescriptorDigest) || !validDigest(v.ContractDigest) ||
			!capabilityToken(v.ActionID) || !capabilityToken(v.ResourceType) || !capabilityToken(v.ResourceID) ||
			v.MaxMessageBytes == 0 || v.MaxMessageBytes > maxGRPCMessageBytes {
			return "", fmt.Errorf("invalid gRPC capability")
		}
		key := identity{v.CapabilityID, v.Version}
		if seen[key] || (v.Enabled && methods[v.FullMethod]) {
			return "", fmt.Errorf("duplicate gRPC capability identity or enabled method")
		}
		seen[key] = true
		if v.Enabled {
			methods[v.FullMethod] = true
		}
		digest, err := GRPCCapabilityContractDigest(v)
		if err != nil || digest != v.ContractDigest {
			return "", fmt.Errorf("gRPC contractDigest mismatch")
		}
	}
	payload, err := grpcCatalogJSON(c)
	if err != nil {
		return "", err
	}
	if len(payload) > maxGRPCCatalogBytes || digestBytes([]byte(payload)) != c.CatalogDigest {
		return "", fmt.Errorf("gRPC catalog exceeds size limit or catalogDigest does not match")
	}
	return payload, nil
}

func decodeGRPCCatalog(provider string, generation uint64, digest, payload string) (GRPCCatalog, error) {
	var out GRPCCatalog
	if len(payload) > maxGRPCCatalogBytes || !validDigest(digest) || digestBytes([]byte(payload)) != digest ||
		json.Unmarshal([]byte(payload), &out) != nil || out.ProviderID != provider || out.Generation != generation {
		return GRPCCatalog{}, fmt.Errorf("invalid stored gRPC catalog")
	}
	out.CatalogDigest = digest
	// Exact round trip also rejects unknown, missing, duplicate, or null
	// fields whose decoded values might otherwise match the Go zero values.
	if canonical, err := validateGRPCCatalog(out); err != nil || canonical != payload {
		return GRPCCatalog{}, fmt.Errorf("noncanonical stored gRPC catalog")
	}
	return out, nil
}

const grpcCatalogActionsExist = `NOT EXISTS (
 SELECT 1 FROM json_each(s.canonical_json,'$.capabilities') c
 WHERE NOT EXISTS (SELECT 1 FROM actions a WHERE a.id=c.value->>'actionId' AND a.provider_id=s.provider_id)
)`

const grpcRetainedActionReferences = `SELECT 1 FROM provider_grpc_capability_sets s
 JOIN json_each(s.canonical_json,'$.capabilities') ref
 WHERE s.provider_id=?`

const grpcActiveActionReferences = `SELECT 1 FROM provider_grpc_capability_active a
 JOIN provider_grpc_capability_sets s USING (provider_id,generation,catalog_digest)
 JOIN json_each(s.canonical_json,'$.capabilities') ref
 WHERE a.provider_id=?`

func (s *Store) StageGRPCCapabilityGeneration(ctx context.Context, provider string, in StageGRPCCapabilityGeneration) (GRPCCatalog, error) {
	var expected any
	if in.ExpectedLatestGeneration != nil {
		if !grpcGenerationNumber(*in.ExpectedLatestGeneration) {
			return GRPCCatalog{}, serviceError(422, "invalid expected gRPC generation")
		}
		expected = *in.ExpectedLatestGeneration
	}
	out := GRPCCatalog{in.SchemaVersion, provider, in.Generation, in.CatalogDigest, in.Capabilities}
	if len(out.Capabilities) == 0 {
		out.Capabilities = []GRPCCapability{}
	}
	payload, err := validateGRPCCatalog(out)
	if err != nil {
		return GRPCCatalog{}, serviceError(422, err.Error())
	}
	err = s.Transaction(ctx, []Statement{{SQL: `INSERT INTO provider_grpc_capability_sets(provider_id,generation,catalog_digest,canonical_json,created_at)
 SELECT s.provider_id,s.generation,s.catalog_digest,s.canonical_json,s.created_at
 FROM (SELECT ? AS provider_id,? AS generation,? AS catalog_digest,? AS canonical_json,? AS created_at) s
 CROSS JOIN (SELECT MAX(generation) AS generation FROM provider_grpc_capability_sets WHERE provider_id=?) latest
 WHERE EXISTS(SELECT 1 FROM service_providers p WHERE p.id=s.provider_id AND p.active=1)
 AND ` + grpcCatalogActionsExist + `
 AND ((latest.generation IS ? AND s.generation=COALESCE(latest.generation,0)+1)
      OR EXISTS(SELECT 1 FROM provider_grpc_capability_sets prior WHERE prior.provider_id=s.provider_id AND prior.generation=s.generation))
 ON CONFLICT(provider_id,generation) DO UPDATE SET canonical_json=excluded.canonical_json
 WHERE provider_grpc_capability_sets.catalog_digest=excluded.catalog_digest
 AND provider_grpc_capability_sets.canonical_json=excluded.canonical_json`,
		Args: []any{provider, in.Generation, in.CatalogDigest, payload, nowString(), provider, expected}, ExpectedRowsAffected: requireOne()}})
	if err != nil {
		return GRPCCatalog{}, grpcCatalogWriteError(err)
	}
	return out, nil
}

func (s *Store) ActivateGRPCCapabilityGeneration(ctx context.Context, provider string, in ActivateGRPCCapabilityGeneration) (GRPCCatalog, error) {
	if !capabilityToken(provider) || !grpcGenerationNumber(in.TargetGeneration) || !validDigest(in.TargetCatalogDigest) ||
		(in.ExpectedGeneration == nil) != (in.ExpectedCatalogDigest == nil) {
		return GRPCCatalog{}, serviceError(422, "invalid gRPC catalog activation")
	}
	var expectedGen, expectedDigest any
	if in.ExpectedGeneration != nil {
		if !grpcGenerationNumber(*in.ExpectedGeneration) || !validDigest(*in.ExpectedCatalogDigest) {
			return GRPCCatalog{}, serviceError(422, "invalid expected gRPC catalog")
		}
		expectedGen, expectedDigest = *in.ExpectedGeneration, *in.ExpectedCatalogDigest
	}
	out, err := s.readGRPCCatalog(ctx, provider, false, in.TargetGeneration, in.TargetCatalogDigest)
	if err != nil {
		return GRPCCatalog{}, err
	}
	// Recheck the live registry, provider and pointer with the mutation. Return
	// the immutable target read above, even if another activation follows ours.
	err = s.Transaction(ctx, []Statement{{SQL: `INSERT INTO provider_grpc_capability_active(provider_id,generation,catalog_digest,activated_at)
 SELECT s.provider_id,s.generation,s.catalog_digest,?
 FROM provider_grpc_capability_sets s
 JOIN service_providers p ON p.id=s.provider_id AND p.active=1
 LEFT JOIN provider_grpc_capability_active a ON a.provider_id=s.provider_id
 WHERE s.provider_id=? AND s.generation=? AND s.catalog_digest=?
 AND a.generation IS ? AND a.catalog_digest IS ?
 AND ` + grpcCatalogActionsExist + `
 ON CONFLICT(provider_id) DO UPDATE SET generation=excluded.generation,
 catalog_digest=excluded.catalog_digest,activated_at=excluded.activated_at`,
		Args: []any{nowString(), provider, in.TargetGeneration, in.TargetCatalogDigest, expectedGen, expectedDigest}, ExpectedRowsAffected: requireOne()}})
	if err != nil {
		return GRPCCatalog{}, grpcCatalogWriteError(err)
	}
	return out, nil
}

func (s *Store) ActiveGRPCCapabilityCatalog(ctx context.Context, provider string) (GRPCCatalog, error) {
	if !capabilityToken(provider) {
		return GRPCCatalog{}, serviceError(422, "invalid gRPC catalog provider")
	}
	return s.readGRPCCatalog(ctx, provider, true, 0, "")
}

// One SQL snapshot contains the pointer, manifest, provider status and every
// action ownership check. Query honors an existing pinned read fence; without
// one it requires a linearizable voter read, never a local fallback.
func (s *Store) readGRPCCatalog(ctx context.Context, provider string, active bool, generation uint64, digest string) (GRPCCatalog, error) {
	query := `SELECT s.generation,s.catalog_digest,s.canonical_json FROM provider_grpc_capability_sets s
 JOIN service_providers p ON p.id=s.provider_id AND p.active=1 `
	args := []any{provider}
	if active {
		query += `JOIN provider_grpc_capability_active a ON a.provider_id=s.provider_id
 AND a.generation=s.generation AND a.catalog_digest=s.catalog_digest WHERE s.provider_id=?`
	} else {
		query += `WHERE s.provider_id=? AND s.generation=? AND s.catalog_digest=?`
		args = append(args, generation, digest)
	}
	rows, err := s.Query(ctx, ReadStrong, query+" AND "+grpcCatalogActionsExist, args...)
	if err != nil {
		return GRPCCatalog{}, err
	}
	if len(rows) != 1 {
		if active {
			return GRPCCatalog{}, serviceError(404, "gRPC capability catalog not active")
		}
		return GRPCCatalog{}, serviceError(409, "gRPC catalog target is unavailable")
	}
	row := rows[0]
	out, err := decodeGRPCCatalog(provider, uint64(rowInt(row, "generation")), rowString(row, "catalog_digest"), rowString(row, "canonical_json"))
	if err != nil {
		return GRPCCatalog{}, serviceError(503, "invalid stored gRPC catalog")
	}
	return out, nil
}

func grpcCatalogWriteError(err error) error {
	if strings.Contains(err.Error(), "transaction rejected") {
		return serviceError(409, "gRPC catalog preconditions changed")
	}
	return err
}
