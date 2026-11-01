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

Grant selects Rhiza v0.22.0 with latticedb-go v0.11.3 and requires Go 1.27.3 or later. The embedded Operator listener is integrated behind `RHIZA_BIND_ADDR`; production migration remains a separate fresh-cluster operational decision. The public HTTP API never exposes raw SQL. `GRANT_RUNTIME_MODE` selects embedded, remote, or split behavior; unsupported combinations fail startup instead of silently selecting a different consistency or topology.

### Rhiza v0.22 integration boundary

Source selects [v0.22.0](https://github.com/mrchypark/rhiza/releases/tag/v0.22.0),
commit `f2366c899c7829feccf3d631336420542a6ca7d8`. This is an isolated integration
candidate, not rollout qualification. The published v0.4.1 image still contains
v0.21.2. Exact-release upstream main CI, performance, publication, v1alpha2,
managed kubelet and provider checks have passed. Separate HA Kubernetes Chaos Mesh
[run 41271300293](https://github.com/mrchypark/rhiza/actions/runs/41271300293)
failed qualification; upstream is not all green, and Grant HA remains unqualified.

Grant continues to use the ordinary native API. It does not opt into
`PreparedStartup`, the private managed SQL reference host, or `ResolveExecute`.
Durability confirmation through `ResolveExecute` may publish existing data;
it is not a pure read or a canonical-state proof for Grant's fences.

The new managed HA cluster/recovery CRDs serve/store v1alpha2 only, with no
v1alpha1 conversion or migration. Grant's optional v1alpha1 overlay and v0.19.0
controller remain unchanged and unqualified with this runtime. The reference
host's prepared recovery, two init containers, no-PVC topology and append-only
storage guarantees do not apply to Grant's existing deployment. See
[the Operator boundary](rhiza-operator.md).

The Local archive format version remains 2, with no increment from v0.21.2.
Separate CGO=0 Grant test executables on Darwin/ARM64 demonstrated v0.21.2 to
v0.22.0 retained-data reopen and direct restoration of an untouched old filesystem
archive after clean Close, preserving authority and ownership revision. This
bounded evidence does not qualify abrupt crash recovery, rolling upgrade, mixed
versions, downgrade, cloud storage or the Linux/AMD64 image. Existing Local
ownership, before-ack durability, independent
archive namespace and fail-closed maintenance-token conditions remain mandatory.
Missing managed authority is an error, not permission to bootstrap over an archive.
For Local storage, a ClusterID change can instead select an empty separate archive
namespace on both tested versions; successful init alone does not prove recovery.
Preserve the expected cluster/namespace and verify recovered business authority.

### Rhiza v0.21 compatibility

The [v0.21.2 release](https://github.com/mrchypark/rhiza/releases/tag/v0.21.2)
retains the v0.21.0/v0.21.1 archive format; this patch needs no format migration. It
uses an archive format incompatible with pre-v0.21 archives. There is no automatic
migration, adoption, or deletion of old archives. For pre-v0.21 sources use a new, empty `DataDir` and a
new, independent archive directory or object-store namespace only. Preserve all
existing data and archives unchanged; do not bootstrap a sibling namespace inside
a rejected archive. Grant's SQL schema and canonical snapshot format are unchanged;
that does not establish physical storage or archive compatibility. Rolling upgrades,
mixed-version clusters, downgrade and in-place production migration are unverified.

v0.21.2 reconciles Local maintenance release errors only when stable remote
readback proves the exact expected free control at a changed object version.
Work errors, cancellation and uncertain operations remain fail-closed; the release
upload timeout is unchanged. It cannot clear an already orphaned held remote token.
There is no archive, identity, receipt or public API change. A free control permits
ordinary same-identity Open only after all existing validation succeeds. This Go
dependency update does not qualify Grant against live S3, GCS or Azure storage.

Explicit `GRANT_RHIZA_CONFIG_JSON` configuration with `Local: true` can use
Rhiza's automatic object-store backup and restore. This is separate from Grant's
canonical JSON snapshot export/import. Local archives require one writer, including
during offline restore; missing local data can restore before `Open` returns.
An empty durability policy defaults to before-ack in this mode, while an explicit
`async` policy permits acknowledged writes ahead of backup. Environment-derived
native configuration defaults to `async`, so select before-ack explicitly when needed.
Checkpoint retention defaults to three snapshots, with a 15-minute checkpoint
interval, hourly GC, and a 24-hour GC grace period. The fixed archive HEAD catalog
rejects unsupported formats rather than upgrading or adopting them. Divergent or unverifiable histories fail
startup while preserving evidence. See the upstream
[Local storage contract](https://github.com/mrchypark/rhiza/blob/v0.22.0/docs/embedded-object-storage.md).

Filesystem Local recovery uses a process-lifetime owner lock on the same host,
supported on Linux and macOS only. Use a dedicated archive root outside `DataDir`;
the root and its parent must remain trusted and stable, without rename, replacement,
remount or external removal while in use. The advisory lock does not fence external
filesystem mutation. Remote maintenance tokens are not cleared based on age or
process state. Grant's default configuration does not enable `Local` or supervised
fencing. Existing lost-WAL voter safeguards still
apply; upgrading the module does not authorize deleting or reusing a voter directory.

The ordinary embedded configuration accepts Grant aliases for the current candidate. The optional Operator deployment instead uses only native `RHIZA_*` values for generation-owned topology: `RHIZA_NODE_ID`, `RHIZA_DATA_DIR`, `RHIZA_CLUSTER_ID`, `RHIZA_CLUSTER_MEMBERS`, `RHIZA_PEER_TOKENS`, `RHIZA_ADMIN_TOKEN`, `RHIZA_PEER_ADDR`, `RHIZA_BIND_ADDR`, and `RHIZA_ENABLE_RECONFIGURATION`. In that mode, Grant rejects nonempty `GRANT_DATA_DIR`, `GRANT_NODE_ID`, `GRANT_CLUSTER_ID`, `GRANT_PEER_ADDR`, `GRANT_CLUSTER_MEMBERS`, `GRANT_RHIZA_CONFIG_JSON`, `GRANT_RHIZA_REPLICA_CONFIG_JSON`, `GRANT_REMOTE_NODES`, and `GRANT_CLUSTER_RESTORE_SNAPSHOT`. `GRANT_RUNTIME_MODE=embedded`, `GRANT_LOCAL_NODE_ROLE=voter`, `GRANT_RUNTIME_BIND_ADDR`, `GRANT_REMOTE_API_SECRET`, and `GRANT_READ_FENCE_KEY` remain allowed with their existing validation; the runtime secret and fence key must remain distinct from native Rhiza admin and peer credentials. Native `RHIZA_LEARNER` controls learner admission. Three native members start concurrently; the old Grant Hiqlite ordinal-zero lifecycle gate is not used. Rauthy's separate lifecycle remains unchanged.

Single-voter local development can use a filesystem object store. Multiple voters require the shared object-store configuration: `RHIZA_OBJSTORE_PROVIDER`, `ENDPOINT`, `BUCKET`, `PREFIX`, `REGION`, `ACCESS_KEY`, `SECRET_KEY`, and optionally `SESSION_TOKEN` (all prefixed `RHIZA_OBJSTORE_`). The full native configuration can be supplied through `GRANT_RHIZA_CONFIG_JSON`. Use `RHIZA_OBJSTORE_DURABILITY=before-ack` for the provided three-voter deployment. A fresh Rhiza prefix must not overlap a legacy Hiqlite backup prefix.

Remote SQL uses a separate private listener `GRANT_RUNTIME_BIND_ADDR` with `GRANT_REMOTE_API_SECRET`; remote clients use `GRANT_REMOTE_NODES`. The secret must be at least 32 characters and distinct from both public API keys. This listener is never mounted on the public HTTP router, service, or ingress. Expose it only on a trusted private network; terminate authenticated TLS for traffic crossing hosts. The Kubernetes optional private-runtime overlay restricts ingress to labelled runtime clients. Keep external SQL callers out of the public `grant` Service.

An embedded native voter may still enable this private runtime transport on TCP 8200. It requires the separate `private-runtime` Service and its `grant-runtime-client: "true"` NetworkPolicy rule; the Rhiza Operator overlay does not enable or publish TCP 8200 by default. Until a native learner completes its quorum-certified promotion, its private runtime handler also returns 503. The Patch Data passive learner architecture remains separate and unchanged.

`GRANT_READ_CONSISTENCY` selects the configured read behavior. Lookup-only `GET /api/v1/me` accepts `consistency=strong`, `consistency=local`, and `consistency=prefer-strong`; other public per-request overrides accept only `consistency=strong`; writes and authorization management guards use the strong path. Learners reject voter-only management endpoints with 503.

See [the compatibility inventory](porting-go-compatibility.md) for the exact supported environment mapping and [the migration guide](backup-restore.md) for existing stores. The [old Hiqlite runtime guide](history/hiqlite/runtime-modes.md) is historical documentation, not a native Rhiza configuration recipe.

The corrected native three-voter fixture passes writes, follower strong reads and
same-directory recovery with the original WAL intact. Cold startup into an empty
directory under the same voter identity intentionally fails with `ErrVoterStateLost`.
The base `emptyDir` deployment cannot recover a replacement under that identity.
The optional Operator integration instead starts a fresh learner; the local fixture
verifies promotion, preserved data and new writes. This does not qualify production
fencing or Kubernetes whole-generation recovery.

Production migration requires either a Rhiza Operator integration with trusted
external fencing or an approved persistent-storage strategy that retains each
voter's original disk and WAL. Lost-WAL replacement requires a fresh learner,
fencing and membership reconfiguration when quorum survives, or Operator-managed
whole-generation recovery. See the upstream [recovery contract](https://github.com/mrchypark/rhiza/blob/v0.21.2/docs/recovery.md)
and [embedded Operator integration](https://github.com/mrchypark/rhiza/blob/v0.21.2/docs/embedded-operator.md).
The isolated fixture uses disposable local Versity Gateway; production recovery
must be rehearsed before traffic cutover.

## Optional embedded Rhiza Operator

[`deploy/overlays/rhiza-operator`](../deploy/overlays/rhiza-operator) is an opt-in, no-PVC recovery controller overlay. It preserves the base `emptyDir` three-voter `StatefulSet`, sets `OnDelete`, and mounts a separate native configuration and credential contract. It does not change the default, IED, production, or private-runtime overlays. The rendered default namespace is `grant`; an IED composition must transform every namespaced resource together and provide namespace-local copies of the three required secrets. See [rhiza-operator.md](rhiza-operator.md) for the provisioning and recovery contract.

### Local authority PDP

A learner can serve the HTTP capability catalog and decision APIs as a colocated PDP. The data unit consists of an unchanged PocketBase backend, an Exposure sidecar, and one Grant learner holding a Rhiza authority replica. Exposure calls that local PDP; the backend does not call authorization APIs or evaluate policy. The Grant unit consists of the original voter and its Exposure sidecar, which calls the private voter directly for decisions and management requests; it needs no management learner. Learners publish only `/healthz`, `/readyz`, `/api/v2/http-capabilities/catalog/{providerId}` and `/api/v2/http-capabilities/decide`. Management writes, bootstrap, identity creation, auth cache persistence, and the Rauthy proxy remain voter-only.

Each catalog or decision request first obtains a quorum-backed applied slot from the voter's private `POST /read-fence`, synchronizes its replica, then reads locally. Every SQL result must report the same applied slot at or above that fence; a changed snapshot triggers at most three attempts within a three-second deadline. PocketBase/Rauthy verification occurs once, outside retries, and only existing scoped identities are accepted. A disconnected voter fence, failed synchronization, or unstable snapshot returns unavailable; local readiness or a recent sync timestamp alone never authorizes access. Decisions already issued retain their existing short expiration.

Configure voters with a private `GRANT_RUNTIME_BIND_ADDR`, a distinct write-capable `GRANT_REMOTE_API_SECRET`, and a distinct `GRANT_READ_FENCE_KEY` (at least 32 bytes). Learners receive only the fence key and `GRANT_REMOTE_NODES` pointing to that private listener; they must not receive runtime write credentials, voter member tokens, or a runtime listener. Configure Rhiza member `public_key` values and the local voter's `PeerToken` through `GRANT_RHIZA_CONFIG_JSON` or `RHIZA_PEER_TOKEN`; learners receive neither. The Rhiza `AdminToken` used by learners is a separate read-sync credential: the peer transport accepts it only for certified sync operations. Do not expose Rhiza's native HTTP handler with that key.

Use `scripts/replica-members` with private stdin JSON containing `ClusterID` and native `Members` to derive the public member array. Its output omits voter tokens. Set that array and the sync credential in `GRANT_RHIZA_REPLICA_CONFIG_JSON`; use `GRANT_LOCAL_NODE_ROLE=learner`, unique local data directories/replica IDs, and the same fixed cluster and object-store configuration. Keep `GRANT_CLUSTER_MEMBERS` containing voter tokens only on voters. The helper is a local setup tool, not a network provisioning service.

The voter uses the management application's fixed PocketBase application context; bootstrap creates that application. Create the separate data application through the organization application API, then explicitly configure its action scope, trust or consent, and PEP binding. The data learner uses that data application's context. This uses one fixed application context per service, without a provider-to-application routing map.

Management Exposure sends decisions and authorized requests directly to the private voter, never through itself. Private bootstrap and inactive-organization recovery remain direct administrative operations. Patch Auth remains the separate account service. Exposure's capability contract is generic so the same sidecar pattern can protect another PocketBase instance or another backend without adding authorization code to that backend.
