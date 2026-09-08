# Native Rhiza runtime modes

## Health probes

`GET /healthz` checks only whether the HTTP process is alive and returns HTTP 200
with `{"status":"ok"}`. It does not contact storage or report database readiness.
Use it for startup and liveness probes so loss of quorum does not cause restart loops.

`GET /readyz` performs bounded database checks and retains the diagnostic fields
`status`, `readReady`, `writeReady`, `runtimeMode`, and `defaultReadConsistency`.
It returns HTTP 503 when the configured read consistency cannot be served. A
strong-read deployment requires a successful linearizable check; a local-read
deployment requires a successful local check. Use this endpoint for traffic readiness.

## Configuration

Grant uses Rhiza v0.12.3 directly. The public HTTP API never exposes raw SQL. `GRANT_RUNTIME_MODE` selects embedded, remote, or split behavior; unsupported combinations fail startup instead of silently selecting a different consistency or topology.

`GRANT_NODE_ID` identifies the exact native member; `POD_NAME` is its Kubernetes fallback. `GRANT_CLUSTER_MEMBERS` is a closed native JSON member list with `node_id`, `peer_url` (`quic://host:port`) and member `token`. Every voter receives the same membership. `GRANT_PEER_ADDR` is the UDP bind address. Three voters start concurrently; the old Grant Hiqlite ordinal-zero lifecycle gate is not used. Rauthy's separate lifecycle remains unchanged.

Single-voter local development can use a filesystem object store. Multiple voters require the shared object-store configuration: `RHIZA_OBJSTORE_PROVIDER`, `ENDPOINT`, `BUCKET`, `PREFIX`, `REGION`, `ACCESS_KEY`, `SECRET_KEY`, and optionally `SESSION_TOKEN` (all prefixed `RHIZA_OBJSTORE_`). The full native configuration can be supplied through `GRANT_RHIZA_CONFIG_JSON`. Use `RHIZA_OBJSTORE_DURABILITY=before-ack` for the provided three-voter deployment. A fresh Rhiza prefix must not overlap a legacy Hiqlite backup prefix.

Remote SQL uses a separate private listener `GRANT_RUNTIME_BIND_ADDR` with `GRANT_REMOTE_API_SECRET`; remote clients use `GRANT_REMOTE_NODES`. The secret must be at least 32 characters and distinct from both public API keys. This listener is never mounted on the public HTTP router, service, or ingress. Expose it only on a trusted private network; terminate authenticated TLS for traffic crossing hosts. The Kubernetes optional private-runtime overlay restricts ingress to labelled runtime clients. Keep external SQL callers out of the public `grant` Service.

`GRANT_READ_CONSISTENCY` selects the configured read behavior. Public per-request overrides accept only `consistency=strong`; writes and authorization management guards use the strong path. Learners reject voter-only management endpoints with 503.

See [the compatibility inventory](porting-go-compatibility.md) for the exact supported environment mapping and [the migration guide](backup-restore.md) for existing stores. The [old Hiqlite runtime guide](history/hiqlite/runtime-modes.md) is historical documentation, not a native Rhiza configuration recipe.

The isolated `scripts/verify-go-three-voter.sh` rehearsal verifies the deployment's
`emptyDir` recovery model: after forced process termination, one voter and then
all three voters recover the latest acknowledged state into fresh local directories
from the same S3 archive, cluster identity and membership. It also verifies that
quorum loss keeps `/healthz` at 200 and closes strong `/readyz` with 503; the latter
may report `readReady: true` and `writeReady: false` because local reads still work.
This proof uses disposable local MinIO. Production S3 permissions, network policy,
node scheduling and resource limits still require a separate isolated deployment
rehearsal before traffic cutover.
