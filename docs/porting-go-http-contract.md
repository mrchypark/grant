# Go HTTP compatibility contract

Baseline: `c716877` (same application tree as `ee5852b`).

The HTTP boundary retains all 51 original distinct paths. GET registrations also accept HEAD.

## Method, path, authorization and node role

| Method | Path | Authentication | Voter only | Success |
|---|---|---|---|---|
| POST | `/api/v1/admin/providers/{providerId}/http-capability-generations` | master | true | 201 |
| POST | `/api/v1/admin/providers/{providerId}/http-capability-active` | master | true | 200 |
| PUT | `/api/v1/admin/providers/{providerId}/pep-clients/{pepClientId}` | master | true | 200 |
| GET | `/api/v2/http-capabilities/catalog/{providerId}` | app | false | 200 |
| POST | `/api/v2/http-capabilities/decide` | application | false | 200 |
| POST | `/api/v1/admin/imports/patch-data-authority` | master | true | 200 |
| POST | `/api/v2/admin/imports/authority-snapshots` | master | true | 200 |
| GET | `/api/v1/me` | bearer | false | 200 |
| POST | `/api/v1/me/materialize` | bearer | true | 200 |
| GET | `/api/v1/providers` |  | false | 200 |
| POST | `/api/v1/admin/providers` | master | true | 201 |
| PUT | `/api/v1/admin/providers/{providerId}/actions` | master | true | 200 |
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
| POST | `/api/v1/admin/orgs/{organizationId}/apps/{applicationId}/provisioning/verify` | master | true | 200 |
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
| POST | `/api/v1/admin/bootstrap/rauthy-owner` | master | true | 201 |
| POST | `/api/v1/admin/bootstrap/external-owner` | master | true | 201 |
| POST | `/api/v1/admin/backups` | master | true | 202 |
| GET | `/api/v1/admin/backups` | master | true | 200 |
| GET | `/api/v1/admin/backups/s3` | master | true | 200 |
| POST | `/api/v1/authz/check` | master | false | 200 |
| POST | `/api/v1/authz/batch-check` | master | false | 200 |
| POST | `/api/v1/authz/filter` | master | false | 200 |
| POST | `/api/v1/application-authz/check` | application | false | 200 |
| POST | `/api/v1/application-authz/batch-check` | application | false | 200 |
| POST | `/api/v1/application-authz/filter` | application | false | 200 |
| POST | `/api/v1/exposure-authz/read` | application | false | 200 |
| POST | `/api/v1/exposure-authz/plant-read` | application | false | 200 |
| POST | `/api/v2/exposure-authz/decide` | application | false | 200 |
| GET/HEAD | `/healthz` | public | false | 200 |
| GET/HEAD | `/readyz` | public | false | 200/503 |
| GET/HEAD | `/auth` | public | false | 307 |
| ANY | `/auth/v1` | Rauthy mode only | false | upstream status |
| ANY | `/auth/v1/{*rauthyPath}` | Rauthy mode only | false | upstream status |

There are 64 method registrations across 51 distinct paths. The exact frozen voter method inventory is `internal/api/testdata/routes-voter.txt`, extracted from the Rust baseline before source removal and checked against the Go router by `TestExactBaselineRouteMethods`.

`master` requires `x-grant-master-key`; `app` requires the distinct `x-grant-app-auth-key`. `application` additionally validates Bearer syntax and requires `x-organization-id`; the service binds the authenticated subject and application. `user` authenticates the selected organization and rejects a different organization in the URL. Writes use strong consistency. `inactive` uses the separately reviewed reactivation authentication path. Secret comparison hashes both operands and uses constant-time digest comparison.

Legacy JSON DTOs retain permissive unknown-field behavior where the Rust DTO did not declare `deny_unknown_fields`. Capability, authority import and Exposure operation contracts are closed. Required fields cannot be omitted or null; duplicate object fields are rejected. Rust enum inputs are checked before service execution. Body limit is 2 MiB for JSON and 10 MiB for Rauthy proxy requests. Canonical signatures/hashes are computed by shared storage/domain functions rather than the HTTP layer.

All six authorization routes reject any present `resource.view` value, including null. Batch/filter requests permit at most 1,000 items. `filter` returns `resource_ids`; batch returns `results`. Unknown operation IDs and mismatched frozen operation contracts fail before evaluation. An absent consent returns 404; successful deletion returns an empty 204 body. Domain errors retain the JSON `{ "error": "..." }` envelope.

The Rauthy proxy preserves the original method, path/query and repeated response headers, does not follow redirects, and returns generic 502/504 errors without upstream transport details. Patch Data mode returns 404 for the proxy. `/auth` remains the 307 redirect to `/auth/v1`.

The raw SQL transport is a distinct private listener and has no public router registration. The native cluster member transport uses UDP QUIC and does not share the API port.
