# One voter and a passive learner

This runbook describes an **unreleased candidate**, pinned to Rhiza v0.21.2.
Candidate CI and actual GCS-backed learner acceptance are pending. It is not a
claim of published v0.8.0 support or live deployment success. The parent release
record must identify the final image digest and verification evidence before use.
The [candidate admission checks](../internal/grant/store_single_voter_restore.go)
and [logical importer](../internal/grant/store_backup.go) define the behavior below.

The target has one authoritative non-Local Grant voter and a read-only passive
learner beside the provider's Exposure/backend. Consumers call that learner;
they do not substitute a direct voter call. This is not HA, an Operator setup,
or a PVC/WAL-loss recovery procedure. The learner is an authority replica, not a
second writer or a promotion learner.

## Artifacts and source freeze

Use a producer-published image containing this candidate restore implementation,
`/usr/local/bin/grant`, and `/usr/local/bin/replica-members`. Pin its image digest
and source provenance. The v0.8.0 image has the member helper but not this new
single-voter cloud-import admission. Do not build supplier source on the consumer
host or use `go run` as a setup step.

Freeze source writers and obtain the existing canonical logical snapshot through
the source's supported backup path. Keep its original bytes, full file checksum,
snapshot `digest`, schema digest, source cluster ID, 31-table inventory, counts
and table digests. The file checksum and snapshot digest are different checks.
Mount the snapshot read-only for import. This is not a Rhiza archive conversion:
never relabel a Local archive as non-Local or reuse its prefix for the new target.

Reserve a fresh target cluster ID, one fixed voter ID, a stable advertised QUIC
address and a new dedicated GCS prefix. Stop every other actor that could write
that target. Namespace listing proves observed vacancy, **not a distributed lock**.
Neither listing nor the local pending marker excludes another process.

## Voter configuration for the offline importer

All examples are templates: replace placeholders through the operator's normal
secret/configuration delivery, without printing credentials. The following are
Grant's actual configuration names, not a new provisioning API.

| Variable | Import value |
|----------|--------------|
| `GRANT_RUNTIME_MODE` | `embedded` |
| `GRANT_LOCAL_NODE_ROLE` | `voter` |
| `GRANT_CLUSTER_ID` | New target cluster ID, different from the snapshot's. |
| `GRANT_NODE_ID` | Fixed voter ID matching the sole member. |
| `GRANT_DATA_DIR` | New absent child directory, e.g. `/data/rhiza/imported`. |
| `GRANT_PEER_ADDR` | Fixed UDP bind address, e.g. `0.0.0.0:8100`. |
| `GRANT_CLUSTER_MEMBERS` | The one-member native array described below. |
| `GRANT_RHIZA_CONFIG_JSON` | Non-Local config with a separate `AdminToken`; omit service-account JSON when using ADC. |
| `RHIZA_OBJSTORE_PROVIDER` | `gcs` |
| `RHIZA_OBJSTORE_BUCKET` | Existing operator-provisioned target bucket. |
| `RHIZA_OBJSTORE_PREFIX` | Fresh canonical prefix, e.g. `candidate-import/target-01`. |
| `RHIZA_OBJSTORE_DURABILITY` | `before-ack` explicitly for the offline seed only. |

