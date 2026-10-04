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

Learner PDP readiness additionally requires its quorum fence, successful synchronization,
and consistent local schema reads; it reports `writeReady: false` because replicas cannot write.

## Configuration

Grant uses Rhiza v0.12.3 directly. The public HTTP API never exposes raw SQL. `GRANT_RUNTIME_MODE` selects embedded, remote, or split behavior; unsupported combinations fail startup instead of silently selecting a different consistency or topology.

`GRANT_NODE_ID` identifies the exact native member; `POD_NAME` is its Kubernetes fallback. `GRANT_CLUSTER_MEMBERS` is a closed native JSON member list with `node_id`, `peer_url` (`quic://host:port`) and member `token`. Every voter receives the same membership. `GRANT_PEER_ADDR` is the UDP bind address. Three voters start concurrently; the old Grant Hiqlite ordinal-zero lifecycle gate is not used. Rauthy's separate lifecycle remains unchanged.

Single-voter local development can use a filesystem object store. Multiple voters require the shared object-store configuration: `RHIZA_OBJSTORE_PROVIDER`, `ENDPOINT`, `BUCKET`, `PREFIX`, `REGION`, `ACCESS_KEY`, `SECRET_KEY`, and optionally `SESSION_TOKEN` (all prefixed `RHIZA_OBJSTORE_`). The full native configuration can be supplied through `GRANT_RHIZA_CONFIG_JSON`. Use `RHIZA_OBJSTORE_DURABILITY=before-ack` for the provided three-voter deployment. A fresh Rhiza prefix must not overlap a legacy Hiqlite backup prefix.

Remote SQL uses a separate private listener `GRANT_RUNTIME_BIND_ADDR` with `GRANT_REMOTE_API_SECRET`; remote clients use `GRANT_REMOTE_NODES`. The secret must be at least 32 characters and distinct from both public API keys. This listener is never mounted on the public HTTP router, service, or ingress. Expose it only on a trusted private network; terminate authenticated TLS for traffic crossing hosts. The Kubernetes optional private-runtime overlay restricts ingress to labelled runtime clients. Keep external SQL callers out of the public `grant` Service.

`GRANT_READ_CONSISTENCY` selects the configured read behavior. Lookup-only `GET /api/v1/me` accepts `consistency=strong`, `consistency=local`, and `consistency=prefer-strong`; other public per-request overrides accept only `consistency=strong`; writes and authorization management guards use the strong path. Learners reject voter-only management endpoints with 503.

See [the compatibility inventory](porting-go-compatibility.md) for the exact supported environment mapping and [the migration guide](backup-restore.md) for existing stores. The [old Hiqlite runtime guide](history/hiqlite/runtime-modes.md) is historical documentation, not a native Rhiza configuration recipe.

The isolated `scripts/verify-go-three-voter.sh` rehearsal verifies the deployment's
`emptyDir` recovery model: after forced process termination, one voter and then
all three voters recover the latest acknowledged state into fresh local directories
from the same S3 archive, cluster identity and membership. It also verifies that
quorum loss keeps `/healthz` at 200 and closes strong `/readyz` with 503; the latter
may report `readReady: true` and `writeReady: false` because local reads still work.
This proof uses disposable local Versity Gateway with its POSIX backend. Production S3 permissions, network policy,
node scheduling and resource limits still require a separate isolated deployment
rehearsal before traffic cutover.

### Local authority PDP

A learner can serve the HTTP capability catalog and decision APIs as a colocated PDP. The data unit consists of an unchanged PocketBase backend, an Exposure sidecar, and one Grant learner holding a Rhiza authority replica. Exposure calls that local PDP; the backend does not call authorization APIs or evaluate policy. The Grant unit consists of the original voter and its Exposure sidecar, which calls the private voter directly for decisions and management requests; it needs no management learner. Learners publish only `/healthz`, `/readyz`, `/api/v2/http-capabilities/catalog/{providerId}` and `/api/v2/http-capabilities/decide`. Management writes, bootstrap, identity creation, auth cache persistence, and the Rauthy proxy remain voter-only.

Each catalog or decision request first obtains a quorum-backed applied slot from the voter's private `POST /read-fence`, synchronizes its replica, then reads locally. Every SQL result must report the same applied slot at or above that fence; a changed snapshot triggers at most three attempts within a three-second deadline. PocketBase/Rauthy verification occurs once, outside retries, and only existing scoped identities are accepted. A disconnected voter fence, failed synchronization, or unstable snapshot returns unavailable; local readiness or a recent sync timestamp alone never authorizes access. Decisions already issued retain their existing short expiration.

Configure voters with a private `GRANT_RUNTIME_BIND_ADDR`, a distinct write-capable `GRANT_REMOTE_API_SECRET`, and a distinct `GRANT_READ_FENCE_KEY` (at least 32 bytes). Learners receive only the fence key and `GRANT_REMOTE_NODES` pointing to that private listener; they must not receive runtime write credentials, voter member tokens, or a runtime listener. The Rhiza `AdminToken` used by learners is a separate read-sync credential: the peer transport accepts it only for certified sync operations. Do not expose Rhiza's native HTTP handler with that key.

Use `scripts/replica-members` with private stdin JSON containing `ClusterID` and native `Members` to derive the public member array. Its output omits voter tokens. Set that array and the sync credential in `GRANT_RHIZA_REPLICA_CONFIG_JSON`; use `GRANT_LOCAL_NODE_ROLE=learner`, unique local data directories/replica IDs, and the same fixed cluster and object-store configuration. Keep `GRANT_CLUSTER_MEMBERS` containing voter tokens only on voters. The helper is a local setup tool, not a network provisioning service.

The voter uses the management application's fixed PocketBase application context; bootstrap creates that application. Create the separate data application through the organization application API, then explicitly configure its action scope, trust or consent, and PEP binding. The data learner uses that data application's context. This uses one fixed application context per service, without a provider-to-application routing map.

Management Exposure sends decisions and authorized requests directly to the private voter, never through itself. Private bootstrap and inactive-organization recovery remain direct administrative operations. Patch Auth remains the separate account service. Exposure's capability contract is generic so the same sidecar pattern can protect another PocketBase instance or another backend without adding authorization code to that backend.
