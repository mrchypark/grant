# Go HTTP compatibility contract

Baseline: `c716877` (same application tree as `ee5852b`).

The executable route inventory is `internal/api/testdata/routes-voter.txt` (88 routes; 71 management capabilities). GET also accepts HEAD. This is a code contract, not evidence that IED has received the new management catalog.

## Method, path, authorization and node role

| Method | Path | Authentication | Voter only | Success |
|---|---|---|---|---|
| POST | `/api/v1/admin/providers/{providerId}/http-capability-generations` | master | true | 201 |
| GET | `/api/v1/admin/providers/{providerId}/http-capability-generations/{generation}` | master or authorized platform bearer (`admin.catalog.read`) | true | 200 |
| POST | `/api/v1/admin/providers/{providerId}/http-capability-active` | master | true | 200 |
| GET | `/api/v1/admin/providers/{providerId}/http-capability-active` | master | true | 200 |
| PUT | `/ops/providers/{providerId}/enforcers/{pepClientId}` | master | true | 200 |
| GET | `/api/v2/http-capabilities/catalog/{providerId}` | app | false | 200 |
| POST | `/api/v2/http-capabilities/decide` | application | false | 200 |
| POST | `/ops/migrations/patch-data-authority` | master | true | 200 |
| GET | `/api/v1/me` | bearer | false | 200 |
| POST | `/api/v1/me/ensure` | bearer | true | 200 |
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
| PUT | `/api/v1/admin/orgs/{organizationId}/apps/{applicationId}/actions/{actionId}` | master only; no bearer fallback | true | 204 |
| GET | `/api/v1/admin/users` | master or authorized platform bearer | true | 200 |
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

Current learner routing rejects both `POST /api/v1/me/ensure` and `GET /api/v1/me` with HTTP 503 and `read replica only serves capability decisions and catalogs`. The legacy `false` registration flag for `GET /api/v1/me` does not override this server-level restriction.

The unreleased candidate additionally admits exactly the three application-authorization POST routes described below. Published v0.4.1 rejects those routes on learners. The registration flags in the table do not independently establish learner availability; master `/api/v1/authz/*` and administration remain blocked.

`master` requires `x-grant-master-key`; `app` requires the distinct `x-grant-app-auth-key`. `application` additionally validates Bearer syntax and requires `x-organization-id`; the service binds the authenticated subject and application. `user` authenticates the selected organization and rejects a different organization in the URL. Writes use strong consistency. `inactive` uses the separately reviewed reactivation authentication path. Secret comparison hashes both operands and uses constant-time digest comparison.

Legacy JSON DTOs retain permissive unknown-field behavior where the Rust DTO did not declare `deny_unknown_fields`. Capability and authority import contracts are closed. Required fields cannot be omitted or null; duplicate object fields are rejected. Rust enum inputs are checked before service execution. Body limit is 2 MiB for JSON. Canonical signatures/hashes are computed by shared storage/domain functions rather than the HTTP layer.

All six authorization routes reject any present `resource.view` value, including null. Batch/filter requests permit at most 1,000 items. `filter` returns `resource_ids`; batch returns `results`. Generic HTTP capability requests validate the active catalog and exact contract before evaluation. An absent consent returns 404; successful deletion returns an empty 204 body. Domain errors retain the JSON `{ "error": "..." }` envelope.

### Unreleased learner application authorization

This candidate is not a released version or deployment qualification. It changes learner availability for exactly `POST /api/v1/application-authz/check`, `POST /api/v1/application-authz/batch-check` and `POST /api/v1/application-authz/filter`; voter wire schemas and decoding compatibility remain unchanged.

All three require these headers. The application key is server-held, deployment-global `GRANT_APP_AUTH_KEY`, not a per-application credential or an application selector. No key issuance/rotation API is added.

```http
Authorization: Bearer <user-access-token>
X-Grant-App-Auth-Key: <deployment-application-key>
X-Organization-ID: <grant-organization-id>
Content-Type: application/json
```

The following values are placeholders, not existing registrations. `workspace` is a flat resource convention; resource IDs are nonempty trimmed strings, not necessarily UUIDs. There is no implicit parent-resource inheritance.

Check request:

```json
{"action":"<registered-action-id>","resource":{"provider":"<provider-id>","type":"workspace","id":"<stable-workspace-id>"}}
```

Batch request wraps check objects in `{"checks":[...]}`. Filter request:

```json
{"action":"<registered-action-id>","resources":[{"provider":"<provider-id>","type":"workspace","id":"<stable-workspace-id>"}]}
```

Successful evaluations return HTTP 200. Check returns `{"allowed":true,"reason":"<decision-reason>"}` or a false decision; batch returns `{"results":[...]}` in input order. Filter returns `{"resource_ids":["<allowed-workspace-id>"]}` or `{"resource_ids":[]}`. It preserves allowed input order and can preserve duplicates. IDs are bare strings: use one provider/type namespace to avoid ambiguity. No subject or application override is accepted on learners; both are derived from authentication and configuration.

