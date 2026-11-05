package grant

import (
	"context"
	"encoding/json"
	"strings"
	"time"
)

func (s *Store) StageHttpCapabilityGeneration(ctx context.Context, p string, in StageHttpCapabilityGeneration) (StagedHttpCapabilityGeneration, error) {
	out := StagedHttpCapabilityGeneration{ProviderID: p, Generation: in.Generation, CatalogDigest: in.CatalogDigest, TotalCount: uint64(len(in.Capabilities))}
	for _, c := range in.Capabilities {
		if c.Enabled {
			out.EnabledCount++
		} else {
			out.DisabledCount++
		}
	}
	if e := validateGeneration(p, in); e != nil {
		return out, serviceError(422, e.Error())
	}
	canonical, e := generationJSON(p, in)
	if e != nil {
		return out, e
	}
	old, e := s.Query(ctx, ReadStrong, "SELECT canonical_json=? AS canonical_match FROM provider_http_capability_sets WHERE provider_id=? AND generation=?", canonical, p, in.Generation)
	if e != nil {
		return out, e
	}
	if len(old) > 0 {
		if rowInt(old[0], "canonical_match") != 1 {
			return out, serviceError(409, "capability generation conflict")
		}
		return out, nil
	}
	n := int64(len(in.Capabilities))
	stageID := strings.TrimPrefix(digestBytes([]byte(canonical)), "sha256:")
	if e = s.stageText(ctx, stageID, "capabilities", canonical); e != nil {
		return out, e
	}
	ss := []Statement{{SQL: "INSERT INTO provider_http_capability_sets(provider_id,generation,catalog_digest,canonical_json,created_at) SELECT ?,?,?," + stagedTextSQL() + ",? WHERE EXISTS(SELECT 1 FROM service_providers WHERE id=? AND active=1) AND ?=COALESCE((SELECT MAX(generation) FROM provider_http_capability_sets WHERE provider_id=?),0)+1 AND ((? IS NULL AND NOT EXISTS(SELECT 1 FROM provider_http_capability_sets WHERE provider_id=?)) OR ?=(SELECT MAX(generation) FROM provider_http_capability_sets WHERE provider_id=?))", Args: []any{p, in.Generation, in.CatalogDigest, stageID, "capabilities", nowString(), p, in.Generation, p, in.ExpectedLatestGeneration, p, in.ExpectedLatestGeneration, p}, ExpectedRowsAffected: requireOne()},
		capabilityCatalogAssertion(p, in.Generation, n)}
	ss = append(ss, Statement{SQL: "DELETE FROM grant_import_payloads WHERE import_id=? AND kind='capabilities'", Args: []any{stageID}})
	if e = s.Transaction(ctx, ss); e != nil {
		return out, capabilityWriteError(e)
	}
	out.Staged = true
	return out, nil
}

// Validate persisted chunks in the publication transaction, without storing a second catalog.
func capabilityCatalogAssertion(provider string, generation uint64, count int64) Statement {
	return assertion(`SELECT 1 FROM provider_http_capability_sets s WHERE provider_id=? AND generation=?
 AND json_type(canonical_json,'$.capabilities')='array'
 AND json_array_length(canonical_json,'$.capabilities')=?
 AND NOT EXISTS(SELECT 1 FROM json_each(s.canonical_json,'$.capabilities') j WHERE
 json_type(j.value) IS NOT 'object' OR j.value->>'capabilityId' IS NULL
 OR json_type(j.value,'$.version') IS NOT 'integer' OR j.value->>'version'<=0
 OR json_type(j.value,'$.enabled') NOT IN ('true','false') OR json_type(j.value,'$.enabled') IS NULL
 OR j.value->>'accessMode' IS NULL OR j.value->>'accessMode' NOT IN ('public_authn','authorized')
 OR j.value->>'method' IS NULL OR j.value->>'pathTemplate' IS NULL OR j.value->>'contractDigest' IS NULL
 OR j.value->'query' IS NULL OR j.value->'request' IS NULL OR j.value->'response' IS NULL)
 AND NOT EXISTS(SELECT 1 FROM json_each(s.canonical_json,'$.capabilities') j
 GROUP BY j.value->>'capabilityId',j.value->>'version' HAVING COUNT(*)>1)`, provider, generation, count)
}

