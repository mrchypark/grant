# Unary gRPC capabilities

This contract covers USER authorization for unary business RPCs. Exposure is the policy enforcement point (PEP): it verifies the call, asks Grant for a decision, then enforces that decision at the configured upstream. Grant is the policy decision point (PDP) and uses HTTP/JSON for every endpoint below. Existing HTTP capability routes, catalogs and contracts are unchanged. This document does not establish support in any deployed Exposure version.

## Endpoints and authority

| Method and path | Purpose |
| --- | --- |
| `GET /api/v2/grpc-capabilities/catalog/{providerId}` | `200`: fetch the active catalog with `pepClientId`, `authorizationMode: "user"`, `pepPolicyRevision`, `issuedAt` and `expiresAt`. Its lease is at most 30 seconds. |
| `POST /api/v2/grpc-capabilities/decide` | `200`: decision for one unary call against the current active generation and PEP policy. |
| `POST /api/v1/admin/providers/{providerId}/grpc-capability-generations` | `201`: stage and return an immutable catalog. |
| `POST /api/v1/admin/providers/{providerId}/grpc-capability-active` | `200`: change the active generation using compare-and-swap (CAS) and return its catalog. |

Catalog `GET` and `HEAD` are bodyless. Grant rejects requests that declare or stream a body, including chunked requests, without reading the body to check whether it is empty. Other methods on the catalog path do not reach the catalog handler or authorize a call. Authorization decisions use `POST /api/v2/grpc-capabilities/decide`.

The voter-only admin routes require `X-Grant-Master-Key`. The runtime routes require `X-Grant-App-Auth-Key` and use the provider binding of the server's configured `GRANT_HTTP_CAPABILITY_PEP_CLIENT_ID`, which must be active and in USER mode. The decision also requires one `Authorization: Bearer …` and one nonempty `X-Organization-ID`; duplicate values for either header are rejected. Grant verifies the credential and the user's active organization membership, then evaluates the registered action and exact resource with USER RBAC. Application admission and consent do not apply. The caller cannot choose a different authorization mode, provider binding, or PEP revision. At issuance, Grant returns `403` if the PEP's `validUntil` is at or before `issuedAt`. Otherwise, when `validUntil` is set, `expiresAt` is the earlier of `validUntil` and the normal lifetime limit. The PEP must stop using the catalog at its returned `expiresAt`. The catalog lease does not extend the lifetime of a decision or permit using a stale catalog or policy revision at the PDP.

Send one app key header. Grant checks the first value if multiple are supplied; the decision's duplicate header check applies to Authorization and organization, not to the app key.

Stage accepts `schemaVersion: 1`, `generation`, `expectedLatestGeneration`, `catalogDigest`, and `capabilities`. Use `expectedLatestGeneration: null` and `generation: 1` for the first generation; otherwise supply the latest stored generation and its successor. An identical stored generation can be restaged without changing it. Stage validates the provider and its registered actions and leaves the active pointer unchanged.

Activation accepts `expectedGeneration`, `expectedCatalogDigest`, `targetGeneration`, and `targetCatalogDigest`. Use null for both expected values when there is no active pointer. Otherwise both must match the current pointer. Activation selects a staged target atomically, including a prior generation for rollback. An unavailable target or changed preconditions return `409`; invalid admin catalog or activation values return `422`. The gRPC catalog and activation state are separate from HTTP capability state.

## Capability and call evidence

A v1 catalog has `schemaVersion: 1`, `providerId`, `generation`, `catalogDigest`, and `capabilities`. Each capability has `capabilityId`, `version`, `enabled`, `fullMethod`, `descriptorDigest`, `actionId`, `resourceType`, `resourceId`, `maxMessageBytes`, and `contractDigest`. `fullMethod` is an exact, case-sensitive `/package.Service/Method`. Resources are constants: v1 has no extractors based on protobuf fields, paths, query parameters or metadata, and no wildcard methods or streaming RPCs. The PEP checks the pinned descriptor and message bytes; Grant does not receive or parse either.

Register a method only if authorization for its one configured resource is sufficient for the entire message and all targets the RPC can affect. A message digest binds bytes but does not authorize IDs inside those bytes. A method accepting arbitrary target IDs requires separate authorization for those targets; do not register it under a coarse constant resource as a bypass.

The catalog permits up to 256 entries and 48 KiB of canonical JSON. Capability ID/version pairs and enabled methods must be unique; generations and versions are positive signed 64-bit integers, and `maxMessageBytes` is 1 through 16 MiB (16,777,216 bytes).

`contractDigest` hashes the capability object with its own `contractDigest` omitted. `catalogDigest` hashes the catalog object with its own `catalogDigest` omitted, preserving capability array order and including each entry's `contractDigest`. Use `[]` for an empty capability array. Both hash compact UTF-8 JSON with object keys sorted lexicographically, integer values in decimal, and no trailing newline. Grant does not HTML-escape `<`, `>` or `&`. Actual U+2028 and U+2029 characters are emitted as UTF-8, not Unicode escapes. Literal text consisting of a backslash followed by `u2028` or `u2029` keeps its escaped backslash in JSON. Neither CAS fields nor PEP lease fields enter the catalog hash. These and the request digests below use `sha256:` followed by 64 lowercase hex digits.

