# K3d Certd TrailBase Chaos Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Build and verify a k3d topology with `certd` as `3 voter + 3 learner_only`, add single-node `TrailBase + Litestream + MinIO` auth recovery, and run chaos scenarios that measure the intended failure boundaries.

**Architecture:** Keep `certd` as the HA data plane and treat `TrailBase` as a fast-recovery auth service. Add TrailBase deployment artifacts to the k8s topology, wire `certd` introspection to the in-cluster TrailBase service, then update bootstrap/e2e/chaos scripts to verify both quorum behavior and auth cache behavior under failure.

**Tech Stack:** Rust, axum, hiqlite, TrailBase, Litestream, MinIO, Kubernetes, k3d, bash, kustomize.

---

### Task 1: Add TrailBase and Litestream k8s manifests

**Files:**
- Create: `deploy/k8s/trailbase-configmap.yaml`
- Create: `deploy/k8s/trailbase-service.yaml`
- Create: `deploy/k8s/trailbase-deployment.yaml`
- Create: `deploy/k8s/trailbase-backup-secret.example.yaml`
- Modify: `deploy/k8s/kustomization.yaml`

**Step 1: Write the manifest inventory**

Document the required resources:
- one `Service`
- one `Deployment`
- one `ConfigMap`
- one `Secret` contract for internal secret and MinIO creds

**Step 2: Add ConfigMap with runtime values**

Include:
- `TRAILBASE_ADDRESS`
- `TRAILBASE_DATA_DIR`
- `TRAILBASE_CERTD_INTERNAL_SECRET`
- `LITESTREAM_DB_PATH`
- `LITESTREAM_REPLICA_URL`

**Step 3: Add Deployment with restore-then-run pattern**

Define:
- init container for `litestream restore`
- main container for `trailbase-auth-bridge`
- sidecar or second container for `litestream replicate`
- `emptyDir` volume for local SQLite files

**Step 4: Add ClusterIP Service**

Expose:
- port `4000` or chosen internal auth port

**Step 5: Add manifests to kustomization**

Update `deploy/k8s/kustomization.yaml` so the new TrailBase resources are deployed with the rest of the topology.

**Step 6: Validate manifests**

Run: `kustomize build deploy/k8s`
Expected: valid rendered YAML including `trailbase`

**Step 7: Commit**

```bash
git add deploy/k8s
git commit -m "feat: add trailbase litestream k8s manifests"
```

### Task 2: Wire certd to in-cluster TrailBase

**Files:**
- Modify: `deploy/k8s/app-configmap.yaml`
- Modify: `deploy/k8s/app-statefulset.yaml`
- Modify: `deploy/k8s/app-learner-1-deployment.yaml`
- Modify: `deploy/k8s/app-learner-2-deployment.yaml`
- Modify: `deploy/k8s/app-learner-3-deployment.yaml`

**Step 1: Write the failing expectation**

Document the required env contract:
- `CERTD_TRAILBASE_BASE_URL=http://trailbase.certd.svc.cluster.local:4000`
- `CERTD_TRAILBASE_INTROSPECT_PATH=/api/certd/introspect`
- `CERTD_TRAILBASE_INTERNAL_SECRET`
- `CERTD_TRAILBASE_CACHE_TTL_SECONDS`

**Step 2: Add shared auth env to ConfigMap**

Put non-secret settings into `app-configmap.yaml`.

**Step 3: Add secret-based auth env to voters and learners**

Reference internal secret from `certd-secrets`.

**Step 4: Re-render manifests**

Run: `kustomize build deploy/k8s`
Expected: `certd` pods now point to in-cluster `trailbase`

**Step 5: Commit**

```bash
git add deploy/k8s/app-configmap.yaml deploy/k8s/app-statefulset.yaml deploy/k8s/app-learner-*-deployment.yaml
git commit -m "feat: wire certd to trailbase auth service"
```

### Task 3: Update k3d bootstrap for TrailBase

**Files:**
- Modify: `scripts/k3d-bootstrap.sh`

**Step 1: Add failing bootstrap expectation**

Bootstrap must now:
- create TrailBase/Litestream secret material
- wait for TrailBase rollout
- keep existing MinIO bucket bootstrap

**Step 2: Add TrailBase secret creation**

Create secrets for:
- `TRAILBASE_CERTD_INTERNAL_SECRET`
- MinIO credentials used by Litestream

**Step 3: Add rollout waits**

Wait for:
- `deployment/trailbase`
- then `statefulset/certd-voter`
- then learner deployments

**Step 4: Run shell syntax check**

Run: `bash -n scripts/k3d-bootstrap.sh`
Expected: no output

**Step 5: Commit**

```bash
git add scripts/k3d-bootstrap.sh
git commit -m "feat: bootstrap trailbase in k3d"
```

### Task 4: Add auth-aware k3d e2e flow

