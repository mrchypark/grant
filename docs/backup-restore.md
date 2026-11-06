# Native backup, migration and recovery

Grant's Rhiza database is not file-compatible with Hiqlite. Rauthy remains a separate identity system with its existing backup/lifecycle controls. The old Grant/Rauthy encrypted backup-pair procedures are retained in [history/hiqlite/backup-restore.md](history/hiqlite/backup-restore.md) for recovering a legacy source; do not run the Hiqlite restore wrapper around the Go executable.

## Rhiza v0.23.0 candidate qualification

Current source selects v0.23.0; the older version-specific evidence below remains
historical. The supplier has not qualified old-state upgrade or rollback. The
new `TestUpgradeLocalRetainedV090` uses only a synthetic large snapshot: an exact
published v0.9.0 executable imports into a fresh Local directory, then the candidate
opens that same directory, compares schema and all 31 canonical table digests,
writes a marker row and reopens again. Baseline image digest, OCI revision,
linux/amd64 platform, CGO-disabled build metadata and binary checksum are checked
in CI. Execution is pending; even a pass does not qualify arbitrary old stores,
non-Local state, crash recovery, downgrade or rollback.

For raw `Local: true`, the supplier confirms ordinary Open of an empty directory
from a complete compatible object archive without an Operator or PVC. Existing
Grant three-provider Local DR covers clean shutdown, full directory removal,
`restore-local-storage` and authority continuity. Rerunning it on v0.23.0 is a
separate validation from the retained-format probe. Async acknowledgement is not
proof of archive completion and carries no fixed recovery-point bound here.

Registered/enrolled/anchored non-Local recovery must preserve native admission
safeguards. The owner confirms v0.23.0 ordinary Open into a new empty directory
for a raw unregistered, unenrolled, unanchored single voter with one fixed member
and a complete certified object-store checkpoint and tail. No Operator or PVC is
required for that exact contract. Keep the old writer stopped and preserve the
exact identity/namespace; no overlapping writer or identity rewrite is allowed.
The Grant three-provider clean-close-to-empty-directory cold-DR extension is
pending CI, separate from that supplier source qualification. Unarchived async
acknowledgements, uncertain maintenance and anchored recovery are not covered.
Managed generation rules do not establish the raw topology's contract. Unresolved GCS
maintenance fails closed; no automatic unlock, GC or identity rewrite is authorized.
The owner separately confirms the initial unregistered/unanchored single-voter
before-ack-to-async exception described in [the runbook](single-voter-learner.md#start-the-sole-voter-and-private-fence-transport).

## Rhiza v0.21.2 upgrade compatibility guide

The historical v0.21.2 integration selected latticedb-go v0.11.3. This paragraph is not a v0.23.0 compatibility claim. Grant's canonical snapshot format and SQL schema were unchanged, but pre-v0.21 archives were incompatible and had no automatic migration. For those sources the guide required a new, empty `DataDir` and a new independent archive; preserve existing data and archives. See [runtime-modes.md](runtime-modes.md#rhiza-v021-compatibility).

Rhiza v0.19.0 advances SQL execution policy to 7 and changes protected QLog/WAL compatibility. It refuses old SQL history, materializations and checkpoints; mixed-version peers and downgrade after activation are unsupported. This requires logical migration to a fresh cluster, not a rolling binary upgrade. Keep the old binary, data directories, object namespace and recovery evidence available until cutover is verified. See the [upstream SQL execution policy compatibility boundary](https://github.com/mrchypark/rhiza/blob/v0.19.0/docs/sql-execution-policy.md) for the exact documented constraints.

For the historical registered three-voter cutover below, require either a Rhiza Operator integration with trusted external fencing or an approved persistent-storage strategy retaining each voter's original disk and WAL. That registered `emptyDir` topology cannot replace a pod under the same identity after WAL loss: cold startup intentionally returns `ErrVoterStateLost`. With surviving quorum, replacement uses a fresh learner, fencing and reconfiguration; otherwise use Operator-managed whole-generation recovery. The [recovery contract](https://github.com/mrchypark/rhiza/blob/v0.19.0/docs/recovery.md) and [embedded Operator guide](https://github.com/mrchypark/rhiza/blob/v0.19.0/docs/embedded-operator.md) define those historical prerequisites, not a blanket restriction on the raw single-voter contract above. Validate the chosen recovery strategy before production migration; copying an old voter identity or replaying old history cannot bypass registered admission.

1. Quiesce writes and create a consistent logical Grant snapshot with the old compatible binary through `POST /ops/backups` or the snapshot export path supported by that source binary. If voters disagree, reconcile the application state before selecting the snapshot.
2. Create a fresh target with a distinct cluster ID, peer identities, empty dedicated S3 prefix and new local data directories. Follow **Coordinated fresh three-voter restore** below, which uses `migrate-prepare-cluster` and requires every new voter to acknowledge the same immutable snapshot before activation.
3. Verify imported Grant rows and authorization behavior before switching traffic. Keep the old installation intact to resolve outcomes from its history; do not copy QLog/WAL segments, physical checkpoints, receipts, applied slots or migration history into the new cluster.

This migration uses the existing logical snapshot and coordinated restore commands; no automatic or in-place Rhiza storage migration is available. The standalone `migrate-restore` path is limited to a fresh isolated local single-voter destination.

## Single-instance Local archive recovery

v0.21.2 retains the v0.21.0 archive format; this patch upgrade requires no format
migration. The fresh-directory requirement below applies to older incompatible
archives, not to an existing compatible v0.21.0 Local installation.

The v0.21 archive format is incompatible with pre-v0.21 archives, with no automatic
migration. Start only with a new, empty `DataDir` and a new independent archive
directory or object-store namespace. Preserve existing data and archives unchanged.
Automatic recovery below applies only to a compatible v0.21 archive created there.
Filesystem maintenance and recovery require a dedicated, trusted, stable archive
root and parent outside `DataDir` on the same Linux or macOS host. Do not rename,
replace, remount or externally remove them while in use. Stop the old process
before offline restore or copying files; a Pod deletion response is not proof of
termination. Remote maintenance tokens cannot be cleared based on age or process state.

Rhiza v0.21.2 supports automatic archive restore for one Grant instance configured
with `Local: true`, a stable `NodeID` and `ClusterID`, and an archive outside its
local data directory. Explicit `ObjStoreDurability: "before-ack"` waits for remote
archival; explicit `"async"` permits acknowledged writes ahead of the archive.
Async data loss can therefore lose acknowledged but unarchived writes; no fixed
loss-window bound is guaranteed here. The v0.4.1 `restore-local-storage` CLI only
accepts before-ack. The v0.7.0 source candidate additionally accepts explicit async,
using the same Local Open/Close recovery path and unchanged ownership/preflight
checks. A successful restore is not proof that every acknowledged write survived. Keep one
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
