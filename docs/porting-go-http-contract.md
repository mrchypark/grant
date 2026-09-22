# Go HTTP compatibility contract

Baseline: `c716877` (same application tree as `ee5852b`).

The current branch registers 81 method/path entries across 54 distinct paths. The executable inventory is `internal/api/testdata/routes-voter.txt`. GET also accepts HEAD. This is a code contract, not evidence that IED has received the new management catalog.

## Method, path, authorization and node role

| Method | Path | Authentication | Voter only | Success |
|---|---|---|---|---|
| POST | `/api/v1/admin/providers/{providerId}/http-capability-generations` | master | true | 201 |
| POST | `/api/v1/admin/providers/{providerId}/http-capability-active` | master | true | 200 |
| GET | `/api/v1/admin/providers/{providerId}/http-capability-active` | master | true | 200 |
| PUT | `/ops/providers/{providerId}/enforcers/{pepClientId}` | master | true | 200 |
| GET | `/api/v2/http-capabilities/catalog/{providerId}` | app | false | 200 |
| POST | `/api/v2/http-capabilities/decide` | application | false | 200 |
| POST | `/ops/migrations/patch-data-authority` | master | true | 200 |
| GET | `/api/v1/me` | bearer | false | 200 |
| POST | `/api/v1/me/materialize` | bearer | true | 200 |
| GET | `/api/v1/providers` |  | false | 200 |
| POST | `/api/v1/admin/providers` | master | true | 201 |
| PUT | `/api/v1/admin/providers/{providerId}/actions` | master | true | 200 |
| GET | `/api/v1/admin/providers/{providerId}/actions` | master | true | 200 |
| GET | `/api/v1/orgs/{organizationId}/providers/{providerId}/actions` | user | true | 200 |
| GET | `/api/v1/orgs/{organizationId}/apps` | user | false | 200 |
| POST | `/api/v1/orgs/{organizationId}/apps` | user | true | 201 |
| GET | `/api/v1/orgs/{organizationId}/apps/{applicationId}` | user | false | 200 |
| GET | `/api/v1/orgs/{organizationId}/apps/{applicationId}/actions` | user | false | 200 |
| PUT | `/api/v1/orgs/{organizationId}/apps/{applicationId}/actions` | user | true | 200 |
| GET | `/api/v1/orgs/{organizationId}/app-grants` | user | false | 200 |
| GET | `/api/v1/orgs/{organizationId}/apps/{applicationId}/consent` | user | false | 200 |
| PUT | `/api/v1/orgs/{organizationId}/apps/{applicationId}/consent` | user | true | 200 |
| DELETE | `/api/v1/orgs/{organizationId}/apps/{applicationId}/consent` | user | true | 204 |
| PUT | `/api/v1/admin/orgs/{organizationId}/apps/{applicationId}/trust` | master | true | 200 |
| PUT | `/api/v1/admin/users/{userId}/active` | master | true | 200 |
| GET | `/api/v1/orgs/{organizationId}/groups` | user | false | 200 |
| POST | `/api/v1/orgs/{organizationId}/groups` | user | true | 201 |
| GET | `/api/v1/orgs/{organizationId}/groups/{groupId}` | user | false | 200 |
| PUT | `/api/v1/orgs/{organizationId}/groups/{groupId}` | user | true | 200 |
| DELETE | `/api/v1/orgs/{organizationId}/groups/{groupId}` | user | true | 204 |
| GET | `/api/v1/orgs/{organizationId}/groups/{groupId}/members` | user | false | 200 |
| PUT | `/api/v1/orgs/{organizationId}/groups/{groupId}/members/{userId}` | user | true | 200 |
| DELETE | `/api/v1/orgs/{organizationId}/groups/{groupId}/members/{userId}` | user | true | 204 |
| PUT | `/api/v1/orgs/{organizationId}/members/{userId}` | user | true | 200 |
| GET | `/api/v1/orgs/{organizationId}/agreements` | user | false | 200 |
| POST | `/api/v1/orgs/{organizationId}/agreements` | user | true | 201 |
| GET | `/api/v1/orgs/{organizationId}/agreements/{agreementId}` | user | false | 200 |
| PUT | `/api/v1/orgs/{organizationId}/agreements/{agreementId}` | user | true | 200 |
| GET | `/api/v1/orgs/{organizationId}/roles` | user | false | 200 |
| POST | `/api/v1/orgs/{organizationId}/roles` | user | true | 201 |
| PUT | `/api/v1/orgs/{organizationId}/roles/{roleId}/actions` | user | true | 200 |
| GET | `/api/v1/orgs/{organizationId}/bindings` | user | false | 200 |
| POST | `/api/v1/orgs/{organizationId}/bindings` | user | true | 201 |
| DELETE | `/api/v1/orgs/{organizationId}/bindings/{bindingId}` | user | true | 204 |
| POST | `/api/v1/orgs/{organizationId}/deactivate` | user | true | 200 |
| POST | `/api/v1/orgs/{organizationId}/reactivate` | inactive | true | 200 |
| POST | `/api/v1/admin/bootstrap/owner` | master | true | 201 |
| POST | `/ops/backups` | master | true | 202 |
| GET | `/ops/backups` | master | true | 200 |
| POST | `/api/v1/authz/check` | master | false | 200 |
| POST | `/api/v1/authz/batch-check` | master | false | 200 |
| POST | `/api/v1/authz/filter` | master | false | 200 |
| POST | `/api/v1/application-authz/check` | application | false | 200 |
| POST | `/api/v1/application-authz/batch-check` | application | false | 200 |
| POST | `/api/v1/application-authz/filter` | application | false | 200 |
| GET/HEAD | `/healthz` | public | false | 200 |
| GET/HEAD | `/readyz` | public | false | 200/503 |

