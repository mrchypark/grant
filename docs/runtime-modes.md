# Native Rhiza runtime modes

Grant uses Rhiza v0.12.3 directly. The public HTTP API never exposes raw SQL. `GRANT_RUNTIME_MODE` selects embedded, remote, or split behavior; unsupported combinations fail startup instead of silently selecting a different consistency or topology.

`GRANT_NODE_ID` identifies the exact native member; `POD_NAME` is its Kubernetes fallback. `GRANT_CLUSTER_MEMBERS` is a closed native JSON member list with `node_id`, `peer_url` (`quic://host:port`) and member `token`. Every voter receives the same membership. `GRANT_PEER_ADDR` is the UDP bind address. Three voters start concurrently; the old Grant Hiqlite ordinal-zero lifecycle gate is not used. Rauthy's separate lifecycle remains unchanged.

Single-voter local development can use a filesystem object store. Multiple voters require the shared object-store configuration: `RHIZA_OBJSTORE_PROVIDER`, `ENDPOINT`, `BUCKET`, `PREFIX`, `REGION`, `ACCESS_KEY`, `SECRET_KEY`, and optionally `SESSION_TOKEN` (all prefixed `RHIZA_OBJSTORE_`). The full native configuration can be supplied through `GRANT_RHIZA_CONFIG_JSON`. Use `RHIZA_OBJSTORE_DURABILITY=before-ack` for the provided three-voter deployment. A fresh Rhiza prefix must not overlap a legacy Hiqlite backup prefix.

Remote SQL uses a separate private listener `GRANT_RUNTIME_BIND_ADDR` with `GRANT_REMOTE_API_SECRET`; remote clients use `GRANT_REMOTE_NODES`. The secret must be at least 32 characters and distinct from both public API keys. This listener is never mounted on the public HTTP router, service, or ingress. Expose it only on a trusted private network; terminate authenticated TLS for traffic crossing hosts. The Kubernetes optional private-runtime overlay restricts ingress to labelled runtime clients. Keep external SQL callers out of the public `grant` Service.

`GRANT_READ_CONSISTENCY` selects the configured read behavior. Public per-request overrides accept only `consistency=strong`; writes and authorization management guards use the strong path. Learners reject voter-only management endpoints with 503.

See [the compatibility inventory](porting-go-compatibility.md) for the exact supported environment mapping and [the migration guide](backup-restore.md) for existing stores. The [old Hiqlite runtime guide](history/hiqlite/runtime-modes.md) is historical documentation, not a native Rhiza configuration recipe.
