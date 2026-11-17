# `internal/grant/` — Core Domain, Authorization, and Rhiza Store

`internal/grant` implements Grant's authorization engine, external identity verification, lifecycle management, and replicated SQLite storage on top of `rhiza v0.25.0`.

## Key Areas

- **Configuration & Runtime Store**: [config.go](config.go), [runtime.go](runtime.go), [store_remote.go](store_remote.go), [service_replica.go](service_replica.go)
  - Supports `voter` and `learner` node roles (`localReplicaRead`, `capabilityAuthorizationRead`, `readFence` with `1s` per-voter failover, `readScope` snapshot pinning).
  - Supports `local`, `remote`, and `split` runtime modes with `ReadPreferStrong` fallback on `503` availability errors.
- **Schema & Indexes**: [store_schema.sql](store_schema.sql), [store_indexes.sql](store_indexes.sql), [store_schema.go](store_schema.go), [store_bootstrap.go](store_bootstrap.go)
  - Any schema or index change must be documented in [docs/database-schema.md](../../docs/database-schema.md).
- **Authorization & Reverse Lookup**: [store_authz.sql](store_authz.sql), [store_lookup_resources.sql](store_lookup_resources.sql), [store_authz.go](store_authz.go), [service_core.go](service_core.go), [service_lookup_resources.go](service_lookup_resources.go), [service_user_authz.go](service_user_authz.go), [service_decisions.go](service_decisions.go)
  - `EvaluateUserActions` batches checks in 90-row `VALUES` chunks.
  - `LookupResources` performs bounded, cursor-paginated reverse lookup (`limit` `1..1000`, default `100`) using `idx_role_bindings_active_lookup`.
- **External Identity Verification**: [auth_external.go](auth_external.go), [auth_jwks.go](auth_jwks.go), [auth_goauthy.go](auth_goauthy.go), [auth_cache.go](auth_cache.go)
  - Supports `jwks`, `rauthy`, `oidc`, `external-check`, `pocketbase`, and `goauthy` modes with bounded (`4096`) identity caching and `refreshMu`-serialized JWKS updates.
- **Backups & Authority Imports**: [store_backup.go](store_backup.go), [store_authority.go](store_authority.go)