The inventory includes lifecycle routes listed below and granular action routes in the next section. Removed Exposure APIs, the V2 authority receiver, Grant IdP proxy paths, and the old provisioning verification route return 404 without aliases. Generic HTTP catalog/decide and private V1 authority migration remain supported.

`master` requires `x-grant-master-key`; `app` requires the distinct `x-grant-app-auth-key`. `application` additionally validates Bearer syntax and requires `x-organization-id`; the service binds the authenticated subject and application. `user` authenticates the selected organization and rejects a different organization in the URL. Writes use strong consistency. `inactive` uses the separately reviewed reactivation authentication path. Secret comparison hashes both operands and uses constant-time digest comparison.

Legacy JSON DTOs retain permissive unknown-field behavior where the Rust DTO did not declare `deny_unknown_fields`. Capability and authority import contracts are closed. Required fields cannot be omitted or null; duplicate object fields are rejected. Rust enum inputs are checked before service execution. Body limit is 2 MiB for JSON. Canonical signatures/hashes are computed by shared storage/domain functions rather than the HTTP layer.

All six authorization routes reject any present `resource.view` value, including null. Batch/filter requests permit at most 1,000 items. `filter` returns `resource_ids`; batch returns `results`. Generic HTTP capability requests validate the active catalog and exact contract before evaluation. An absent consent returns 404; successful deletion returns an empty 204 body. Domain errors retain the JSON `{ "error": "..." }` envelope.

Grant does not proxy IdP requests. `/auth`, `/auth/v1`, and `/auth/v1/*` return 404; route IdP traffic directly through the Gateway. External token validation remains enabled for the configured provider.

The raw SQL transport is a distinct private listener and has no public router registration. The native cluster member transport uses UDP QUIC and does not share the API port.

## Granular action mutations and preconditions

| Method | Path | Authentication | Voter only | Success |
|---|---|---|---|---|
| POST | `/api/v1/admin/providers/{providerId}/actions` | master or authorized platform bearer | true | 201 |
| PATCH | `/api/v1/admin/providers/{providerId}/actions/{actionId}` | master or authorized platform bearer | true | 200 |
| DELETE | `/api/v1/admin/providers/{providerId}/actions/{actionId}` | master or authorized platform bearer | true | 204 |
| GET | `/api/v1/orgs/{organizationId}/roles/{roleId}/actions` | user | true | 200 |
| PUT | `/api/v1/orgs/{organizationId}/roles/{roleId}/actions/{actionId}` | user | true | 204 |
| DELETE | `/api/v1/orgs/{organizationId}/roles/{roleId}/actions/{actionId}` | user | true | 204 |
| PUT | `/api/v1/orgs/{organizationId}/apps/{applicationId}/actions/{actionId}` | user | true | 204 |
| DELETE | `/api/v1/orgs/{organizationId}/apps/{applicationId}/actions/{actionId}` | user | true | 204 |