func (s *Store) UpsertProviderPepClient(ctx context.Context, p, id, actor string, in UpsertProviderPepClient) (ProviderPepClient, error) {
	return s.upsertProviderPepClient(ctx, p, id, actor, in, nil)
}

// A zero revision creates only; a positive revision replaces that exact policy.
// The legacy wrapper is reserved for existing internal/bootstrap callers.
func (s *Store) UpsertProviderPepClientConditional(ctx context.Context, p, id, actor string, in UpsertProviderPepClient, expectedRevision *int64) (ProviderPepClient, error) {
	if expectedRevision == nil {
		return ProviderPepClient{}, serviceError(428, "PEP policy precondition required")
	}
	if *expectedRevision < 0 {
		return ProviderPepClient{}, serviceError(400, "invalid PEP policy precondition")
	}
	return s.upsertProviderPepClient(ctx, p, id, actor, in, expectedRevision)
}

func (s *Store) upsertProviderPepClient(ctx context.Context, p, id, actor string, in UpsertProviderPepClient, expectedRevision *int64) (ProviderPepClient, error) {
	if !capabilityToken(p) || !capabilityToken(id) || (in.Lifecycle != "active" && in.Lifecycle != "disabled" && in.Lifecycle != "deleted") {
		return ProviderPepClient{}, serviceError(422, "invalid PEP client")
	}
	if in.AuthorizationMode != nil && *in.AuthorizationMode != "application" && *in.AuthorizationMode != "user" {
		return ProviderPepClient{}, serviceError(422, "invalid PEP authorization mode")
	}
	if in.ValidUntil != nil {
		t, e := time.Parse(time.RFC3339Nano, *in.ValidUntil)
		if e != nil {
			return ProviderPepClient{}, serviceError(422, "validUntil must be RFC3339")
		}
		v := t.UTC().Format("2006-01-02T15:04:05.000000000Z")
		in.ValidUntil = &v
	}
	now, receipt := nowString(), requestID()
	ss := []Statement{assertion("SELECT 1 WHERE EXISTS(SELECT 1 FROM service_providers WHERE id=? AND active=1) AND NOT EXISTS(SELECT 1 FROM provider_pep_clients WHERE pep_client_id=? AND provider_id<>?)", p, id, p),
		{SQL: `INSERT INTO provider_pep_clients(pep_client_id,application_id,provider_id,lifecycle,valid_until,created_at,updated_at,authorization_mode,policy_revision)
 VALUES(?,NULL,?,?,?,?,?,COALESCE(?,'application'),1)
 ON CONFLICT(pep_client_id) DO UPDATE SET lifecycle=excluded.lifecycle,valid_until=excluded.valid_until,updated_at=excluded.updated_at,
 authorization_mode=COALESCE(?,provider_pep_clients.authorization_mode),policy_revision=provider_pep_clients.policy_revision+1
 WHERE provider_pep_clients.provider_id=excluded.provider_id AND provider_pep_clients.policy_revision<9223372036854775807`, Args: []any{id, p, in.Lifecycle, in.ValidUntil, now, now, in.AuthorizationMode, in.AuthorizationMode}, ExpectedRowsAffected: requireOne()},
		{SQL: `INSERT INTO provider_pep_client_receipts(receipt_id,pep_client_id,application_id,provider_id,lifecycle,valid_until,changed_at,changed_by,authorization_mode,policy_revision)
 SELECT ?,pep_client_id,application_id,provider_id,lifecycle,valid_until,updated_at,?,authorization_mode,policy_revision
 FROM provider_pep_clients WHERE provider_id=? AND pep_client_id=?`, Args: []any{receipt, actor, p, id}, ExpectedRowsAffected: requireOne()}}
	if expectedRevision != nil {
		ss = append([]Statement{assertion(`SELECT 1 WHERE
 (?=0 AND NOT EXISTS(SELECT 1 FROM provider_pep_clients WHERE pep_client_id=?)) OR
 (? > 0 AND EXISTS(SELECT 1 FROM provider_pep_clients WHERE provider_id=? AND pep_client_id=? AND policy_revision=?))`,
			*expectedRevision, id, *expectedRevision, p, id, *expectedRevision)}, ss...)
	}
	if e := s.Transaction(ctx, ss); e != nil {
		if expectedRevision != nil && strings.Contains(e.Error(), "transaction rejected") {
			return ProviderPepClient{}, serviceError(412, "PEP policy precondition failed")
		}
		return ProviderPepClient{}, capabilityWriteError(e)
	}
	// Read this write's receipt, rather than a possibly newer concurrent policy.
	rows, e := s.Query(ctx, ReadStrong, `SELECT r.pep_client_id,r.application_id,r.provider_id,r.lifecycle,r.valid_until,
 r.authorization_mode,r.policy_revision,p.created_at,r.changed_at AS updated_at
 FROM provider_pep_client_receipts r JOIN provider_pep_clients p ON p.pep_client_id=r.pep_client_id AND p.provider_id=r.provider_id
 WHERE r.receipt_id=?`, receipt)
	if e != nil {
		return ProviderPepClient{}, e
	}
	if len(rows) != 1 {
		return ProviderPepClient{}, serviceError(503, "PEP policy result unavailable")
	}
	return decodeRow[ProviderPepClient](rows[0])
}

