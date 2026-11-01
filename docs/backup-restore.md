# Native backup, migration and recovery

Grant's Rhiza database is not file-compatible with Hiqlite. Rauthy remains a separate identity system with its existing backup/lifecycle controls. The old Grant/Rauthy encrypted backup-pair procedures are retained in [history/hiqlite/backup-restore.md](history/hiqlite/backup-restore.md) for recovering a legacy source; do not run the Hiqlite restore wrapper around the Go executable.

## Rhiza v0.22.0 integration compatibility guide

Grant selects Rhiza v0.22.0 and latticedb-go v0.11.3 as an isolated integration candidate. Rollout is not qualified. Grant's canonical snapshot format and SQL schema are unchanged, but pre-v0.21 archives are incompatible and have no automatic migration. For pre-v0.21 sources use a new, empty `DataDir` and a new independent archive only; preserve existing data and archives. See [the v0.22 integration boundary](runtime-modes.md#rhiza-v022-integration-boundary) and [archive compatibility](runtime-modes.md#rhiza-v021-compatibility).

The new v1alpha2 managed HA reference path is not a recovery service for arbitrary
embedded Grant applications. The existing v1alpha1 Operator overlay is not qualified
with v0.22.0; do not replace its controller or CRDs as part of this dependency update.
There is no v1alpha1 conversion/migration path. Any managed deployment requires
separate integration and an explicitly provisioned isolated storage binding.