Action-set GETs and consent GET return a strong ETag. Replacement PUTs require
that ETag in `If-Match`; first consent creation requires `If-None-Match: *`.
Missing conditions return 428; stale conditions return 412 without changing
canonical rows. Link PUT/DELETE is bodyless and idempotent. PATCH changes only
supplied metadata; nullable text may be cleared with null. Immutable IDs/codes
and unknown fields are rejected. Full sets retain their replacement semantics.

The bootstrap route requires both master key and a configured-provider Bearer
with an explicit `X-Organization-ID`. It creates no Application. `/ops/*` and
bootstrap never accept platform-bearer fallback and are absent from the
management catalog. The old provider-specific bootstrap, admin backup/S3,
admin import and pep-clients paths return 404 without aliases.

## Lifecycle routes

| Method | Path | Authentication | Voter only | Success |
|---|---|---|---|---|
| PATCH | `/api/v1/admin/providers/{providerId}` | master | true | 200 |
| PUT | `/api/v1/admin/orgs/{organizationId}/apps/{applicationId}/identity-binding` | master | true | 200 |
| POST | `/api/v1/admin/orgs` | master | true | 201 |
| GET | `/api/v1/orgs` | bearer | false | 200 |
| GET | `/api/v1/orgs/{organizationId}` | user | false | 200 |
| PATCH | `/api/v1/orgs/{organizationId}` | user | true | 200 |
| PATCH | `/api/v1/orgs/{organizationId}/groups/{groupId}` | user | true | 200 |
| PATCH | `/api/v1/orgs/{organizationId}/agreements/{agreementId}` | user | true | 200 |
| PATCH | `/api/v1/orgs/{organizationId}/apps/{applicationId}` | user | true | 200 |
| PUT | `/api/v1/orgs/{organizationId}/apps/{applicationId}/active` | user | true | 200 |
| DELETE | `/api/v1/orgs/{organizationId}/apps/{applicationId}` | user | true | 204 |
| PATCH | `/api/v1/orgs/{organizationId}/roles/{roleId}` | user | true | 200 |
| DELETE | `/api/v1/orgs/{organizationId}/roles/{roleId}` | user | true | 204 |
| GET | `/api/v1/orgs/{organizationId}/members` | user | false | 200 |
| DELETE | `/api/v1/orgs/{organizationId}/members/{userId}` | user | true | 204 |
| DELETE | `/api/v1/orgs/{organizationId}/app-grants/{grantId}` | user | true | 204 |

In the table, `master` also supports an explicitly authorized platform bearer
for cataloged admin routes. Private operations and bootstrap do not. The
learner's global gate admits only health/readiness, runtime catalog reads and
HTTP decide; `Voter only=false` on a registration does not override that gate.

Provider POST is create-only and returns 409 on duplicate ID. Metadata PATCH
preserves omitted fields, clears nullable text with null, and rejects empty
patches, unknown fields and null for nonnullable fields. Existing group and
agreement PUTs remain full replacements. Lifecycle PATCH does not gain ETags;
concurrent writes to the same field use transaction order. Action-set and
consent replacement preconditions remain mandatory.

Identity binding accepts `{"clientId":"existing-client"}` and validates the
external client with the configured read-only Rauthy adapter. No client ID or
secret is issued by Grant. Unsupported adapters return 409 without fallback;
missing verification credentials return 503. Existing ready bindings remain
valid. See [lifecycle semantics](api-lifecycle.md) for deletion constraints,
owner creation, membership tombstones, consent revocation and rollout steps.

## Organizationless creation and ownership transfer

| Method | Path | Authentication | Voter only | Success |
|---|---|---|---|---|
| POST | `/api/v1/orgs` | bearer | true | 201 |
| GET | `/api/v1/orgs/{organizationId}/owner` | user | false | 200 |
| PUT | `/api/v1/orgs/{organizationId}/owner` | user | true | 200 |

Self-create accepts only `id` and `name`; the verified current user becomes the owner. Owner GET returns an ETag; transfer requires its exact `If-Match` and an `ownerUserId` naming another active member. Only the current owner with organization management permission may transfer. Missing conditions return 428; stale conditions return 412.

The exact four organizationless onboarding capabilities and trusted management-app prerequisites are defined in [the lifecycle contract](api-lifecycle.md). This is not anonymous access or a general removal of organization headers. Agreement classification is now service-defined display metadata: trimmed 1–128 UTF-8 characters, no control characters, unchanged status and interval authorization rules. Existing classification values and field names remain valid.
