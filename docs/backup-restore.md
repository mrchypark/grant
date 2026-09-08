# Native backup, migration and recovery

Grant's Rhiza database is not file-compatible with Hiqlite. Rauthy remains a separate identity system with its existing backup/lifecycle controls. The old Grant/Rauthy encrypted backup-pair procedures are retained in [history/hiqlite/backup-restore.md](history/hiqlite/backup-restore.md) for recovering a legacy source; do not run the Hiqlite restore wrapper around the Go executable.

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

Verify restored authority, generation/replay behavior and negative authorization checks before routing traffic. The isolated verification script exercises admission with one voter absent, all-voter activation, restored rows, rejection of an occupied target, and quorum recovery after restore.

## Native backups

The master-key-protected `POST /api/v1/admin/backups` produces a consistent logical snapshot. `GET /api/v1/admin/backups` lists local artifacts; `GET /api/v1/admin/backups/s3` lists S3 artifacts when S3 is configured. Logical snapshots preserve the canonical application schema, table data and generation/provenance history. These JSON artifacts are different from Hiqlite encrypted SQLite backups.

`GRANT_BACKUP_DIR` sets the local artifact directory. S3 backups use the configured Rhiza bucket under the `grant-backups/` subdirectory of its prefix. Use bucket retention/access controls appropriate to operational recovery. A native backup can be restored with the explicit `migrate-restore` command into a new isolated destination.

The local three-voter verification script checks continued writes with one voter down, same-directory peer recovery and a logical restore into a separate fresh three-voter prefix. It uses a disposable local object store and never invokes the current Kubernetes context.

## Retired Grant Hiqlite entrypoints

This native checkout refuses real Grant operations through `restore-full-cluster.sh`, `create-recovery-pair.sh`, and `materialize-s3-restore.sh grant` before any external mutation. Their dry runs remain available for historical contract fixtures. Recover a legacy Grant source from its preserved Hiqlite checkout; do not bypass the native backend boundary. `materialize-s3-restore.sh rauthy` remains supported for Rauthy's unchanged backend.