**Files:**
- Modify: `scripts/k3d-e2e.sh`
- Create: `scripts/k3d-seed-trailbase-user.sh`

**Step 1: Replace direct business-era assumptions**

E2E should:
- seed TrailBase user
- login through `certd`
- bootstrap owner
- use bearer token + `x-organization-id`

**Step 2: Add helper to seed TrailBase user**

Implement a helper script that creates a verified user in-cluster.

**Step 3: Update E2E script**

Change flow to:
- port-forward `certd`
- seed user
- login through `/api/auth/v1/login`
- call `/api/v1/admin/bootstrap/trailbase-owner`
- call `/api/v1/me`
- create or inspect plant
- optionally upload cert and sync if secrets exist

**Step 4: Run shell syntax check**

Run: `bash -n scripts/k3d-e2e.sh scripts/k3d-seed-trailbase-user.sh`
Expected: no output

**Step 5: Commit**

```bash
git add scripts/k3d-e2e.sh scripts/k3d-seed-trailbase-user.sh
git commit -m "feat: add trailbase-authenticated k3d e2e flow"
```

### Task 5: Rewrite chaos script around quorum and auth cache

**Files:**
- Modify: `scripts/k3d-chaos.sh`

**Step 1: Write the target scenarios**

Cover:
- voter pod delete
- learner restart
- TrailBase pod delete
- TrailBase auth cache TTL expiry
- MinIO restart

**Step 2: Add token-based baseline**

Script must:
- seed TrailBase user
- login through `certd`
- bootstrap owner
- create or locate baseline plant

**Step 3: Add voter/learner scenarios**

Retain quorum checks with:
- `/api/v1/me`
- `/api/v1/plants`
- sync or another write path

**Step 4: Add TrailBase failure scenario**

Verify:
- new login fails while TrailBase is down
- existing token works before cache TTL expires
- existing token fails after TTL expires
- login recovers after restart

**Step 5: Add MinIO failure scenario**

Verify:
- `certd` core API still works
- backup path behavior is observed explicitly
- document or artifact path behavior is observed explicitly

**Step 6: Run shell syntax check**

Run: `bash -n scripts/k3d-chaos.sh`
Expected: no output

**Step 7: Commit**

```bash
git add scripts/k3d-chaos.sh
git commit -m "feat: add trailbase-aware k3d chaos tests"
```

### Task 6: Add restore and backup checks for TrailBase

**Files:**
- Modify: `scripts/k3d-backup-restore.sh`
- Create: `scripts/k3d-trailbase-restore-check.sh`

**Step 1: Add TrailBase recovery scenario**

Define:
- delete TrailBase pod
- wipe local SQLite if needed
- verify Litestream restore path

**Step 2: Script restore verification**

Check:
- login works after restore
- `/api/v1/me` still maps to the same local user

**Step 3: Syntax check**

Run: `bash -n scripts/k3d-backup-restore.sh scripts/k3d-trailbase-restore-check.sh`
Expected: no output

**Step 4: Commit**

```bash
git add scripts/k3d-backup-restore.sh scripts/k3d-trailbase-restore-check.sh
git commit -m "feat: verify trailbase litestream restore path"
```

### Task 7: Document chaos findings and operating guidance

**Files:**
- Modify: `README.md`
- Create: `docs/plans/2026-04-04-k3d-certd-trailbase-chaos-results.md`

**Step 1: Update README topology section**

Document:
- `certd` quorum
- learner roles
- TrailBase single-node fast recovery model
- MinIO responsibilities

**Step 2: Add operator runbook snippets**

Include commands for:
- bootstrap
- e2e
- chaos
- restore

**Step 3: Add results document template**

Record:
- scenario
- expected behavior
- actual behavior
- follow-up items

**Step 4: Commit**

```bash
git add README.md docs/plans/2026-04-04-k3d-certd-trailbase-chaos-results.md
git commit -m "docs: document k3d certd trailbase chaos operations"
```

### Task 8: Run full verification

**Files:**
- Test: `deploy/k8s/*`
- Test: `scripts/*`

**Step 1: Run Rust test suite**

Run: `cargo test --workspace -- --nocapture`
Expected: PASS

**Step 2: Render manifests**

Run: `kustomize build deploy/k8s >/tmp/certd-k8s-rendered.yaml`
Expected: PASS

**Step 3: Run bootstrap**

Run: `./scripts/k3d-bootstrap.sh`
Expected: k3d cluster ready, all rollouts healthy

**Step 4: Run e2e**

Run: `./scripts/k3d-e2e.sh`
Expected: login, bootstrap, me, plant flow succeeds

**Step 5: Run chaos**

Run: `./scripts/k3d-chaos.sh`
Expected: scenarios complete and findings are captured

**Step 6: Run restore verification**

Run: `./scripts/k3d-backup-restore.sh`
Expected: restore path succeeds

**Step 7: Commit**

```bash
git add .
git commit -m "test: verify k3d certd trailbase chaos topology"
```