// ProviderPepClientAdmin reads durable policy even when its provider or PEP is
// disabled, expired or deleted, and does not require an active capability catalog.
func (s *Store) ProviderPepClientAdmin(ctx context.Context, p, id string) (ProviderPepClient, error) {
	rows, err := s.Query(ctx, ReadStrong, "SELECT * FROM provider_pep_clients WHERE provider_id=? AND pep_client_id=?", p, id)
	if err != nil {
		return ProviderPepClient{}, err
	}
	if len(rows) != 1 {
		return ProviderPepClient{}, serviceError(404, "PEP policy not found")
	}
	return decodeRow[ProviderPepClient](rows[0])
}
func (s *Store) ProviderPepClient(ctx context.Context, p, id string) (ProviderPepClient, error) {
	r, e := s.Query(ctx, ReadStrong, "SELECT pc.* FROM provider_pep_clients pc JOIN service_providers sp ON sp.id=pc.provider_id AND sp.active=1 WHERE pc.provider_id=? AND pc.pep_client_id=?", p, id)
	if e != nil {
		return ProviderPepClient{}, e
	}
	if len(r) != 1 {
		return ProviderPepClient{}, serviceError(403, "provider PEP client not found")
	}
	return decodeRow[ProviderPepClient](r[0])
}
func requireActivePEP(v ProviderPepClient, at time.Time) error {
	if v.Lifecycle != "active" {
		return serviceError(403, "provider PEP client is not active")
	}
	if v.ValidUntil != nil {
		expiry, e := time.Parse(time.RFC3339Nano, *v.ValidUntil)
		if e != nil || !expiry.After(at) {
			return serviceError(403, "provider PEP client expired")
		}
	}
	return nil
}
func (s *Store) ActivateHttpCapabilityGeneration(ctx context.Context, p, actor string, in ActivateHttpCapabilityGeneration) (HttpCapabilityActivation, error) {
	var out HttpCapabilityActivation
	if !capabilityToken(p) || in.TargetGeneration == 0 || !validDigest(in.TargetCatalogDigest) || (in.ExpectedGeneration == nil) != (in.ExpectedCatalogDigest == nil) {
		return out, serviceError(422, "invalid activation")
	}
	if in.ExpectedCatalogDigest != nil && !validDigest(*in.ExpectedCatalogDigest) {
		return out, serviceError(422, "expectedCatalogDigest must be sha256 digest")
	}
	at := time.Now().UTC()
	cutoff := at.Format("2006-01-02T15:04:05.000000000Z")
	r, e := s.Query(ctx, ReadStrong, "SELECT pc.* FROM provider_pep_clients pc JOIN service_providers sp ON sp.id=pc.provider_id AND sp.active=1 WHERE pc.provider_id=? AND pc.lifecycle='active' AND (pc.valid_until IS NULL OR pc.valid_until>?) ORDER BY pc.pep_client_id", p, cutoff)
	if e != nil {
		return out, e
	}
	if len(r) == 0 {
		return out, serviceError(409, "activation requires an active unexpired PEP client")
	}
	pep, e := decodeRow[ProviderPepClient](r[0])
	if e != nil {
		return out, e
	}
	receipt := requestID()
	ss := []Statement{assertion("SELECT 1 WHERE EXISTS(SELECT 1 FROM service_providers WHERE id=? AND active=1) AND EXISTS(SELECT 1 FROM provider_http_capability_sets WHERE provider_id=? AND generation=? AND catalog_digest=?) AND ((? IS NULL AND NOT EXISTS(SELECT 1 FROM provider_http_capability_active WHERE provider_id=?)) OR EXISTS(SELECT 1 FROM provider_http_capability_active WHERE provider_id=? AND generation=? AND catalog_digest=?)) AND EXISTS(SELECT 1 FROM provider_pep_clients pc WHERE pc.provider_id=? AND pc.pep_client_id=? AND pc.lifecycle='active' AND pc.valid_until IS ? AND (pc.valid_until IS NULL OR pc.valid_until>?) )", p, p, in.TargetGeneration, in.TargetCatalogDigest, in.ExpectedGeneration, p, p, in.ExpectedGeneration, in.ExpectedCatalogDigest, p, pep.PepClientID, pep.ValidUntil, cutoff),
		{SQL: "INSERT INTO provider_http_capability_activations(activation_receipt_id,provider_id,activation_sequence,from_generation,to_generation,catalog_digest,activated_at,activated_by) SELECT ?,?,COALESCE((SELECT MAX(activation_sequence) FROM provider_http_capability_activations WHERE provider_id=?),0)+1,(SELECT generation FROM provider_http_capability_active WHERE provider_id=?),?,?,?,?", Args: []any{receipt, p, p, p, in.TargetGeneration, in.TargetCatalogDigest, cutoff, actor}},
		{SQL: "INSERT INTO provider_http_capability_active(provider_id,generation,catalog_digest,activated_at,activated_by) VALUES(?,?,?,?,?) ON CONFLICT(provider_id) DO UPDATE SET generation=excluded.generation,catalog_digest=excluded.catalog_digest,activated_at=excluded.activated_at,activated_by=excluded.activated_by", Args: []any{p, in.TargetGeneration, in.TargetCatalogDigest, cutoff, actor}}}
	// Staged generations may outlive their action registrations. Check all
	// capability and response-graph references in the activation transaction.
	ss = append([]Statement{assertion(`SELECT 1 FROM provider_http_capability_sets s
 WHERE provider_id=? AND generation=? AND NOT EXISTS
 (SELECT 1 FROM json_tree(s.canonical_json) ref WHERE ref.key='actionId' AND ref.value IS NOT NULL
 AND NOT EXISTS(SELECT 1 FROM actions a WHERE a.id=ref.value AND a.provider_id=s.provider_id))`, p, in.TargetGeneration)}, ss...)
	if e = s.Transaction(ctx, ss); e != nil {
		return out, capabilityWriteError(e)
	}
	r, e = s.Query(ctx, ReadStrong, "SELECT * FROM provider_http_capability_activations WHERE activation_receipt_id=?", receipt)
	if e != nil {
		return out, e
	}
	return decodeRow[HttpCapabilityActivation](r[0])
}
func (s *Store) ActiveHttpCapabilityCatalogAdmin(ctx context.Context, p string) (AdminHttpCapabilityCatalog, error) {
	var out AdminHttpCapabilityCatalog
	rows, err := s.Query(ctx, ReadStrong, `WITH RECURSIVE catalog AS (
 SELECT s.canonical_json,a.catalog_digest,a.activated_at,a.activated_by
 FROM provider_http_capability_active a JOIN provider_http_capability_sets s
 ON s.provider_id=a.provider_id AND s.generation=a.generation AND s.catalog_digest=a.catalog_digest
 WHERE a.provider_id=?), chunks(payload,remaining,catalog_digest,activated_at,activated_by,ordinal) AS (
 SELECT substr(canonical_json,1,60000),substr(canonical_json,60001),catalog_digest,activated_at,activated_by,0 FROM catalog
 UNION ALL SELECT substr(remaining,1,60000),substr(remaining,60001),catalog_digest,activated_at,activated_by,ordinal+1 FROM chunks WHERE remaining<>'')
 SELECT payload,catalog_digest,activated_at,activated_by FROM chunks ORDER BY ordinal`, p)
	if err != nil {
		return out, err
	}
	if len(rows) == 0 {
		providers, err := s.Query(ctx, ReadStrong, "SELECT id FROM service_providers WHERE id=?", p)
		if err != nil {
			return out, err
		}
		if len(providers) == 0 {
			return out, serviceError(404, "service provider not found")
		}
		return out, serviceError(404, "HTTP capability catalog not active")
	}
	var canonical strings.Builder
	for _, row := range rows {
		canonical.WriteString(rowString(row, "payload"))
	}
	var v canonicalGeneration
	if err = json.Unmarshal([]byte(canonical.String()), &v); err != nil {
		return out, err
	}
	return AdminHttpCapabilityCatalog{SchemaVersion: 1, ProviderID: p, Generation: v.Generation, CatalogDigest: rowString(rows[0], "catalog_digest"), ActivatedAt: rowString(rows[0], "activated_at"), ActivatedBy: rowString(rows[0], "activated_by"), CapabilityCount: len(v.Capabilities), ResponseResourceCount: len(v.ResponseResources), Capabilities: v.Capabilities, ResponseResources: v.ResponseResources}, nil
}
func (s *Store) ActiveHttpCapabilityCatalog(ctx context.Context, p, pepID string) (ActiveHttpCapabilityCatalog, error) {
	var out ActiveHttpCapabilityCatalog
	pep, e := s.ProviderPepClient(ctx, p, pepID)
	if e != nil {
		return out, e
	}
	if e = requireActivePEP(pep, time.Now()); e != nil {
		return out, e
	}
	r, e := s.Query(ctx, ReadStrong, "WITH RECURSIVE catalog AS (SELECT s.canonical_json,a.catalog_digest FROM provider_http_capability_active a JOIN provider_http_capability_sets s ON s.provider_id=a.provider_id AND s.generation=a.generation AND s.catalog_digest=a.catalog_digest JOIN service_providers sp ON sp.id=a.provider_id AND sp.active=1 WHERE a.provider_id=?), chunks(payload,remaining,catalog_digest,ordinal) AS (SELECT substr(canonical_json,1,60000),substr(canonical_json,60001),catalog_digest,0 FROM catalog UNION ALL SELECT substr(remaining,1,60000),substr(remaining,60001),catalog_digest,ordinal+1 FROM chunks WHERE remaining<>'') SELECT payload,catalog_digest FROM chunks ORDER BY ordinal", p)
	if e != nil {
		return out, e
	}
	if len(r) == 0 {
		return out, serviceError(403, "HTTP capability catalog not active")
	}
	var canonical strings.Builder
	for _, row := range r {
		canonical.WriteString(rowString(row, "payload"))
	}
	var v canonicalGeneration
	if e = json.Unmarshal([]byte(canonical.String()), &v); e != nil {
		return out, e
	}
	now := time.Now().UTC()
	return ActiveHttpCapabilityCatalog{SchemaVersion: 1, ProviderID: p, PepClientID: pepID, Generation: v.Generation, CatalogDigest: rowString(r[0], "catalog_digest"), Capabilities: v.Capabilities, ResponseResources: v.ResponseResources, IssuedAt: now.Format(time.RFC3339Nano), ExpiresAt: now.Add(30 * time.Second).Format(time.RFC3339Nano)}, nil
}
func capabilityWriteError(err error) error {
	if strings.Contains(err.Error(), "transaction rejected") {
		return serviceError(409, "HTTP capability preconditions changed")
	}
	return err
}
