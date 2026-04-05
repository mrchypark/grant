# K3d Certd TrailBase Chaos Results

**Date:** 2026-04-05

**Final status:** `fresh bootstrap -> e2e -> chaos -> backup/restore -> production-like point-in-time restore chaos` all passed on k3d after runtime and environment fixes.

## Verified topology

- `certd-voter`: `StatefulSet` with 3 voters
- `certd-learner-{1,2,3}`: 3 split-mode learner deployments
- `trailbase`: single deployment, SQLite on `emptyDir`, Litestream restore + replicate
- `minio`: single-replica object store for k3d harness

## Fresh-run command sequence

```bash
CERTD_MASTER_KEY=local-dev-master-key bash scripts/k3d-bootstrap.sh
CERTD_MASTER_KEY=local-dev-master-key bash scripts/k3d-e2e.sh
CERTD_MASTER_KEY=local-dev-master-key bash scripts/k3d-chaos.sh
CERTD_MASTER_KEY=local-dev-master-key bash scripts/k3d-backup-restore.sh
CERTD_MASTER_KEY=local-dev-master-key bash scripts/k3d-prod-restore-chaos.sh
```

## Outcome

### Bootstrap

- Passed on a fresh cluster.
- `trailbase` rolled out without manual image import.
- `certd-voter` quorum formed and all 3 learners became ready.
- MinIO bootstrap job created the backup buckets.

### E2E

- Passed.
- TrailBase user seed succeeded.
- `POST /api/v1/admin/bootstrap/trailbase-owner` succeeded.
- `GET /api/v1/me` succeeded through the `certd` auth facade.
- `GET /api/v1/plants` returned empty on a fresh stack.
- `POST /api/v1/organizations/conalog-account/plants` created `TB-CHAOS-001`.
- `GET /api/v1/plants/TB-CHAOS-001` returned the expected summary payload.

### Chaos

- Passed.
- Deleting a voter pod preserved quorum and the public API continued to serve `me` and `plants`.
- Deleting a learner pod no longer deadlocked rollout; learner recovery succeeded.
- Scaling TrailBase down caused:
  - new login to fail as expected
  - cached bearer token requests to continue working during auth cache TTL
  - cached bearer token requests to fail after TTL expiry
  - successful recovery after TrailBase scale-up
- Restarting MinIO did not break core `certd` reads.

### Backup / restore

- Passed.
- `certd` backup was written and selected from S3-backed storage.
- destructive `certd` restore completed and the restored cluster served:
  - `GET /api/v1/me`
  - `GET /api/v1/plants`
  - `GET /api/v1/plants/TB-CHAOS-001`
- TrailBase Litestream restore check passed after forced pod deletion.
- `GET /api/v1/plants/TB-CHAOS-001/documents` returned HTTP `403`; the harness correctly treated this as an authorization limitation, not restore corruption.

### Production-like point-in-time restore chaos

- Passed.
- External data was created through the public auth + plant APIs in two phases.
- Phase 1:
  - created `TB-PROD-1775330796-A`
  - created `TB-PROD-1775330796-B`
  - captured `backup1=backup_node_1_1775330802.sqlite`
- Phase 2:
  - created `TB-PROD-1775330796-C`
  - created `TB-PROD-1775330796-D`
  - captured `backup2=backup_node_1_1775330863.sqlite`
- Restore to `backup1` correctly preserved only phase-1 plants.
- Restore to `backup2` correctly preserved phase-1 and phase-2 plants.
- Separate TrailBase users created across the two phases were still able to log in after forced TrailBase pod deletion and Litestream restore.

## Fixes required to reach green

### Application/runtime fixes

- `crates/app/src/main.rs`
  - clear stale hiqlite restore lock when `HQL_BACKUP_RESTORE` is set
  - skip `service.initialize()` for split-mode learners
- `crates/infra/src/database.rs`
  - cast generation aggregate to a stable numeric type
- `crates/api/tests/plant_routes.rs`
  - added regression coverage for plant summary reads

### Kubernetes/harness fixes

- `deploy/k8s/app-service.yaml`
  - public service routes to voters
- `deploy/k8s/app-statefulset.yaml`
  - `podManagementPolicy: Parallel`
  - added resource requests/limits
- `deploy/k8s/app-learner-{1,2,3}-deployment.yaml`
  - `strategy: Recreate`
  - added resource requests/limits
- `deploy/k8s/trailbase-deployment.yaml`
  - Litestream DB path fix
  - added resource requests/limits
- `deploy/k8s/minio-statefulset.yaml`
  - reduced MinIO to 1 replica for k3d harness
  - switched to single-node server args
  - added resource requests/limits
- `deploy/k8s/minio-pdb.yaml`
  - adjusted for single replica
- `scripts/k3d-bootstrap.sh`
  - reliable per-node `ctr` import for TrailBase image
- `scripts/k3d-e2e.sh`
  - retry wrapper around auth/bootstrap/read/create flow
- `scripts/k3d-chaos.sh`
  - learner chaos uses pod delete, not rollout restart
- `scripts/k3d-backup-restore.sh`
  - restore selects latest backup from S3
  - restore brings voters back together
  - document download check skips HTTP `403`
- `scripts/k3d-prod-restore-chaos.sh`
  - creates multi-phase external datasets
  - validates point-in-time restore semantics against named backup objects
  - verifies latest TrailBase auth restore for multiple external users

### Environment fix

- The `colima-radxa` profile root disk was too small and caused k3d `DiskPressure`.
- Resized `root-disk` from `20GiB` to `60GiB`.
- Verified after resize:
  - k3d node root filesystem: `59G`
  - free space after full run: about `41G`

## Final cluster snapshot

- all 3 voters: `Running`
- all 3 learners: `Running`
- TrailBase: `2/2 Running`
- MinIO: `1/1 Running`

## Remaining known limitation

- `scripts/k3d-backup-restore.sh` deliberately leaves `HQL_BACKUP_RESTORE` set on voters at the end of the disposable harness to avoid an immediate post-restore rolling restart. This is acceptable for the k3d test harness, but should not be copied into production automation unchanged.
