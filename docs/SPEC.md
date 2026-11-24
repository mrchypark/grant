# Grant Specification

`grant` is a Go 1.27.4 multi-tenant authorization service acting as a Policy Decision Point (PDP) and Policy Administration Point (PAP), backed by [Rhiza v0.27.5](https://github.com/mrchypark/rhiza) native Go storage without cgo.

> This document is the unified specification derived from the `mrchypark/grant` Go codebase. Detailed references live in [architecture.md](architecture.md), [authorization-model.md](authorization-model.md), [database-schema.md](database-schema.md), [runtime-modes.md](runtime-modes.md), [api-lifecycle.md](api-lifecycle.md), [http-capabilities.md](http-capabilities.md), [management-api.md](management-api.md), [provider-user-authorization.md](provider-user-authorization.md), [admin-ui.md](admin-ui.md), and [backup-restore.md](backup-restore.md).

---

## 1. System Scope and Core Boundaries

1. **Authorization, Not Identity Provider**: `grant` does not issue user logins or act as an OIDC/OAuth2 IdP. External identity verification is delegated to an external IdP (targeting **`goauthy`**, standard OIDC JWKS/Introspection, or a configurable `external-check` endpoint via `GRANT_AUTH_PROVIDER`) or a Trusted Gateway.
2. **Tenant and Legal Entity Separation**: `organizations.id` is the canonical tenant and authorization key. Legal/business counterparty identifiers live separately in `parties` and `organization_party_links`.
3. **Coarse Allow/Deny Evaluation**: Public authorization APIs evaluate `(subject, action, provider_id, resource_type, resource_id)` and return binary allow/deny decisions. Requests containing `resource.view` (including `null`) are rejected with `400 Bad Request`.
4. **No External Resource Ownership or Unbounded List**: `grant` does not store external provider resources or FK-validate external `(resource_type, resource_id)` pairs. External services register their `provider_id` and `actions` through standard admin APIs and pass candidate resource IDs to `filter` endpoints.
5. **No Raw SQL Exposure**: The public HTTP API never exposes raw SQL. Remote SQL transport (`GRANT_RUNTIME_BIND_ADDR`) is an authenticated private listener protected by `GRANT_REMOTE_API_SECRET` (for voter writes/queries) or `GRANT_READ_FENCE_KEY` (for learner read-fence verification).

---

## 2. Runtime Architecture and Storage (`rhiza v0.27.5`)

`grant` uses Rhiza's consensus and replicated object-store architecture:
- **`voter`**: Consensus-voting node with full read, write, migration, and admin capabilities.
- **`learner`**: Non-voting read-only replica that syncs from voters/object store and serves only `/healthz`, `/readyz`, `/api/v2/http-capabilities/catalog/{providerId}`, `/api/v2/http-capabilities/decide`, `/api/v1/authz/{check,batch-check,filter}`, `/api/v1/application-authz/{check,batch-check,filter}`, and `/api/v2/providers/{providerId}/authz/{batch-check,filter}` (returning `503` for voter-only routes).

### 2.1 Binaries (`cmd/`)

| Binary | Purpose | Source |
|---|---|---|
| `grant` | HTTP server (`voter` or `learner`) and CLI migration/restore | [cmd/grant/main.go](../cmd/grant/main.go) |
| `grant-admin` | Standalone Admin UI server | [cmd/grant-admin/main.go](../cmd/grant-admin/main.go) |
| `grant-patch-data-manifest-digest` | Canonical SHA-256 `manifestDigest` tool for authority imports | [cmd/grant-patch-data-manifest-digest/main.go](../cmd/grant-patch-data-manifest-digest/main.go) |
| `grant-management-catalog` | Emits the management route registry / action catalog | [cmd/grant-management-catalog/main.go](../cmd/grant-management-catalog/main.go) |
| `patch-data-catalog` | Emits reference HTTP capability catalog / stage payload | [cmd/patch-data-catalog/main.go](../cmd/patch-data-catalog/main.go) |

### 2.2 Runtime Modes and Read Consistency

- **`GRANT_RUNTIME_MODE`**: `embedded` | `remote` | `split`
- **`GRANT_LOCAL_NODE_ROLE`**: `voter` | `learner`
- **`GRANT_READ_CONSISTENCY`**: `local` (default), `strong` (linearizable), `prefer-strong` (linearizable with fallback to `local` strictly on `rhiza.ErrNotReady`, `rhiza.ErrQuorumUnavailable`, or `"unavailable:"`). HTTP `?consistency=strong` is supported on read/authz routes.
- **Cluster Peer Identity (`rhiza v0.27.5`)**: `rhiza.Member` carries public `PublicKey` (base64); each voter's private `PeerToken` is loaded separately via `RHIZA_PEER_TOKEN` / `GRANT_RHIZA_CONFIG_JSON` and forbidden on learners.

---

## 3. Authentication & Security Keys

1. **Key Separation** ([internal/grant/config.go](../internal/grant/config.go)):
   - `GRANT_MASTER_KEY` (`x-grant-master-key`): Required (placeholder `"dev-only-master-key"` rejected). Protects `/api/v1/admin/*`, `/api/v1/authz/*`, and `/ops/*`.
   - `GRANT_APP_AUTH_KEY` (`x-grant-app-auth-key`): Required, must differ from `GRANT_MASTER_KEY`. Protects `/api/v1/application-authz/*`, `/api/v2/providers/{providerId}/authz/{batch-check,filter}`, `/api/v2/http-capabilities/catalog/{providerId}`, and `/api/v2/http-capabilities/decide`.
   - `GRANT_TRUSTED_GATEWAY_KEY` (`x-grant-gateway-key`): Optional (≥32 bytes, distinct from master/app keys). When enabled, an internal gateway that already authenticated the caller may pass `x-grant-gateway-key` + `x-grant-subject` (plus optional `x-grant-client-id`, `x-grant-email`, `x-grant-display-name`). Disabled (fail-closed) when unset.
   - `GRANT_REMOTE_API_SECRET` / `GRANT_READ_FENCE_KEY`: ≥32 bytes, protecting voter runtime transport and learner read-fence checks.
2. **IdP Verification Modes (`GRANT_AUTH_PROVIDER`)** ([internal/grant/auth_external.go](../internal/grant/auth_external.go), [internal/grant/auth_goauthy.go](../internal/grant/auth_goauthy.go), [internal/grant/auth_jwks.go](../internal/grant/auth_jwks.go)):
   - **`goauthy`**: Audience/resource/scope-enforced OAuth 2.0 introspection against `GRANT_GOAUTHY_ISSUER`.
   - **`jwks`** (or `oidc`/`rauthy` with `GRANT_OIDC_JWKS_URL`): Local JWT signature verification (`RS256`, `ES256`, `EdDSA`) against `GRANT_OIDC_JWKS_URL` validating `exp`, `nbf`, `iss` (`GRANT_OIDC_ISSUER`), `aud` (`GRANT_OIDC_AUDIENCE`), `sub`, and `client_id`/`azp`.
   - **`oidc` / `rauthy`**: Standard OAuth 2.0 Token Introspection + UserInfo with `auth_identity_cache`.
   - **`pocketbase`**: Online `POST /api/collections/{collection}/auth-refresh` against the configured PocketBase origin bound to `GRANT_POCKETBASE_APPLICATION_ID`.
   - **`external-check`**: Configurable HTTP verification endpoint (`GRANT_EXTERNAL_CHECK_URL`, `GRANT_EXTERNAL_CHECK_METHOD`) with dot-separated JSON response field mappings (`GRANT_EXTERNAL_CHECK_SUBJECT_PATH`, `EMAIL_PATH`, `EMAIL_VERIFIED_PATH`, `NAME_PATH`, `TOKEN_PATH`, `EXPECTED_FIELD`/`EXPECTED_VALUE`) bound to `GRANT_EXTERNAL_CHECK_APPLICATION_ID`.

---

## 4. Data Model & Migrations

Canonical schema ([store_schema.sql](../internal/grant/store_schema.sql), [store_indexes.sql](../internal/grant/store_indexes.sql), [store_schema.go](../internal/grant/store_schema.go)) applies 7 named Rhiza migrations, including:
- Base canonical tables and indexes (`v1`–`v3`)
- Qualified Goauthy identities (`v4`), independent PEP identities (`v5`), PEP authorization policy (`v6`)
- Authz lookup indexes (`v7`: `idx_organization_memberships_user_org_status`, `idx_group_memberships_member_group`, `idx_role_bindings_resource_lookup`, `idx_role_actions_action_role`)

---

## 5. Deployment Contract

- **Container** ([Dockerfile](../Dockerfile)): `golang:1.27.4-bookworm` builder (`CGO_ENABLED=0`), `debian:bookworm-slim` runtime (`USER 10001:10001`), ports `8080/tcp` and `8100/udp`.
- **Kubernetes**: 3-voter `StatefulSet` sharing an S3-compatible object store (`RHIZA_OBJSTORE_DURABILITY=before-ack`), with optional `learner` sidecars.