Keep normal Grant security/Goauthy configuration valid; see
[provider-user authorization](provider-user-authorization.md#caller-and-authority).
The import command still loads `ConfigFromEnv`. Its required Goauthy application
ID configuration string does not require creating an application row for user mode.

Prepare private member input for the packaged helper:

```json
{
  "ClusterID": "<new-cluster-id>",
  "Members": [
    {
      "node_id": "<voter-id>",
      "peer_url": "quic://<stable-voter-dns>:8100",
      "token": "<voter-peer-secret>"
    }
  ]
}
```

Set voter `GRANT_CLUSTER_MEMBERS` to that object's `Members` array. Grant derives
the member public key and local peer token from it. Alternatively, native member
`public_key` may carry the matching base64-encoded public key, with the local
token supplied through `RHIZA_PEER_TOKEN`; if both representations are supplied,
they must agree. Do not advertise `0.0.0.0`, `[::]`, a random port, or loopback for
a voter that must serve another Pod.

The accompanying `GRANT_RHIZA_CONFIG_JSON` object uses these actual fields:

```json
{
  "Local": false,
  "AdminToken": "<separate-replica-sync-secret>"
}
```

Use the existing KSA Workload Identity/ADC configuration for GCS on the seed,
main voter and learner. No static GCP key or `ObjStoreServiceAccount` value is
required for this profile. With that field empty or omitted, the namespace-list
helper uses `storage.NewClient` without explicit credentials, allowing ADC; the
owner-confirmed native GCS factory also supports default ADC. Ensure the workload
identity has the target bucket permissions required by each component.

An operator-supplied explicit credential is optional: `ObjStoreServiceAccount`,
if used, contains complete service-account JSON encoded as one JSON string, not
a credential-file path. Do not create static keys to follow this runbook. Keep the
peer secret, sync secret, public API keys, fence key and runtime writer secret
distinct. The sync credential is distributed to the passive learner only for
certified replica synchronization; do not expose Rhiza's native admin handler.

During import leave all of these unset: `GRANT_RUNTIME_BIND_ADDR`,
`GRANT_REMOTE_NODES`, `GRANT_RHIZA_REPLICA_CONFIG_JSON`,
`GRANT_CLUSTER_RESTORE_SNAPSHOT`, and `RHIZA_BIND_ADDR`. Do not configure native
`Learner`, `BindAddr`, or `EnableReconfiguration`. There must be exactly one
consistent explicit member, no filesystem archive directory, and no GCS endpoint
override or insecure mode. For real GCS leave `STORAGE_EMULATOR_HOST` unset.

## Import once and verify before serving

Run the packaged command as the sole offline target actor, with the configuration
above and the snapshot/data mount visible to it:

```sh
/usr/local/bin/grant migrate-restore \
  --snapshot /run/import/source-snapshot.json \
  --data-dir /data/rhiza/imported
```

The target directory must **not exist**; an existing empty directory also fails.
An `emptyDir` mount can contain the absent child path. No `init` or separate
genesis subcommand exists in Grant: the importer opens Rhiza with the fixed target
membership. It does not start Grant's public HTTP listeners or normal bootstrap.
Do not use `migrate-prepare-cluster`, which is a separate multi-voter S3 procedure.

The importer validates the snapshot before touching the target, lists namespace
vacancy through the GCS SDK, creates `grant-restore-pending.json`, and imports all
canonical tables atomically into an empty authority store. It re-exports and
compares table digests, verifies the schema and the internal
`grant_restore_activation` receipt against the source snapshot digest and target
cluster ID, then successfully closes the store before publishing completion.
Recognized legacy PEP tables receive only the documented application/revision-1
defaults; providers, action IDs, identity mappings, roles, bindings, catalogs and
authority receipts are not renamed or inferred.

Require a successful exit, absence of `grant-restore-pending.json`, and presence
of `grant-cutover-verified.json`. The verified marker contains `sourceDigest`,
`verifiedAt`, `backend: "rhiza"`, and `clusterId`; match its source digest and
cluster ID to the intended import. The non-Local pending marker additionally
binds a target-configuration digest. The internal receipt is checked by the
command; there is no new public raw-SQL verification endpoint.

On failure, keep the target isolated and preserve its directory, namespace,
markers and logs. Normal startup rejects a pending restore. The CLI requires an
absent directory, so do not delete a marker or reset/reuse a failed target merely
to retry. Preserve the frozen source for a reviewed retry or rollback decision.

## Deployment-owned one-time seed guard

Seed and main containers in the same Pod must mount the **same `emptyDir`** and
use the **same `GRANT_DATA_DIR` child path**. The seed runs before-ack; main opens
that verified directory with the steady-state configuration below. Do not copy
or recreate authority in a second permanent directory.

The deployment owner supplies an optional, one-time Snapshot Secret and startup
guards using the image's existing `/bin/sh`. These are deployment checks, not new
Grant CLI flags or automatic recovery behavior. Apply this order:

| Observed local state | Init/seed behavior |
|----------------------|--------------------|
| Pending marker exists, or verification/identity is inconsistent | Fail closed and retain evidence. |
| Valid verified marker exists and pending marker is absent | Skip seeding; require the expected target cluster/source provenance. |
| No verified marker, target directory absent, approved one-time payload present | Run the offline import above, then require its verified marker and successful exit. |
| No verified marker and payload absent, or target directory already exists without verified completion | Fail before main; do not open a blank database or reset the directory. |

Main's own deployment guard must also require the matching verified marker and
absence of a pending marker before executing `/usr/local/bin/grant` in async
mode. Do not rely on init completion alone or call normal Grant/SDK Open as a
test for an empty target.

After the first verified seed, the operator deletes the one-time Snapshot Secret
without changing the Pod template. Its mount must be optional so a replacement
Pod can reach the guard and fail explicitly when the payload is absent. Do not
keep the old snapshot permanently available to a recurring init container.
Ordinary main-container restarts retain the same Pod's `emptyDir` and verified
marker. A new Pod has a new empty directory; without the one-time payload its
guard must fail closed until an explicitly approved recovery is supplied.

Never automatically replay the original V5 snapshot after target writes have
begun. Recovery requires a reviewed newer logical import into a fresh target or
another explicitly planned procedure. This guard prevents stale initialization;
it does not qualify full voter WAL-loss recovery, add HA/PVCs, or replace the
source/target exclusivity requirement.

## Start the sole voter and private fence transport

After verified import, start `/usr/local/bin/grant` normally with the same target
identity, membership, peer/sync credentials, directory and GCS namespace, using
steady-state `ObjStoreDurability: "async"`. Remove the seed-only before-ack value
from configuration overrides. The equivalent environment value is
`RHIZA_OBJSTORE_DURABILITY=async`. `ObjStoreSyncInterval` is a Go duration: for
example, one minute is JSON integer `60000000000`, or environment value
`RHIZA_OBJSTORE_SYNC_INTERVAL=1m`. Choose the batching interval for the deployment;
it is not an RPO bound or proof that no other path flushes storage. This restart
contract is limited to the fresh, unregistered/unanchored single-voter target;
it does not authorize changing a registered generation's durability. Preserve
the snapshot's bootstrap identity configuration so startup does not seed a
different owner. Keep the source installation stopped.

Add these voter-only runtime settings:

| Variable | Value |
|----------|-------|
| `GRANT_RUNTIME_BIND_ADDR` | `127.0.0.1:8200` for the same-Pod terminator. |
| `GRANT_REMOTE_API_SECRET` | Distinct runtime writer credential, at least 32 bytes; required by this listener, never sent to the learner. |
| `GRANT_READ_FENCE_KEY` | Separate fence credential, at least 32 bytes, shared only with authorized fence clients. |

Expose only the required private fence path through the Operable-owned HTTPS
terminator; do not publish the writer API or put fence credentials on the public
Exposure route. The terminator forwards to same-Pod loopback TCP 8200. Its exact
upstream request is:

```http
POST /read-fence
Authorization: Bearer <fence-key>
```

Use a nil/zero-length body. A successful response is exactly the JSON object
`{"appliedSlot":<unsigned-integer>}` with no envelope or extra fields. Preserve
the status/body and avoid redirects. The request performs a linearizable read;
an unavailable voter returns 503 rather than a reusable cached slot.

Cross-Pod clients use HTTPS with a certificate valid for the configured hostname.
Mount a composed root bundle and set `SSL_CERT_FILE` on the learner to include the
terminator CA **and** the roots needed for Goauthy/GCS. This is standard Go TLS
trust, not a native Grant mTLS feature. Grant's fence client disables proxy use,
refuses redirects and uses only `GRANT_REMOTE_NODES[0]`. Verify that exact first
endpoint is reachable within the three-second fence/catch-up/read budget.

The stable QUIC peer on **UDP 8100** is separate from this HTTPS/TCP fence path.
Allow the learner to reach the advertised QUIC address. Do not send replica sync
through the HTTP terminator or assume TCP fence readiness proves QUIC readiness.

## Start the passive learner

Produce public membership using the packaged image, without a Go toolchain:

```sh
docker run --rm -i --network none --read-only --cap-drop ALL \
  --security-opt no-new-privileges --user 10001:10001 \
  --entrypoint /usr/local/bin/replica-members "$GRANT_IMAGE" \
  < private-members.json > public-replica-members.json
```

`GRANT_IMAGE` must be the verified producer image digest. The input uses `ClusterID`,
`Members`, and native member keys `node_id`, `peer_url`, optional `log_url`,
`wal_identity`, `public_key` and private `token`. The output is the public replica
member array, with no voter token. Preserve it unchanged; native voter member
JSON and replica member JSON are different shapes. Do not give the input file to
the learner.

Set learner configuration as follows, then start `/usr/local/bin/grant`:

| Variable | Value |
|----------|-------|
| `GRANT_RUNTIME_MODE` | `embedded` |
| `GRANT_LOCAL_NODE_ROLE` | `learner` |
| `GRANT_CLUSTER_ID` | The imported target cluster ID. |
| `GRANT_DATA_DIR` | Dedicated writable replica directory, separate from the voter's. |
| `GRANT_REMOTE_NODES` | The HTTPS terminator origin, without `/read-fence`. |
| `GRANT_READ_FENCE_KEY` | Matching private voter fence credential. |
| `GRANT_RHIZA_REPLICA_CONFIG_JSON` | Object with `ClusterID`, a unique `ReplicaID`, matching sync `AdminToken`, and `Members` set to the complete public helper output array. |
| `GRANT_RHIZA_CONFIG_JSON` | `Local: false`, without voter membership or peer secret. Omit `ObjStoreServiceAccount` for existing KSA Workload Identity/ADC; explicit JSON credentials are optional only. |
| `RHIZA_OBJSTORE_*` | Same provider, bucket and namespace. This passive reader does not select the voter's write durability policy. |

Leave `GRANT_CLUSTER_MEMBERS`, `RHIZA_PEER_TOKEN`, `GRANT_RUNTIME_BIND_ADDR`,
`GRANT_REMOTE_API_SECRET`, `RHIZA_BIND_ADDR`, native `Learner` promotion config and
restore-snapshot settings unset. Never distribute voter peer or runtime writer
credentials to this learner. Configure normal Goauthy validation, private
AppAuthKey and the exact provider-bound PEP ID for its consumer. Existing imported
provider/PEP bindings remain authoritative; this setup creates no resource
registry, delegation, application or implicit resource grant.

## Consumer cutover and acceptance

Check voter and learner readiness, then use actual end-user requests through the
provider-bound learner API. Readiness alone is not a fence. Require preserved
imported authority, an allowed check, a denied exact-resource check, revocation
visibility, and 503 when the fence endpoint is disconnected. Keep separate
consumer URLs for different provider-bound PEPs. See the
[user batch/filter contract](provider-user-authorization.md); HTTP forwarding still
uses its separate [catalog/record-scope contract](http-capabilities.md#record-level-list-scope),
including the 199-ID bound rather than unlimited list authorization.

Cut over to one authoritative target only after those checks. Keep the old source
and immutable snapshot stopped for rollback; never maintain two writable copies.
Post-cutover writes need a separate reverse-migration decision before reverting.

This procedure is a one-time logical import, not automatic recovery after voter
WAL or whole-directory loss. `emptyDir` and accepted async loss elsewhere do not
override non-Local identity safeguards. Preserve original voter state across an
ordinary restart; do not promise cold replacement under the same identity. GCS
SDK/emulator contract results also do not establish production-cloud parity,
cross-Pod TLS/QUIC reachability, or actual learner acceptance.