Rhiza v0.19.0 advances SQL execution policy to 7 and changes protected QLog/WAL compatibility. It refuses old SQL history, materializations and checkpoints; mixed-version peers and downgrade after activation are unsupported. This requires logical migration to a fresh cluster, not a rolling binary upgrade. Keep the old binary, data directories, object namespace and recovery evidence available until cutover is verified. See the [upstream SQL execution policy compatibility boundary](https://github.com/mrchypark/rhiza/blob/v0.19.0/docs/sql-execution-policy.md) for the exact documented constraints.

Before these logical cutover steps, require either a Rhiza Operator integration with trusted external fencing or an approved persistent-storage strategy retaining each voter's original disk and WAL. The current `emptyDir` deployment cannot replace a pod under the same identity after WAL loss: cold startup intentionally returns `ErrVoterStateLost`. With surviving quorum, replacement uses a fresh learner, fencing and reconfiguration; otherwise use Operator-managed whole-generation recovery. The [recovery contract](https://github.com/mrchypark/rhiza/blob/v0.19.0/docs/recovery.md) and [embedded Operator guide](https://github.com/mrchypark/rhiza/blob/v0.19.0/docs/embedded-operator.md) define the prerequisites. Validate the chosen recovery strategy before production migration; copying an old voter identity or replaying old history cannot bypass this requirement.

1. Quiesce writes and create a consistent logical Grant snapshot with the old compatible binary through `POST /ops/backups` or the snapshot export path supported by that source binary. If voters disagree, reconcile the application state before selecting the snapshot.
2. Create a fresh target with a distinct cluster ID, peer identities, empty dedicated S3 prefix and new local data directories. Follow **Coordinated fresh three-voter restore** below, which uses `migrate-prepare-cluster` and requires every new voter to acknowledge the same immutable snapshot before activation.
3. Verify imported Grant rows and authorization behavior before switching traffic. Keep the old installation intact to resolve outcomes from its history; do not copy QLog/WAL segments, physical checkpoints, receipts, applied slots or migration history into the new cluster.

This migration uses the existing logical snapshot and coordinated restore commands; no automatic or in-place Rhiza storage migration is available. The standalone `migrate-restore` path is limited to a fresh isolated local single-voter destination.

## Single-instance Local archive recovery

v0.22.0 retains Local archive format version 2 from v0.21.x. Separate CGO=0 Grant
fixtures on Darwin/ARM64 passed v0.21.2 to v0.22.0 retained-data reopen and direct
old-filesystem-archive restore after clean Close. Roles, bindings, ownership ETag,
allow/deny decisions and later candidate writes survived recovery. These are
bounded Local results; rolling upgrade, mixed-version HA, downgrade, abrupt crash
and live cloud recovery remain unqualified. The fresh-directory requirement below applies to older incompatible
archives; do not erase a compatible archive to test an upgrade.

Wrong NodeID at the existing archive was refused with archive contents unchanged;
the original identity subsequently recovered its authority. Changing only ClusterID
was accepted on both v0.21.2 and v0.22.0 because it selected a different, empty
archive namespace, not the old archive. Keep ClusterID, prefix and archive root
fixed during recovery and verify the expected business rows and ownership revision
after init succeeds. Do not interpret successful Open/Close as proof that the
intended archive was restored, or rely on ClusterID mismatch as a refusal guard.

The v0.21 archive format is incompatible with pre-v0.21 archives, with no automatic
migration. Start only with a new, empty `DataDir` and a new independent archive
directory or object-store namespace. Preserve existing data and archives unchanged.
Automatic recovery below applies only to a compatible v0.21 archive created there.
Filesystem maintenance and recovery require a dedicated, trusted, stable archive
root and parent outside `DataDir` on the same Linux or macOS host. Do not rename,
replace, remount or externally remove them while in use. Stop the old process
before offline restore or copying files; a Pod deletion response is not proof of
termination. Remote maintenance tokens cannot be cleared based on age or process state.

Rhiza v0.22.0 retains automatic archive restore for one Grant instance configured
with `Local: true`, a stable `NodeID` and `ClusterID`, and an archive outside its
local data directory. Use `ObjStoreDurability: "before-ack"` explicitly. Keep one
writer for that archive, including during recovery. On startup, `OpenStore` restores
missing local data before returning; no canonical snapshot import is required.
This path does not use the Rhiza recovery Operator, which requires at least three
voters and does not support Local mode. See [the Operator constraints](rhiza-operator.md).

Run the actual Grant store/service regression fixture with:

```sh
go test ./internal/grant -run '^TestServiceLocalArchiveTotalLoss$' -count=1 -v -timeout 2m
go test -race ./internal/grant -run '^TestServiceLocalArchiveTotalLoss$' -count=1 -timeout 2m
```

The fixture uses only temporary directories and a separate filesystem archive.
It creates users, an organization, a role/action binding and an ownership transfer,
closes the store, deletes its complete local directory, and lets normal Grant
startup restore it. Across two loss cycles it checks ownership identity/revision,
allowed and denied authorization decisions, denial of the former owner's writes,
and new group writes and reads. The second loss also verifies a write made after
the first recovery. Test cleanup closes the store and removes the disposable data
and archive; existing `rhiza-data` is never used.

This verifies clean-close total-directory-loss recovery through Grant's Go service
and native store, without a public HTTP server. It does not verify abrupt process
termination or prove archive durability before close. A filesystem archive on the
same physical disk cannot protect against losing that disk. S3, GCS and Azure
adapter contracts, Kubernetes recovery, external identity-provider behavior and
production-cloud parity are not covered by this fixture.

## Existing Hiqlite authority data

Stop source writers and obtain a consistent SQLite source through the legacy system's supported recovery path. Preserve the original database, keyring, backup artifacts and import receipts. The native exporter reads a transaction snapshot and never writes authority rows to that source.

```sh
go run ./cmd/grant migrate-export --writes-stopped \
  --sqlite /absolute/path/to/legacy.sqlite --out /absolute/path/to/new-export.json
```

The explicit `--writes-stopped` flag records the operator's assertion that source writes are stopped; it does not stop a running service itself. Export includes the application schema identity, table inventory, per-table counts/digests, canonical rows and whole-snapshot digest. Generation and provenance tables are included with authority rows. The output path must not exist.

Configure an isolated destination node and a new object-store prefix, then restore into a directory that does not exist. Supply the same required server security configuration as normal startup, but the restore command does not open HTTP listeners.

```sh
go run ./cmd/grant migrate-restore \
  --snapshot /absolute/path/to/new-export.json --data-dir /absolute/path/to/new-rhiza-directory
```

Restore writes a pending marker while importing. It validates table digests and schema before writing `grant-cutover-verified.json` and removing the pending marker. A failed restore destination must remain isolated from service traffic; normal startup rejects the pending marker. Preserve the source and export until application, generation/replay and negative authorization checks pass against the restored destination. Switch routing only after these checks. Rollback uses the preserved source while writes remain stopped; data written after cutover requires a separately planned reverse migration.

The standalone `migrate-restore` command supports an isolated local single-voter destination. It rejects shared or multi-voter configurations before creating files. For a fresh three-voter destination, use the coordinated procedure below. Do not import into an already active quorum.

## Coordinated fresh three-voter restore

Prepare a distinct target cluster ID, an empty S3 prefix, fresh local directories, and fixed membership for all target voters. Preserve the source cluster and its prefix. Configure the target `GRANT_CLUSTER_ID`, `GRANT_CLUSTER_MEMBERS`, `GRANT_NODE_ID`, peer address, S3 settings and required security variables. Admission rejects a nonempty target prefix and a target identity that matches the source.

```sh
go run ./cmd/grant migrate-prepare-cluster --snapshot /absolute/path/to/export.json
```

This command validates the snapshot and creates the target admission record without opening the database or public HTTP listener. Distribute the identical immutable snapshot file to every target voter, then explicitly set `GRANT_CLUSTER_RESTORE_SNAPSHOT=/absolute/path/to/export.json` on every voter and start them concurrently. With Kubernetes, mount the snapshot read-only at that path and set the variable in each pod's environment before starting the fresh StatefulSet. The artifact must come from an operator-controlled volume; it is not fetched from a user-supplied request URL.

Each voter validates the admission and acknowledges the same snapshot. Startup waits for all configured voters before opening the native cluster. Import and activation occur atomically, and public readiness remains closed until the restored content is verified. Normal startup cannot bypass a pending admission by omitting the snapshot variable. An interrupted restore retries with the same target identity, snapshot, directories and prefix; repreparing the identical admission is idempotent, while a changed snapshot or membership is rejected. Keep the artifact available during same-directory voter recovery.

Verify restored authority, generation/replay behavior and negative authorization checks before routing traffic. The isolated verification script exercises admission with one voter absent, all-voter activation, restored rows, rejection of an occupied target, and same-directory quorum recovery with original WALs after restore.

## Native backups

The private master-key-protected `POST /ops/backups` produces a consistent logical snapshot. `GET /ops/backups` returns `scope: "logical-snapshots"` and an `items` array whose entries identify `storage: "local"` or `"s3"`. The service selects configured storage; callers do not select an S3 endpoint. Neither route is published through management Exposure. Logical snapshots preserve the canonical application schema, table data and generation/provenance history.

On IED, native GCS durability/checkpoints remain the persistence mechanism. This logical export API writes a local JSON snapshot (and uploads it only when the existing S3 adapter is configured); it does not list or verify native GCS checkpoints. A successful empty list is not evidence of a durable GCS backup. With NoPVC, local exports must be copied off the Pod before replacement. No MinIO is required.

`GRANT_BACKUP_DIR` sets the local artifact directory. S3 backups use the configured Rhiza bucket under the `grant-backups/` subdirectory of its prefix. Use bucket retention/access controls appropriate to operational recovery. A native backup can be restored with the explicit `migrate-restore` command into a new isolated destination.

The corrected local three-voter fixture passes writes, follower strong reads and same-directory peer recovery with original WALs. Cold empty-directory startup under the same voter identity intentionally fails with `ErrVoterStateLost`; the fixture does not establish `emptyDir` pod-replacement compatibility. The verification script also exercises logical restore into a separate fresh three-voter prefix using a disposable local object store and never invokes the current Kubernetes context.

## Retired Grant Hiqlite entrypoints

`restore-full-cluster.sh` and `create-recovery-pair.sh` are retirement notices that exit 2 for every invocation, including dry runs. Their old recovery state machine, paired recovery workflow and fixtures have been removed. Historical Hiqlite procedures remain in the [immutable c716877 checkout](https://github.com/mrchypark/grant/tree/c716877e7b970b7d38392b377f64384936992884); use that checkout for a legacy source.

`materialize-s3-restore.sh grant` still refuses real Grant mutations. `materialize-s3-restore.sh rauthy`, the live Rauthy lifecycle gate, and the Go Rauthy launcher's authorized one-shot recovery remain supported. The unused shell `restore-once.sh` is removed from the generated ConfigMap; Rauthy's StatefulSet runs `/usr/local/bin/rauthy-launcher` directly. Native Grant uses the coordinated Rhiza recovery procedure above, not the retired paired Hiqlite workflow.
