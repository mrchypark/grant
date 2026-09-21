# Go port compatibility inventory

## Frozen reference and boundary

The reference is merge commit `c716877` (its tree contains the standalone bootstrap fixes first staged as `ee5852b`). The original dirty `feature/grant-compos-company-strong` work is excluded: its unfinished authorization projection/signing work is not a port target. The retired Rust implementation remains available in the [immutable baseline](https://github.com/Conalog/grant/tree/c716877e7b970b7d38392b377f64384936992884). Current fixtures are stored under internal/api/testdata and internal/grant tests; runtime builds use Go only.

The reference has **51 distinct HTTP paths**, **60 router registrations** (30 shared and 30 voter registrations with 9 overlapping paths), **35 persistent CREATE TABLE definitions**, and **3 executable programs**. Learner routing additionally reserves management paths with the voter-required failure. The [method/path/auth inventory](porting-go-http-contract.md) records method-level details; counting paths alone is not evidence of semantic equivalence.

## Implementation ownership

- `internal/grant/models.go`, `service_*.go`, `auth_*.go`, `domain_*`: domain wire shapes, validated service operations, identity verification, authorization and evidence bindings.
- `internal/grant/store_*.go`, `config.go`, `runtime.go`: native embedded Rhiza, schema/legacy import, transactional authority, persisted catalogs, identity and consent writes, readiness and backup behavior.
- `internal/api`, `cmd`: method/path/status/wire extraction, reverse proxy, server entrypoint and exporter tools.

## HTTP paths in the reference

This section is historical: it records the original Rust baseline, not the current route inventory. The current API has 48 paths and 63 method registrations after retiring the three Exposure endpoints and the receipt-only V2 authority snapshot receiver, and adding three internal registration-management reads. The retired paths return 404; generic HTTP capabilities and V1 authority imports remain supported. See the current [HTTP contract](porting-go-http-contract.md) and [registration workflow](registration-api-management.md).

The following is extracted from [crates/api/src/lib.rs](https://github.com/Conalog/grant/blob/c716877e7b970b7d38392b377f64384936992884/crates/api/src/lib.rs); management/shared overlap is intentional. Learner writes must fail with HTTP 503 and `endpoint requires voter node`.

| Router | Path |
|---|---|
| voter | `/api/v1/me/materialize` |
| voter | `/api/v1/admin/providers` |
| voter | `/api/v1/admin/providers/{providerId}/actions` |
| voter | `/api/v1/admin/providers/{providerId}/http-capability-generations` |
| voter | `/api/v1/admin/providers/{providerId}/http-capability-active` |
| voter | `/ops/providers/{providerId}/enforcers/{pepClientId}` |
| voter | `/api/v1/orgs/{organizationId}/apps` |
| voter | `/api/v1/orgs/{organizationId}/apps/{applicationId}/actions` |
| voter | `/api/v1/orgs/{organizationId}/apps/{applicationId}/consent` |
| voter | `/api/v1/orgs/{organizationId}/groups` |
| voter | `/api/v1/orgs/{organizationId}/groups/{groupId}` |
| voter | `/api/v1/orgs/{organizationId}/groups/{groupId}/members/{userId}` |
| voter | `/api/v1/orgs/{organizationId}/members/{userId}` |
| voter | `/api/v1/admin/orgs/{organizationId}/apps/{applicationId}/trust` |
| voter | `/api/v1/admin/orgs/{organizationId}/apps/{applicationId}/provisioning/verify` |
| voter | `/api/v1/admin/users/{userId}/active` |
| voter | `/api/v1/orgs/{organizationId}/agreements` |
| voter | `/api/v1/orgs/{organizationId}/agreements/{agreementId}` |
| voter | `/api/v1/orgs/{organizationId}/roles` |
| voter | `/api/v1/orgs/{organizationId}/roles/{roleId}/actions` |
| voter | `/api/v1/orgs/{organizationId}/bindings` |
| voter | `/api/v1/orgs/{organizationId}/bindings/{bindingId}` |
| voter | `/api/v1/orgs/{organizationId}/deactivate` |
| voter | `/api/v1/orgs/{organizationId}/reactivate` |
| voter | `/ops/backups` |
| voter | `/api/v1/admin/bootstrap/owner` |
| voter | `/ops/migrations/patch-data-authority` |
| voter | `/api/v2/admin/imports/authority-snapshots` |
| shared | `/healthz` |
| shared | `/readyz` |
| shared | `/auth` |
| shared | `/auth/v1` |
| shared | `/auth/v1/{*rauthyPath}` |
| shared | `/api/v1/me` |
| shared | `/api/v1/providers` |
| shared | `/api/v2/http-capabilities/catalog/{providerId}` |
| shared | `/api/v2/http-capabilities/decide` |
| shared | `/api/v1/orgs/{organizationId}/apps` |
| shared | `/api/v1/orgs/{organizationId}/apps/{applicationId}` |
| shared | `/api/v1/orgs/{organizationId}/apps/{applicationId}/actions` |
| shared | `/api/v1/orgs/{organizationId}/app-grants` |
| shared | `/api/v1/orgs/{organizationId}/apps/{applicationId}/consent` |
| shared | `/api/v1/orgs/{organizationId}/groups` |
| shared | `/api/v1/orgs/{organizationId}/groups/{groupId}` |
| shared | `/api/v1/orgs/{organizationId}/groups/{groupId}/members` |
| shared | `/api/v1/orgs/{organizationId}/agreements` |
| shared | `/api/v1/orgs/{organizationId}/agreements/{agreementId}` |
| shared | `/api/v1/orgs/{organizationId}/roles` |
| shared | `/api/v1/orgs/{organizationId}/bindings` |
| shared | `/api/v1/authz/check` |
| shared | `/api/v1/authz/batch-check` |
| shared | `/api/v1/authz/filter` |
| shared | `/api/v1/application-authz/check` |
| shared | `/api/v1/application-authz/batch-check` |
| shared | `/api/v1/application-authz/filter` |
| shared | `/api/v1/exposure-authz/read` |
| shared | `/api/v1/exposure-authz/plant-read` |
| shared | `/api/v2/exposure-authz/decide` |

## Persistent schema inventory

Schema meaning, migration, constraints and relation changes must be documented in [database-schema.md](database-schema.md). This list does not replace that document. The temporary `actions_provider_scoped_migration` table is not a persistent table.

- `users`
- `user_identities`
- `auth_identity_cache`
- `parties`
- `organizations`
- `organization_party_links`
- `organization_memberships`
- `groups`
- `group_memberships`
- `agreements`
- `service_providers`
- `actions`
- `roles`
- `role_actions`
- `role_action_field_views`
- `field_view_policy_audit_outbox`
- `role_bindings`
- `applications`
- `application_redirect_uris`
- `application_actions`
- `application_grants`
- `application_grant_actions`
- `application_consent_assertions`
- `patch_data_authority_imports`
- `organization_hierarchy_edges`
- `patch_data_authority_import_entities`
- `provider_http_capability_sets`
- `provider_http_capability_active`
- `provider_http_capability_activations`
- `provider_pep_clients`
- `provider_pep_client_receipts`

## Executables

| Program | Reference | Compatibility obligations |
|---|---|---|
| `grant` | [crates/app/src/main.rs](https://github.com/Conalog/grant/blob/c716877e7b970b7d38392b377f64384936992884/crates/app/src/main.rs) | Validated configuration, native DB lifecycle, migration before serving, voter/learner separation, health/readiness, HTTP |
| `grant-patch-data-manifest-digest` | [crates/app/src/bin/grant-patch-data-manifest-digest.rs](https://github.com/Conalog/grant/blob/c716877e7b970b7d38392b377f64384936992884/crates/app/src/bin/grant-patch-data-manifest-digest.rs) | JSON stdin, strict shape, canonical digest excludes `manifestDigest`, lowercase digest stdout |

## Semantic obligations and evidence

| Boundary | Obligations | Reference evidence |
|---|---|---|
| Coarse deployment | Reject visibility-granting/field input; no field route/evaluator; reject populated legacy field/mask/audit state before migration without erasing it | [crates/infra/tests/coarse_deployment.rs](https://github.com/Conalog/grant/blob/c716877e7b970b7d38392b377f64384936992884/crates/infra/tests/coarse_deployment.rs); `crates/api/tests/field_view_{admin,authz}.rs` |
| Identity | Rauthy introspection plus matching userinfo subject/client; bounded token cache; inactive local user never reactivated by sync; PocketBase refresh verifies the pinned issuer and collection, and is not cached; local membership is authority | [crates/api/tests/rauthy_auth.rs](https://github.com/Conalog/grant/blob/c716877e7b970b7d38392b377f64384936992884/crates/api/tests/rauthy_auth.rs), `patch_data_auth.rs`, `admin_user_active.rs`; Go `auth_external_test.go` |
| Core authority | Provider-qualified actions and exact resource tuple; organization/group/user binding scope; agreement direction/lifecycle; no cross-tenant drift | [crates/api/tests/organization_authorization.rs](https://github.com/Conalog/grant/blob/c716877e7b970b7d38392b377f64384936992884/crates/api/tests/organization_authorization.rs); [crates/infra/tests/authz_activity.rs](https://github.com/Conalog/grant/blob/c716877e7b970b7d38392b377f64384936992884/crates/infra/tests/authz_activity.rs) |
| Applications | Reservation precedes external provisioning; Rauthy verify is read-only; explicit consent versus trusted application; transaction preconditions and stale-action cleanup | [crates/api/tests/application_routes.rs](https://github.com/Conalog/grant/blob/c716877e7b970b7d38392b377f64384936992884/crates/api/tests/application_routes.rs), `application_authz.rs`; [crates/infra/tests/application_update.rs](https://github.com/Conalog/grant/blob/c716877e7b970b7d38392b377f64384936992884/crates/infra/tests/application_update.rs) |
| Retired Exposure decisions (historical only) | Exact server-owned mapping; binary decision; five-second bound; length-framed principal digest; echo replay bindings without field data | [crates/api/tests/exposure_authz.rs](https://github.com/Conalog/grant/blob/c716877e7b970b7d38392b377f64384936992884/crates/api/tests/exposure_authz.rs); [crates/api/tests/fixtures/plant_read_contract.json](https://github.com/Conalog/grant/blob/c716877e7b970b7d38392b377f64384936992884/crates/api/tests/fixtures/plant_read_contract.json) |
| HTTP capabilities | Immutable staging; activation compare-and-swap and receipts; configured PEP identity; provider/application lifecycle checks; all request dimensions validated before decision | [crates/api/tests/http_capability_admin.rs](https://github.com/Conalog/grant/blob/c716877e7b970b7d38392b377f64384936992884/crates/api/tests/http_capability_admin.rs); [crates/infra/tests/http_capabilities.rs](https://github.com/Conalog/grant/blob/c716877e7b970b7d38392b377f64384936992884/crates/infra/tests/http_capabilities.rs); Go canonical evidence tests |
| V1 authority import | Length-framed canonical digest, exact source ownership, generation compare-and-swap, idempotent replay, atomic import beyond Rhiza's statement bound | [crates/infra/tests/patch_data_authority_import.rs](https://github.com/Conalog/grant/blob/c716877e7b970b7d38392b377f64384936992884/crates/infra/tests/patch_data_authority_import.rs); Go real-engine import rollback tests |
| Runtime and migration | No silent read-consistency downgrade; learner writes closed; legacy state preserved/exported through verified import; schema-aware readiness | [crates/infra/tests/runtime_routing.rs](https://github.com/Conalog/grant/blob/c716877e7b970b7d38392b377f64384936992884/crates/infra/tests/runtime_routing.rs), `database_schema.rs`; native Rhiza tests |

JSON integer generations and revisions must remain exact `uint64`/`int64` (or `json.Number`), not float64. Null, omitted optional fields, empty arrays, acronym casing and canonical collection ordering are separate contracts. `domain_contract_test.go` uses explicit expected wire JSON and the frozen Rust negative vectors; it does not regenerate expected answers from the new implementation.

## Verification status

This inventory is a porting checklist, not a claim that all reference behavior has passed. Full package tests, HTTP contract tests, exporter fixtures, race checks, legacy-data migration verification and deployment checks must pass before removing Rust or treating the Go branch as complete. Native Rhiza replaces Hiqlite; any unsupported legacy transport, runtime or backup configuration must fail explicitly and be documented, never silently substitute weaker semantics.
