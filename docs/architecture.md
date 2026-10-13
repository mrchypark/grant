# Grant architecture

Grant is a Go 1.27 service with a concrete `Service` and `Store` in `internal/grant`. `internal/api` owns the HTTP boundary. Four `cmd` packages provide the `grant` server and migration commands, the canonical Patch Data manifest digest tool, the management catalog generator, and the Patch Data catalog generator. Storage calls Rhiza v0.19.0's native Go API and does not require cgo.

The application preserves the organization, identity, membership, provider, action, role-binding, application-consent, authority-import and HTTP-capability contracts from baseline `c716877`. The [HTTP inventory](porting-go-http-contract.md) records method/path/authentication boundaries. All public authorization endpoints retain coarse allow/deny semantics and reject the presence of `resource.view`, including null. Historical field-view rows remain guarded by fail-closed startup validation.

Canonical mutations use native atomic execution with statement expectations. Bulk authority imports retain a single transaction even when entity count exceeds the native statement-count limit. Shared canonical hashing functions are used by the API and the manifest and catalog tools. Imported authority ownership and generation/replay checks must remain intact across migration.

Each native voter has a local materialized store and participates in the shared durable object-store topology. Strong reads use the native linearizable path. Remote SQL is an authenticated separate private listener; public API credentials cannot authorize it. Kubernetes peers use UDP QUIC and fixed member identities.

Learner reads retain a fresh per-request quorum fence: the learner catches up to at least that fence, and each decision uses one pinned applied snapshot. Fence acquisition or catch-up failure, or a snapshot that remains unstable after bounded retries, fails closed. The local performance comparison did not establish a consistent benefit from removing this freshness guarantee; an unfenced stale snapshot can still allow an action after its permission is revoked.

See [runtime modes](runtime-modes.md), [migration and recovery](backup-restore.md), and [schema documentation](database-schema.md). The previous [Rust/Hiqlite architecture](history/hiqlite/architecture.md) is retained only as migration reference.