Limits remain 1,000 checks/candidates and a 2 MiB JSON body. Learner-only pre-evaluation validation rejects unknown nested/top-level fields, duplicate JSON keys, explicit null required values, malformed/trailing JSON, missing collections, blank action/resource tuple fields, any `resource.view`, and multiple Authorization, X-Grant-App-Auth-Key or X-Organization-ID headers. Explicit empty arrays remain valid but still require authentication and local application context. Voters retain legacy unknown-extension-field handling. Oversized bodies return 413, excess items 400, and closed-shape/required-field errors generally 422; malformed JSON, duplicate headers and unsupported view return 400. Authentication and context checks can return 401/403. Unavailable fences or unstable snapshots fail closed with 503; no partial successful result is returned. Treat all other non-200 or malformed responses as failures as well. Domain errors use `{"error":"..."}` with an optional `code`.

External identity verification occurs once before the learner fence budget. The complete Grant-local identity/user/application active-ready/org context and the whole batch's membership, RBAC, action admission and consent evaluation must share one accepted applied slot at or beyond the request's quorum fence. Snapshot movement retries the whole local operation, at most three attempts within the existing three-second learner budget. External verification is not repeated by those retries and is not pinned to Grant's slot. This is not an IdP/Grant distributed snapshot or a lock. Voters retain individual strong reads; no whole-batch slot guarantee is added there.

Filter is candidate-in/candidate-out, not allowed-resource enumeration: no continuation token, authorized total or full-scope snapshot is returned. The caller owns candidate inventory and pagination. Neither these responses nor the accepted slot provide an expiry lease or effect fence; recheck authorization for protected operations. Configured issuer/client/audience/scope validation and application readiness remain mandatory. No new issuer registry, automatic identity alias/email merge, OAuth client, or missing-account allow fallback is introduced.

For these learner reads, external verification is uncached and performs no cache writes. Rauthy bypasses both memory and replicated caches regardless of positive TTL; an inactive token returns 401 and an unavailable IdP fails closed without cached identity fallback. Voter Rauthy cache semantics are unchanged; Goauthy/PocketBase online validation is preserved.

Grant does not proxy IdP requests. `/auth`, `/auth/v1`, and `/auth/v1/*` return 404; route IdP traffic directly through the Gateway. External token validation remains enabled for the configured provider. Bodyless Bearer `POST /api/v1/me/ensure` creates a missing internal user and external-identity mapping or reuses the existing user without profile writes, and returns only `{ "userId": "..." }`; it grants no membership, role, Binding, or consent. `GET /api/v1/me` and general authentication lookup never create domain users or update profiles at any consistency level. A missing local mapping from `GET /api/v1/me` returns 404 with `LOCAL_USER_NOT_PROVISIONED`.

`GET /api/v1/me` supports `consistency=strong`, `local`, and `prefer-strong`. Consistency selects read freshness and never enables user or identity provisioning. Other routes retain their existing strong-only explicit consistency override. A `LOCAL_USER_NOT_PROVISIONED` 404 under `local` may reflect replica lag; use `GET /api/v1/me?consistency=strong` after ensure when an immediate current view is required.

The raw SQL transport is a distinct private listener and has no public router registration. The native cluster member transport uses UDP QUIC and does not share the API port.

## Admin user listing

`GET /api/v1/admin/users` requires the platform gate action `admin.users.read`
(`grant-management.admin.users.read`) on `grant-management/platform/grant`
for Bearer access; existing master-key access remains supported. The route is
bodyless and returns `200`. Optional `q` is a literal substring filter over
Grant User `id`, `email`, and `displayName`, not regex or wildcard syntax.
`limit` defaults to 50 and is capped at 100. Optional `cursor` provides bounded
pagination in stable Grant user ID ascending order, continuing strictly after
the cursor's user ID. Each page and its lookahead come from a single strong
consistent read. Only `q`, `limit`, `cursor`, and `consistency` are accepted,
without duplicates; malformed or invalid query values and bodies return `400`.
`consistency` cannot select a weaker read than `strong`.

The response is `{"items":[...],"nextCursor":"..."}`; `nextCursor` is omitted
on the final page. Each item uses the existing Grant User JSON fields `id`,
optional `email`, `displayName`, and `active`. Inactive/disabled Grant users
(`active: false`) are included. External subjects, identity mappings, credentials,
and secrets are excluded; this is not an identity-provider account listing.
See [management query and access policies](management-api.md#admin-user-listing).

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
learner's global gate admits health/readiness, runtime catalog reads and
HTTP decide, plus the unreleased candidate's exact three application-authz POST
routes above; `Voter only=false` on a registration does not override that gate.

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

## Operational first-party admission

The master-only application action PUT above accepts no body and adds only the
specified association, idempotently. Its transaction requires an active
organization, same-organization active/ready first-party auto-approve app, no
active security-mutation lease, and an application-grantable canonical action
belonging to an active provider. Failed state preconditions return 409. The
999-action limit permits retrying an existing association at the limit.
Unauthenticated and ordinary bearer requests return 401; learners return 503.
Other associations, consent grants, role permissions, trust and identity fields
are preserved. Tenant action PUTs still reject trusted applications with 403.
This operational route has no management capability or admin UI registry entry.
