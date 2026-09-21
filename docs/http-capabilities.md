# Provider HTTP capabilities

Current support is limited to internal service Providers deployed directly in Kubernetes (K8s). External Application integration is deferred to the [external application roadmap](external-application-roadmap.md). The contracts below remain unchanged and do not constitute an external onboarding or credential-distribution procedure.

Grant stores authorization-significant HTTP contracts; it does not proxy provider traffic and never stores an upstream origin.

The Phase A API is inactive by default:

- `POST /api/v1/admin/providers/{providerId}/http-capability-generations` stages one immutable next generation.
- `PUT /ops/providers/{providerId}/enforcers/{pepClientId}` binds one globally unique active/ready application identity to one provider.
- `POST /api/v1/admin/providers/{providerId}/http-capability-active` atomically activates or rolls back the provider pointer.
- `GET /api/v2/http-capabilities/catalog/{providerId}` returns only the active pointer generation to the configured service-authenticated PEP application.
- `POST /api/v2/http-capabilities/decide` re-resolves the active capability, verifies the exact method/path/query/resource/digest/credential evidence, evaluates the existing RBAC/application policy tuple and returns a short-lived receipt.

Admin routes require `X-Grant-Master-Key`. Fetch and decision require `X-Grant-App-Auth-Key`; the server additionally resolves that credential to the single application configured as `GRANT_HTTP_CAPABILITY_PEP_APPLICATION_ID` and checks its active PEP binding. Authorized decisions also require the same end-user Bearer credential and organization context used by the provider-forwarding request.

`public_authn` entries deliberately have no action/resource and never call the decision endpoint. They are still exact catalog entries; The provider PEP owns their limiter, secret redaction and fixed-origin forwarding. `authorized` entries must name an existing provider action and one closed resource extractor. Phase A accepts only `pathSegment`, `queryScalar`, `jsonPointer`, and `constant`; `stringTemplate` is not a stageable contract. A query scalar must be a required member of the exact query policy. A JSON pointer is the explicit body-extraction trust boundary: the authenticated provider PEP supplies the scalar, while Grant binds it to the exact forwarded JSON body digest and length, the opaque forwarded-header digest, principal, and request. Grant does not independently parse or verify the `Content-Type` header value. Phase B must prove that same-principal/request extraction before enabling such a capability. Unknown extractor kinds, methods, paths, query shapes, stale catalogs, disabled capabilities and cross-provider PEP identities fail closed.

The three Exposure-specific routes (`/api/v1/exposure-authz/read`, `/api/v1/exposure-authz/plant-read`, `/api/v2/exposure-authz/decide`) and their fixed `plant.get` contract are retired and return 404. Generic HTTP capability routes remain supported. Consumers must explicitly provision the provider/action, stage and activate a catalog, bind their PEP, and send the required request evidence. No automatic alias or migration is provided. V1 authority import and the PocketBase application identity configuration remain supported.

PocketBase deployments use the configured canonical issuer and verified collection/record identity. Credential evidence subject is `pocketbase:<issuer>/<collectionId>/<recordId>`; audience is the server-configured PocketBase application ID. These values are verified against auth-refresh, not accepted from caller JWT claims.

`request.authorization` selects `forward-user` (also the default when omitted) or `service-token`. The latter is allowed only for `authorized` capabilities, is covered by contract/catalog digests, and requires the PEP to replace the caller credential with its configured server-owned upstream credential only after Grant allows the exact request. No service credential is stored in the catalog or returned by Grant. `public_authn` cannot select `service-token`.

`response.errorStatuses` optionally lists sorted, unique HTTP statuses 304 or 400 through 599 that the proxy may preserve after enforcing the response body/header limits. Empty or absent retains the default upstream-error behavior. Redirect statuses are never admitted by this field. This list is part of the canonical capability digest.

A path template may contain multiple uniquely named variables. A `pathSegment` resource extractor selects exactly one existing variable; the other segments stay bound by the complete canonical request path. By default, every captured segment must contain 1–256 ASCII letters, digits, `.`, `-`, `_`, or `:`; empty segments, `.`/`..`, percent encoding and backslashes are rejected. The selected resource additionally satisfies its configured scalar format. This supports nested management routes without granting authority over arbitrary path patterns.

## Bounded request and file transport