| Evidence | Definition |
| --- | --- |
| Descriptor pin | SHA-256 of the exact pinned serialized `FileDescriptorSet` bytes. The PEP resolves the exact method in that set and checks that both client and server streaming are false. Do not hash a parsed, reordered, or reserialized descriptor set. |
| Message digest and length | SHA-256 and byte count of the exact outbound protobuf message. Exclude the gRPC frame and compression; do not hash decoded fields, JSON, or a different protobuf encoding of the same fields. |
| Metadata digest | SHA-256 of zero bytes. V1 forwards no application metadata. Transport headers and the separately bound credential and organization evidence are not application metadata. |
| Provider credential digest | Let the credential be the entire Authorization header value after trimming leading and trailing whitespace, for example `Bearer <token>`. Hash the ASCII domain `grant.http-capability.provider-credential.v1`, followed by the credential byte length as an unsigned 64-bit big-endian integer and the credential bytes. Include the scheme and separating space and preserve scheme case and interior whitespace. Hashing only the token yields a digest mismatch (`403`). |

Exposure uses a trusted upstream origin configured outside the catalog and authenticates that origin according to its transport configuration. Client authority, URLs, and metadata cannot select an upstream or substitute credentials. The Bearer and organization headers above are PDP inputs, not an upstream metadata allowlist. Grant master and service keys must never be sent to the business origin.

## Decision request and receipt

The decision request binds these fields:

- `wireVersion: 1`, `providerId`, `pepClientId`, and a positive `pepPolicyRevision` matching the current USER policy.
- `catalogGeneration`, `catalogDigest`, `capabilityId`, `capabilityVersion`, and `contractDigest`.
- `fullMethod`, `descriptorDigest`, `rpcKind: "unary"`, `resourceType`, `resourceId`, `messageDigest`, `messageLength`, and `forwardedMetadataDigest`. Zero-length messages use SHA-256 of zero bytes.
- `providerCredentialDigest`, `providerCredential: {"subject": "…", "audience": "…"}`, and a fresh unpredictable `requestBinding` unique to the pending call. Grant checks the binding's URL-safe alphabet (`A-Z`, `a-z`, `0-9`, `-`, `_`) and length (22–256 characters); the PEP generates it and prevents reuse.

Send the original Bearer credential and required organization with the PDP request. Grant re-resolves the enabled capability and checks the submitted evidence against current policy; caller-supplied evidence is not a source of authority.

A decision echoes every request field exactly and adds `allowed`, `reason`, `actionId`, `organizationId`, `principalDigest`, `issuedAt` and `expiresAt`. Timestamps are UTC RFC 3339 with optional fractional seconds. The principal digest is lowercase hex SHA-256 without a prefix: hash the ASCII domain `grant.exposure-read.principal.v1` followed by the local user ID, organization ID and empty application ID, each prefixed by its unsigned 64-bit big-endian byte length. The PEP compares the echoed evidence, expected action and organization with the pending call and enforces the returned `expiresAt`. The receipt lifetime is at most five seconds and may be shorter because of the PEP's `validUntil`. Only `allowed: true` can authorize one send: consume that receipt once and obtain a fresh decision for any retry. Keep the method, message, credential context and configured origin fixed between authorization and the send.

An RBAC denial can be `200` with `allowed: false`. Grant's application errors are HTTP JSON with `error` and optional `code`:

| Status | Examples |
| --- | --- |
| `400` | Malformed JSON or auth header; missing organization; duplicate Authorization or organization header; invalid digest syntax, request binding, wire version or RPC kind. |
| `401` | Missing or incorrect app/master key; missing Bearer credential when the other required headers are valid. |
| `403` | Wrong or inactive PEP policy, stale generation or revision, well-formed digest that fails the binding check, or nonempty application metadata evidence. |
| `404` | No active gRPC catalog for the authorized provider. |
| `409` | Admin CAS preconditions changed or the activation target is unavailable. |
| `413` / `415` | JSON request body exceeds the size limit / unsupported content type. |
| `422` | JSON violates the request schema (for example, a missing, unknown, duplicate or disallowed null field); invalid catalog or activation values. |
| `503` | Authority unavailable, invalid stored catalog, or a voter-only request sent to a learner. |

The PEP must reject every non-allow result, malformed or mismatched receipt, expired receipt, and ambiguous or unavailable PDP response. This contract imposes no gRPC status wire format on Grant.

Before relying on the contract, verify an allowed call reaches only the configured origin once. Check that mismatches in method, unary cardinality, descriptor, exact message bytes, metadata, credential, organization, provider, revision, generation and receipt binding produce zero upstream calls. Verify that a near-expiry PEP gets shorter lifetimes and an expired PEP gets neither a catalog lease nor a decision receipt. Verify expired and reused receipts, unavailable PDP, and stale activation CAS separately, and confirm that the existing HTTP capability path still works.
