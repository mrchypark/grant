# Native Rhiza runtime modes

## Rhiza v0.23.2 source integration

Current source pins runtime and replica-members helper to Rhiza v0.23.2.
Supplier SDK source `2dc60add1c4cf99a9ff1050bfc7ad586f29ae7e1` is distinct
from tested Grant runtime `1d8704781590c695196372c07e10af42143665f4`.
Full [CI 42208148947](https://github.com/mrchypark/grant/actions/runs/42208148947)
passed for that Grant SHA: `go-verification` and `local-bootstrap` succeeded,
including module checks, Go/race, three-provider recovery, native **BeforeAck
quorum/restore integration** and AMD64 container checks. Optional Exposure and
launcher publication jobs were skipped. No operational deployment or released
image changed; a later documentation-only commit is not the tested runtime SHA.

[Baseline artifact 13499975522](https://github.com/mrchypark/grant/actions/runs/42208148947/artifacts/13499975522)
has verified SHA256
`fec7e5cbab266c6ef4c34f45e891fc08967326382ca7303ee63cd6af62749f05`.
Both mandatory v0.9.0/v0.9.1 synthetic Local retained probes passed, covering
old CLI import, 31-table/schema preservation, new write and retained reopen.
Artifact provenance matches the expected immutable image/source/version and
Rhiza v0.21.2/v0.23.0 CGO-disabled linux/amd64 baselines; cleanup records removal
of owned containers, binaries and isolated registry credentials.

Separately, the supplier's final-main [Performance 42206666450](https://github.com/mrchypark/rhiza/actions/runs/42206666450)
passed; the owner also confirms general CI, six HA checks and deployment artifact
verification complete. Earlier candidate [42203758889](https://github.com/mrchypark/rhiza/actions/runs/42203758889)
remains failed: its three-voter Async/Versity S3 SIGKILL lane recorded two
background 250 ms fetch deadlines, including a surviving-peer read. Foreground
100,000-request/rowset/latency/final-tip checks passed, but its subsequent
BeforeAck checks did not run. The later PASS does not establish the surviving-peer
read's root cause; this remains performance follow-up, not evidence requiring SDK
republication. Grant's BeforeAck integration does not qualify that Async fault
lane, exact-original UNKNOWN recovery, or arbitrary ACK/voter-loss/catch-up/latency
conditions. Do not generalize that Async/S3 result to other ACK modes or providers.
Neither qualification establishes arbitrary old non-Local compatibility,
downgrade, mixed-version peers, crash recovery, unpublished async ACKs or live-cloud parity.

## Historical Rhiza v0.23.1 source qualification

That source pinned both runtime and replica-members helper to Rhiza v0.23.1.
Full [CI 42182756781](https://github.com/mrchypark/grant/actions/runs/42182756781)
passed for exact runtime source `06fdcfb29a90cad460045a2fcfb58a2f299067cb`:
`go-verification` and `local-bootstrap` both succeeded, including module checks,
Go/race, three-provider recovery, native quorum/restore and AMD64 container checks.
Optional Exposure and launcher publication jobs were skipped, not qualified.
This source qualification changes no operational deployment or published image;
the following v0.9.1 receipt remains historical. Any later documentation-only
commit is distinct from the tested runtime SHA above.

[Baseline artifact 13487645854](https://github.com/mrchypark/grant/actions/runs/42182756781/artifacts/13487645854)
has verified download SHA256
`244fb77bca4622d9e926277c5b985d6bbe3238bcb176ec79fd382019df9b5631`.
Its immutable v0.9.0 and v0.9.1 images match their expected OCI source/version,
Rhiza v0.21.2 and v0.23.0 respectively, and CGO-disabled linux/amd64 build metadata.
Both required synthetic Local probes passed: old CLI import, candidate retained
Open, 31-table/schema preservation, new write and retained reopen. The artifact
records binary hashes and owned-container/binary/registry-credential cleanup.
These probes do not qualify arbitrary old non-Local data, downgrade, mixed-version
peers, crashes, unpublished async acknowledgements or production-cloud parity.

## Historical v0.9.1 / Rhiza v0.23.0 receipt

Grant v0.9.1 targets Rhiza v0.23.0 in both runtime and member helper. Full
[CI 42138972875](https://github.com/mrchypark/grant/actions/runs/42138972875)
passed for exact runtime source `8c3dd5d9ff252b033452ea107ec91c0e872ae929`,
including Go/race, native quorum/restore, production-security AMD64 containers
and local bootstrap. This documentation-only finalization is a separate revision:
record its commit SHA and the final release/image revision at publication.
Use the verified v0.9.1 image digest after publication, not an assumed available tag.

[Artifact 13468575056](https://github.com/mrchypark/grant/actions/runs/42138972875/artifacts/13468575056)
records the successful required synthetic v0.9.0 CLI-to-v0.23.0 Local retained-store
probe and cleanup. [Artifact 13467909402](https://github.com/mrchypark/grant/actions/runs/42138972875/artifacts/13467909402)
records successful S3/GCS/Azure clean-close ordinary cold Open with schema and
all 31 table counts/columns/digests preserved. Neither qualifies arbitrary old
non-Local data, mixed-version peers, downgrade, rollback, crash recovery,
unarchived async acknowledgements, partition fencing or live GCS deployment.

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

Grant v0.9.1 source selects Rhiza v0.23.0 and requires Go 1.27.3 or later.
The supplier confirms a self-contained CGO-disabled Go SDK. Dependency verification
does not establish old-state or archive compatibility: upgrade and rollback remain
unqualified beyond the narrow synthetic Local result above. Full CI and that
retained-directory probe passed; neither proves production non-Local migration.
The embedded Operator listener is integrated behind `RHIZA_BIND_ADDR`; the public
HTTP API never exposes raw SQL. `GRANT_RUNTIME_MODE` selects embedded, remote, or
split behavior; unsupported combinations fail startup rather than silently changing
consistency or topology. The following v0.21 sections describe historical contracts.

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
[Local storage contract](https://github.com/mrchypark/rhiza/blob/v0.21.2/docs/embedded-object-storage.md).

Filesystem Local recovery uses a process-lifetime owner lock on the same host,
supported on Linux and macOS only. Use a dedicated archive root outside `DataDir`;
the root and its parent must remain trusted and stable, without rename, replacement,
remount or external removal while in use. The advisory lock does not fence external
filesystem mutation. Remote maintenance tokens are not cleared based on age or
process state. Grant's default configuration does not enable `Local` or supervised
fencing. Existing lost-WAL voter safeguards still
apply; upgrading the module does not authorize deleting or reusing a voter directory.

The ordinary embedded configuration accepts Grant aliases for v0.9.1. The optional Operator deployment instead uses only native `RHIZA_*` values for generation-owned topology: `RHIZA_NODE_ID`, `RHIZA_DATA_DIR`, `RHIZA_CLUSTER_ID`, `RHIZA_CLUSTER_MEMBERS`, `RHIZA_PEER_TOKENS`, `RHIZA_ADMIN_TOKEN`, `RHIZA_PEER_ADDR`, `RHIZA_BIND_ADDR`, and `RHIZA_ENABLE_RECONFIGURATION`. In that mode, Grant rejects nonempty `GRANT_DATA_DIR`, `GRANT_NODE_ID`, `GRANT_CLUSTER_ID`, `GRANT_PEER_ADDR`, `GRANT_CLUSTER_MEMBERS`, `GRANT_RHIZA_CONFIG_JSON`, `GRANT_RHIZA_REPLICA_CONFIG_JSON`, `GRANT_REMOTE_NODES`, and `GRANT_CLUSTER_RESTORE_SNAPSHOT`. `GRANT_RUNTIME_MODE=embedded`, `GRANT_LOCAL_NODE_ROLE=voter`, `GRANT_RUNTIME_BIND_ADDR`, `GRANT_REMOTE_API_SECRET`, and `GRANT_READ_FENCE_KEY` remain allowed with their existing validation; the runtime secret and fence key must remain distinct from native Rhiza admin and peer credentials. Native `RHIZA_LEARNER` controls learner admission. Three native members start concurrently; the old Grant Hiqlite ordinal-zero lifecycle gate is not used. Rauthy's separate lifecycle remains unchanged.

Single-voter local development can use a filesystem object store. Multiple voters require the shared object-store configuration: `RHIZA_OBJSTORE_PROVIDER`, `ENDPOINT`, `BUCKET`, `PREFIX`, `REGION`, `ACCESS_KEY`, `SECRET_KEY`, and optionally `SESSION_TOKEN` (all prefixed `RHIZA_OBJSTORE_`). The full native configuration can be supplied through `GRANT_RHIZA_CONFIG_JSON`. Use `RHIZA_OBJSTORE_DURABILITY=before-ack` for the provided three-voter deployment. A fresh Rhiza prefix must not overlap a legacy Hiqlite backup prefix.

Remote SQL uses a separate private listener `GRANT_RUNTIME_BIND_ADDR` with `GRANT_REMOTE_API_SECRET`; remote clients use `GRANT_REMOTE_NODES`. The secret must be at least 32 characters and distinct from both public API keys. This listener is never mounted on the public HTTP router, service, or ingress. Expose it only on a trusted private network; terminate authenticated TLS for traffic crossing hosts. The Kubernetes optional private-runtime overlay restricts ingress to labelled runtime clients. Keep external SQL callers out of the public `grant` Service.

An embedded native voter may still enable this private runtime transport on TCP 8200. It requires the separate `private-runtime` Service and its `grant-runtime-client: "true"` NetworkPolicy rule; the Rhiza Operator overlay does not enable or publish TCP 8200 by default. Until a native learner completes its quorum-certified promotion, its private runtime handler also returns 503. The Patch Data passive learner architecture remains separate and unchanged.

`GRANT_READ_CONSISTENCY` selects the configured read behavior. Lookup-only `GET /api/v1/me` accepts `consistency=strong`, `consistency=local`, and `consistency=prefer-strong`; other public per-request overrides accept only `consistency=strong`; writes and authorization management guards use the strong path. Learners reject voter-only management endpoints with 503.

See [the compatibility inventory](porting-go-compatibility.md) for the exact supported environment mapping and [the migration guide](backup-restore.md) for existing stores. The [old Hiqlite runtime guide](history/hiqlite/runtime-modes.md) is historical documentation, not a native Rhiza configuration recipe.

The historical native three-voter fixture passes writes, follower strong reads and
same-directory recovery with the original WAL intact. For that registered topology, cold startup into an empty
directory under the same voter identity intentionally fails with `ErrVoterStateLost`.
The base `emptyDir` deployment cannot recover a replacement under that identity.
The optional Operator integration instead starts a fresh learner; the local fixture
verifies promotion, preserved data and new writes. This does not qualify production
fencing or Kubernetes whole-generation recovery.

For registered/enrolled non-Local recovery, the existing guide requires either a Rhiza Operator integration with trusted
external fencing or an approved persistent-storage strategy that retains each
voter's original disk and WAL. Lost-WAL replacement requires a fresh learner,
fencing and membership reconfiguration when quorum survives, or Operator-managed
whole-generation recovery. See the upstream [recovery contract](https://github.com/mrchypark/rhiza/blob/v0.21.2/docs/recovery.md)
and [embedded Operator integration](https://github.com/mrchypark/rhiza/blob/v0.21.2/docs/embedded-operator.md).
These historical HA paths do not establish requirements for every raw single voter.
For v0.23.0, the owner confirms raw Local empty-directory recovery from a complete
compatible archive without an Operator or PVC. The owner also confirms ordinary
Open into a new empty directory for a raw non-Local single voter with one fixed
member, no registration/enrollment/anchor, and a complete certified checkpoint
and tail. This exact raw topology needs neither an Operator nor PVC. Stop the old
writer and retain the exact identity and archive namespace; overlapping writers,
unarchived async acknowledgements, uncertain maintenance and anchored recovery
are outside that qualification. Grant's three-provider clean-close cold-DR tests
passed in the CI/artifact above; crash and live-cloud behavior remain separate.
The initial raw before-ack-to-async exception remains available before registration
or anchoring, excluding enrollment and prepared learners; see the
[exact runbook conditions](single-voter-learner.md#start-the-sole-voter-and-private-fence-transport).
GCS unresolved maintenance remains fail-closed; do not force-unlock, GC or rewrite
identity. Managed generation rules must not be generalized to the raw topology.
The isolated fixture uses disposable local Versity Gateway; production recovery
must be rehearsed before traffic cutover.

## Optional embedded Rhiza Operator

[`deploy/overlays/rhiza-operator`](../deploy/overlays/rhiza-operator) is an opt-in, no-PVC recovery controller overlay. It preserves the base `emptyDir` three-voter `StatefulSet`, sets `OnDelete`, and mounts a separate native configuration and credential contract. It does not change the default, IED, production, or private-runtime overlays. The rendered default namespace is `grant`; an IED composition must transform every namespaced resource together and provide namespace-local copies of the three required secrets. See [rhiza-operator.md](rhiza-operator.md) for the provisioning and recovery contract.

### Local authority PDP

A learner can serve the HTTP capability catalog and decision APIs as a colocated PDP. In the Exposure/PocketBase arrangement, Exposure calls that local PDP and protects an unchanged backend. Grant v0.9.1 also serves the existing learner application-authorization check/batch/filter routes and the two provider-bound USER batch/filter routes introduced in v0.9.0; see [provider-user authorization](provider-user-authorization.md). `/healthz` and `/readyz` remain available. Master `/api/v1/authz/*` and administration remain blocked on learners; there is no central runtime control-plane exception. Management writes, bootstrap, identity creation, auth cache persistence, and the Rauthy proxy remain voter-only.

Each catalog or decision request first obtains a quorum-backed applied slot from the voter's private `POST /read-fence`, synchronizes its replica, then reads locally. Every SQL result must report the same applied slot at or above that fence; a changed snapshot triggers at most three attempts within a three-second deadline. PocketBase/Rauthy verification occurs once, outside retries, and only existing scoped identities are accepted. A disconnected voter fence, failed synchronization, or unstable snapshot returns unavailable; local readiness or a recent sync timestamp alone never authorizes access. Decisions already issued retain their existing short expiration.

For generic application-authorization reads, external identity verification occurs once before the learner's three-second fence/catch-up/retry budget. The complete local identity mapping, active user, configured active/ready application and organization context, and every membership, RBAC, application-admission and consent evaluation use one accepted Grant applied slot across the entire request. A slot change retries the whole local operation, without repeating external verification or returning partial decisions. This detects snapshot movement; it does not lock the database. IdP verification is not part of a distributed snapshot. Voters retain their legacy individual strong reads for application-authorization routes; provider-bound USER reads instead require the fenced snapshot on both voters and learners.

The generic wire format and 1,000-item/2 MiB limits are unchanged. Application-authorization learners apply closed nested validation; provider-bound USER routes apply closed DTOs and duplicate-header rejection on both voters and learners. Filter accepts caller-supplied candidates and returns allowed IDs in input order; it provides no inventory, continuation token, authorized total, expiry lease or effect fence. Recheck protected operations after listing. See [the application HTTP contract](porting-go-http-contract.md#unreleased-learner-application-authorization) and [USER contract](provider-user-authorization.md) for headers, JSON and validation details. The current source engine is Rhiza v0.23.2.

Configured issuer, client, audience and scope checks remain trust boundaries. A different deployment issuer denotes a different qualified identity; Grant does not automatically alias subjects or merge accounts by email. This release adds no issuer-registration feature, OAuth-client provisioning or missing-account allow fallback. Existing local identity mapping is required; application readiness applies to application mode, not explicit provider-bound USER mode.

Generic learner application reads use uncached, nonwriting external verification. In particular, Rauthy bypasses both its memory cache and replicated identity cache, even with a positive cache TTL; revoked tokens and an unavailable IdP cannot fall back to a cached allow. Voter Rauthy caching remains unchanged. Goauthy and PocketBase retain online validation.

Configure voters with a private `GRANT_RUNTIME_BIND_ADDR`, a distinct write-capable `GRANT_REMOTE_API_SECRET`, and a distinct `GRANT_READ_FENCE_KEY` (at least 32 bytes). Learners receive only the fence key and `GRANT_REMOTE_NODES` pointing to that private listener; they must not receive runtime write credentials, voter member tokens, or a runtime listener. Configure Rhiza member `public_key` values and the local voter's `PeerToken` through `GRANT_RHIZA_CONFIG_JSON` or `RHIZA_PEER_TOKEN`; learners receive neither. The Rhiza `AdminToken` used by learners is a separate read-sync credential: the peer transport accepts it only for certified sync operations. Do not expose Rhiza's native HTTP handler with that key.

Use `scripts/replica-members` with private stdin JSON containing `ClusterID` and native `Members` to derive the public member array. Its output omits voter tokens. Set that array and the sync credential in `GRANT_RHIZA_REPLICA_CONFIG_JSON`; use `GRANT_LOCAL_NODE_ROLE=learner`, unique local data directories/replica IDs, and the same fixed cluster and object-store configuration. Keep `GRANT_CLUSTER_MEMBERS` containing voter tokens only on voters. The helper is a local setup tool, not a network provisioning service.

The v0.9.1 image includes `/usr/local/bin/replica-members`, built with Rhiza v0.23.0.
Current source pins its separate `scripts/replica-members-src` module to Rhiza
v0.23.2 like the runtime.
The supplier's [v0.19.0](https://github.com/mrchypark/rhiza/tree/v0.19.0) to
[v0.23.0](https://github.com/mrchypark/rhiza/tree/v0.23.0) source comparison confirms
unchanged key derivation without trimming, member conversion/URL fallback and
`ID`/`PeerURL`/32-number `PublicKey` JSON. This is source-confirmed compatibility,
not a cross-version golden-key runtime test. Transport/profile/ALPN changes remain
outside that claim: keep voter, learner and helper on the same version (v0.23.2
for current source), without
claiming rolling or mixed-version qualification. No Go toolchain is needed in the
runtime image. Invoke the helper explicitly with the verified published image:

```sh
docker run --rm -i --network none --read-only --cap-drop ALL \
  --security-opt no-new-privileges --user 10001:10001 \
  --entrypoint /usr/local/bin/replica-members "$GRANT_IMAGE" \
  < private-members.json > public-replica-members.json
```

Stdin is one JSON object using `ClusterID` and `Members`. Each member uses native Rhiza member keys; `token` is an optional private input for deriving the public key. For example, with synthetic values:

```json
{"ClusterID":"example-cluster","Members":[{"node_id":"n1","peer_url":"quic://n1:8100","log_url":"http://n1:8081/log","wal_identity":"wal-1","token":"example-private-token"}]}
```

Instead of `token`, a member can supply native `public_key` as a base64-encoded 32-byte public key; if both are supplied they must match. Stdout is the JSON array of `rhiza.ReplicaMember` values returned by `NewReplicaMember`, in input order, followed by a newline. Each output member has `ID`, `PeerURL`, and `PublicKey`; `PublicKey` is a JSON array of 32 byte values, not the base64 string used by native member input. Private tokens are excluded. Use that entire array as the replica configuration's `Members` value; it is not a complete replica configuration. Do not pass the private input to learners. Invalid input or conversion failure exits 1 with `invalid replica member input` on stderr. The helper does not contact voters, issue credentials, create identities, or generate other bootstrap settings.

In legacy application mode, provision the data application and its action scope,
trust/consent separately, then configure its fixed application context and PEP.
Owner bootstrap does not create an application. Explicit Goauthy USER mode instead
requires its configured provider-bound user PEP and exact RBAC, without application
admission or consent; it does not silently switch provider or mode.

Management Exposure sends decisions and authorized requests directly to the private voter, never through itself. Private bootstrap and inactive-organization recovery remain direct administrative operations. Patch Auth remains the separate account service. Exposure's capability contract is generic so the same sidecar pattern can protect another PocketBase instance or another backend without adding authorization code to that backend.