`request.body` supports `none`, `json`, `opaque`, and `multipart`. Opaque and multipart bodies require a positive `maxBytes`, a sorted `contentTypes` list of canonical media types, and `content-type` in `forwardHeaders`. Exposure hashes and forwards the original bytes, including multipart boundaries; it does not rewrite JSON or multipart fields. Body resource extraction remains JSON-only. Constant, query, and JSON extractors may coexist with independently validated path variables.

Query parameters may use `format: text` with `maxBytes` from 1 to 4096 and explicit `repeated: true`. Canonical query pairs use strict RFC3986 escaping, sorted keys and preserved order within repeated values; the original upstream query remains unchanged except for the explicit record filter described below. A repeated parameter cannot identify a single resource. `pathParameters` can declare a variable as `utf8Segment` with a decoded-byte bound; encoded separators, percent signs after decoding, traversal, controls and invalid UTF-8 are rejected. Undeclared variables retain the existing ASCII-only behavior.

File and conditional request headers can be explicitly allowed, including Range and If-Range, and bounded responses can preserve Content-Range, Accept-Ranges, Content-Disposition and Content-Length. The existing `errorStatuses` list also accepts 304, alongside 400–599; it does not enable redirect following.

## Record-level list scope

An authorized GET catalog entry can declare `recordScope` with `filterParameter`, `rowKey`, `grammar: fexpr`, `maxIds`, `maxFilterBytes`, sorted root-level `allowedFields`, and an optional `sortParameter`. Its existing action and resource type identify per-record authority. The constant resource ID identifies the list request only; it does not grant all records or require a fabricated collection permission.

Grant verifies principal, active organization membership, application action scope and consent, then evaluates exact candidate record IDs through its existing authorization engine in one fenced view. The decision includes `recordScope.ids` as a sorted unique list. An empty array means a legitimately empty result scope; a missing scope is not equivalent. Literal `*` remains a literal resource ID, never a wildcard. Candidate enumeration is capped at 1000 and authorized output at the catalog's `maxIds` (at most 199); overflow fails closed rather than truncating.

Exposure independently parses the client filter, permits only catalog-declared scalar fields, and combines the ACL expression with the client expression using AND. It sends this filter to the existing upstream API so pagination and totals describe only authorized rows. Scope and client expressions must fit the upstream fexpr limit of 200 expressions and the catalog's byte bound. Related-record expansion, relation traversal and undeclared query parameters are rejected until an explicit authorization contract supports them. Catalog authors must supply field semantics; a field name alone does not establish that it is safe or non-relational.

A record-scope catalog permits no arbitrary query passthrough. Apart from its bound filter and sort parameters, it may declare `pagination.pageParameter`, `pagination.pageSizeParameter`, and `pagination.skipTotalParameter`. Page values use canonical unsigned 32-bit decimal (`format: uint`); the count flag uses exactly `true` or `false` (`format: bool`). Every remaining query parameter is rejected, even if its value would satisfy a scalar format. This blocks undeclared expansion, field projection and unbound sort options. The operator is trusted to map these roles to genuine upstream pagination/count controls; the proxy cannot discover semantics from an arbitrary parameter name. Root-field and resource mappings have the same explicit trust boundary.

## Atomic request-array batches

An authorized POST with a constant resource and no query can declare `batch`. The request uses bounded opaque JSON/multipart transport; the batch policy identifies the root request array, metadata fields, eligible child capability IDs and optional upsert/multipart encodings. This is generic catalog data, not a PocketBase route switch inside either runtime. Eligible children are enabled, non-batch POST/PATCH/DELETE capabilities with constant/path resource extraction, no query, and the same upstream authorization mode. Child metadata is closed; arbitrary record-body properties remain provider input.

```json
{"encoding":"request-array","requestsPointer":"/requests","methodField":"method","urlField":"url","bodyField":"body","headersField":"headers","maxOperations":20,"allowedCapabilityIds":["files.create.0","files.update.0"],"upserts":[{"method":"PUT","pathTemplate":"/api/collections/files/records","createCapabilityId":"files.create.0","updateCapabilityId":"files.update.0","idPointer":"/id"}],"multipart":{"payloadField":"@jsonPayload","fileFieldPatterns":["requests.{index}.{field}","requests[{index}].{field}"]}}
```

Exposure interprets the bounded original request before sending anything upstream. It rejects recursive batches, unknown child routes/metadata, origins, queries, supplied child headers, ambiguous multipart payloads and invalid attachment indices. It forwards the original outer bytes exactly once only after one Grant decision permits the root and every child. PocketBase retains responsibility for validating and atomically executing its native batch; Exposure does not fan children out into separate HTTP writes.

The decision input adds ordered `batchChildren` plus `batchDigest`. Each child carries its original array index/method/path, active capability ID/version/contract digest, derived method/path, canonical empty query and header digests, semantic JSON body digest/length, and existing resource evidence. The original outer `bodyDigest`/`bodyLength` bind all forwarded bytes, including multipart attachments. Child body digests describe the interpreted JSON body, not the multipart HTTP body synthesized internally by PocketBase. Exposure is the trusted extraction boundary, as with `jsonPointer`; Grant does not independently reconstruct the original body from these descriptors.

`batchDigest` uses the same canonical JSON SHA-256 algorithm as capability contracts, applied to the ordered child descriptor array. The decision echoes it together with the existing outer request binding, body digest and principal metadata; Exposure validates all of them before forwarding. Ordinary capabilities reject unexpected batch evidence. Grant rechecks the active catalog, PEP and principal and evaluates root plus all children within one fenced read snapshot. Any denied child denies the entire request; no partial authorization receipt or upstream write is produced.

A declared PUT upsert with an ID requires both create(collection) and update(record) authority. It emits create then update descriptors at the same index, sharing `originalMethod`, `originalPath`, `upsertId` and semantic body digest/length. Grant validates the exact pair against the catalog recipe. An upsert without an ID requires the create check only. This conservatively avoids an existence pre-read and its race; even when the eventual provider branch is create, an explicit ID still needs both permissions. The catalog currently bounds requests at 20 operations (at most 40 checks for upsert pairs).

## Buffered response expansion authorization

A catalog may include one `responseResources` graph after `capabilities`. The field is omitted when absent, preserving legacy catalog digest bytes. Each graph node declares `key`, `actionId`, `resourceType`, `idField`, `idFormat`, `identityField`, `identityValue`, and sorted `relations:[{field,targetKey}]`. Relation edges permit traversal to a separately authorized resource; they never inherit a parent's grant. Graph actions must be registered for the provider. Cycles are allowed in schema data, but each requested path and returned occurrence is bounded.

A GET record/list capability can reference that graph with:

```json
{"rootKey":"permissions","recordsPointer":"/items","expansionField":"expand","queryParameter":"expand","maxDepth":6,"maxRecords":200}
```

This object is stored as `responseAuthorization`. An empty `recordsPointer` means a detail object; a list selects a declared root array and retains its `recordScope`. The query parameter is optional bounded text. Both runtimes validate comma-separated relation paths against the graph before the upstream read; unknown edges, reverse/join forms and excessive depth are rejected. Ordinary reads without requested expansion retain their existing single decision path.

For requested expansion, Exposure buffers the native response before writing headers or body to the caller. It validates record identities, graph edges, requested paths and parent relation IDs, then describes every root and expanded occurrence as ordered `responseReferences:[{resourceKey,id,parentIndex?,relationField?}]`. Root references have no parent; child references point backward to a parent and name its declared relation edge. Maximum depth is six and maximum total occurrences is 200, including repeated records. Missing/null relations may yield no child; unknown/unrequested expanded fields do not become implicit grants. The native error/status and bounded-body behavior remains governed by the response contract.

One post-response decision reuses the original request/principal binding and adds `responseDigest`, `responseLength`, and `responseReferencesDigest`. The last digest is canonical JSON SHA-256 over the ordered references, with an empty list represented as `[]`. Grant verifies the graph-derived action/resource tuples and rechecks the active catalog, PEP, principal, root access and every related record within one fenced snapshot. Scoped lists recheck admission and exact root IDs, rather than requiring a fabricated collection-wide grant; an empty list can still pass admission with no references. A missing related grant or a revoked root denies the whole buffered response.

The response echoes both digests and length, and Exposure verifies them before forwarding the original bytes unchanged. It does not trim expansion fields, prune records, or rewrite native counts. If post-response authorization is denied or unavailable, none of the upstream response bytes are released. The trusted Exposure parser is the response-extraction boundary; Grant verifies its digest-bound descriptors rather than independently receiving and reparsing the business response body.
