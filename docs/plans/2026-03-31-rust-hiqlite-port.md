# Rust Hiqlite Port Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Replace the current Go + PocketBase implementation with a Rust service that uses hiqlite for replicated metadata storage and MinIO for artifact storage, with Kubernetes-first deployment and documented backup/restore.

**Architecture:** Build a single Rust binary that embeds hiqlite for strongly consistent metadata and uses S3-compatible object storage for PDF artifacts. Keep the public API shape aligned with the current `/api/v1` contract where possible, port the existing certificate/RPS sync logic into Rust modules, and deploy as a Kubernetes-first topology with a 3-node hiqlite cluster plus MinIO.

**Tech Stack:** Rust, Tokio, Axum, hiqlite, rusqlite/sqlx-compatible query layer as needed, object_store or aws-sdk-s3, MinIO, Docker, Kubernetes, kind for local validation.

---

### Task 1: Scaffold the Rust workspace

**Files:**
- Create: `Cargo.toml`
- Create: `rust-toolchain.toml`
- Create: `crates/app/Cargo.toml`
- Create: `crates/app/src/main.rs`
- Create: `crates/domain/Cargo.toml`
- Create: `crates/domain/src/lib.rs`
- Create: `crates/infra/Cargo.toml`
- Create: `crates/infra/src/lib.rs`
- Create: `crates/api/Cargo.toml`
- Create: `crates/api/src/lib.rs`
- Test: `cargo test --workspace`

**Step 1: Write the failing smoke test**

Add a workspace smoke test that asserts the API crate can build a router and the domain crate exposes a health type.

**Step 2: Run test to verify it fails**

Run: `cargo test --workspace`
Expected: FAIL because the workspace and crates do not exist yet.

**Step 3: Write minimal implementation**

Create the workspace, crates, and a minimal `main.rs` that starts and exits cleanly.

**Step 4: Run test to verify it passes**

Run: `cargo test --workspace`
Expected: PASS for the workspace smoke test.

### Task 2: Port the domain model

**Files:**
- Create: `crates/domain/src/business.rs`
- Create: `crates/domain/src/plant.rs`
- Create: `crates/domain/src/sync.rs`
- Create: `crates/domain/src/document.rs`
- Modify: `crates/domain/src/lib.rs`
- Test: `crates/domain/tests/model_roundtrip.rs`

**Step 1: Write the failing test**

Define JSON round-trip tests for:
- business
- plant summary
- generation record
- rate record
- REC record
- document metadata

**Step 2: Run test to verify it fails**

Run: `cargo test -p domain model_roundtrip`
Expected: FAIL because the types do not exist.

**Step 3: Write minimal implementation**

Port the current Go JSON model into Rust `serde` structs without persistence details.

**Step 4: Run test to verify it passes**

Run: `cargo test -p domain model_roundtrip`
Expected: PASS.

### Task 3: Add hiqlite-backed metadata storage

**Files:**
- Create: `crates/infra/src/config.rs`
- Create: `crates/infra/src/db/mod.rs`
- Create: `crates/infra/src/db/migrations.rs`
- Create: `crates/infra/src/db/repository.rs`
- Modify: `crates/infra/src/lib.rs`
- Test: `crates/infra/tests/repository_smoke.rs`

**Step 1: Write the failing test**

Write repository tests for:
- upserting businesses
- storing certificates metadata
- storing plants
- creating sync runs
- storing generation/rates/rec/documents
- retrieving latest plant summary

**Step 2: Run test to verify it fails**

Run: `cargo test -p infra repository_smoke`
Expected: FAIL because the repository and migrations do not exist.

**Step 3: Write minimal implementation**

Add hiqlite configuration, bootstrap, schema creation, and repository methods mirroring the current Go store layer.

**Step 4: Run test to verify it passes**

Run: `cargo test -p infra repository_smoke`
Expected: PASS.

### Task 4: Add MinIO/S3 artifact storage

**Files:**
- Create: `crates/infra/src/object_store.rs`
- Create: `crates/infra/tests/object_store_smoke.rs`
- Modify: `crates/infra/src/lib.rs`

**Step 1: Write the failing test**

Write tests for:
- object key generation
- artifact upload metadata
- download URL/path resolution

**Step 2: Run test to verify it fails**

Run: `cargo test -p infra object_store_smoke`
Expected: FAIL because the object store abstraction does not exist.

**Step 3: Write minimal implementation**

Implement a trait-backed artifact store with S3-compatible semantics and a local in-memory or filesystem test backend.

**Step 4: Run test to verify it passes**

Run: `cargo test -p infra object_store_smoke`
Expected: PASS.

### Task 5: Port certificate encryption and loading

**Files:**
- Create: `crates/infra/src/crypto.rs`
- Create: `crates/infra/tests/crypto_roundtrip.rs`

**Step 1: Write the failing test**

Add a round-trip test for encrypting and decrypting certificate bundles and passwords.

**Step 2: Run test to verify it fails**

Run: `cargo test -p infra crypto_roundtrip`
Expected: FAIL because crypto helpers do not exist.

**Step 3: Write minimal implementation**

Port the current AES-GCM envelope encryption logic from Go.

**Step 4: Run test to verify it passes**

Run: `cargo test -p infra crypto_roundtrip`
Expected: PASS.

### Task 6: Port the onerec and RPS clients

**Files:**
- Create: `crates/infra/src/onerec/mod.rs`
- Create: `crates/infra/src/rps/mod.rs`
- Create: `crates/infra/tests/rps_range_logic.rs`

**Step 1: Write the failing test**

Start with the deterministic pieces first:
- range resolution
- unpublished rate filtering
- delta merge behavior

**Step 2: Run test to verify it fails**

Run: `cargo test -p infra rps_range_logic`
Expected: FAIL because the logic has not been ported yet.

**Step 3: Write minimal implementation**

Port the non-network logic first, then the HTTP and certificate-login flows.

**Step 4: Run test to verify it passes**

Run: `cargo test -p infra rps_range_logic`
Expected: PASS.

### Task 7: Port the sync service

**Files:**
- Create: `crates/app/src/service.rs`
- Create: `crates/app/tests/service_sync.rs`

**Step 1: Write the failing test**

Cover:
- first sync defaults to full
- next sync defaults to delta
- `mode=full` with explicit range
- `mode=delta` rejects explicit range
- latest plant summary is read-only

**Step 2: Run test to verify it fails**

Run: `cargo test -p app service_sync`
Expected: FAIL because the service does not exist.

**Step 3: Write minimal implementation**

Port the current Go sync orchestration and latest-summary composition into Rust.

**Step 4: Run test to verify it passes**

Run: `cargo test -p app service_sync`
Expected: PASS.

### Task 8: Port the HTTP API

**Files:**
- Create: `crates/api/src/router.rs`
- Create: `crates/api/src/handlers.rs`
- Create: `crates/api/tests/http_contract.rs`

**Step 1: Write the failing test**

Mirror the current HTTP contract:
- `PUT /api/v1/businesses/{businessNo}/certificate`
- `POST /api/v1/businesses/{businessNo}/sync`
- `GET /api/v1/businesses/{businessNo}/plants`
- `GET /api/v1/plants/{facilityCode}`
- generation/rates/rec/documents/download

**Step 2: Run test to verify it fails**

Run: `cargo test -p api http_contract`
Expected: FAIL because the router does not exist.

**Step 3: Write minimal implementation**

Wire the service into Axum handlers and preserve current response filtering rules.

**Step 4: Run test to verify it passes**

Run: `cargo test -p api http_contract`
Expected: PASS.

### Task 9: Add Docker packaging

**Files:**
- Create: `Dockerfile`
- Create: `.dockerignore`
- Create: `docker/docker-compose.local.yml`
- Create: `docker/minio/init.sh`

**Step 1: Write the failing test**

Add a script or CI-style build command that expects the image to build.

**Step 2: Run test to verify it fails**

Run: `docker build -t cert-rust-local .`
Expected: FAIL because the Dockerfile does not exist.

**Step 3: Write minimal implementation**

Add a multi-stage Rust build Dockerfile and a local compose stack with app + MinIO for development.

**Step 4: Run test to verify it passes**

Run: `docker build -t cert-rust-local .`
Expected: PASS.

### Task 10: Add Kubernetes manifests

**Files:**
- Create: `deploy/k8s/app-configmap.yaml`
- Create: `deploy/k8s/app-statefulset.yaml`
- Create: `deploy/k8s/app-service.yaml`
- Create: `deploy/k8s/minio-tenant.yaml`
- Create: `deploy/k8s/ingress.yaml`
- Create: `deploy/k8s/kustomization.yaml`

**Step 1: Write the failing validation**

Run manifest validation expecting missing files.

**Step 2: Run test to verify it fails**

Run: `kubectl kustomize deploy/k8s`
Expected: FAIL because the manifests do not exist.

**Step 3: Write minimal implementation**

Add a Kubernetes-first deployment:
- app as StatefulSet for stable hiqlite node identities
- 3-node hiqlite cluster inside the app pods
- MinIO tenant manifest

**Step 4: Run test to verify it passes**

Run: `kubectl kustomize deploy/k8s`
Expected: PASS.

### Task 11: Add local Kubernetes validation

**Files:**
- Create: `scripts/dev-kind-up.sh`
- Create: `scripts/dev-kind-down.sh`
- Create: `scripts/dev-smoke.sh`

**Step 1: Write the failing validation**

Expect the scripts to be missing.

**Step 2: Run test to verify it fails**

Run: `bash scripts/dev-smoke.sh`
Expected: FAIL because the scripts do not exist.

**Step 3: Write minimal implementation**

Create scripts to:
- start a kind cluster
- deploy MinIO and the app
- run certificate upload + sync smoke checks

**Step 4: Run test to verify it passes**

Run: `bash scripts/dev-smoke.sh`
Expected: PASS in a prepared local environment.

### Task 12: Add backup and restore workflows

**Files:**
- Create: `scripts/backup-trigger.sh`
- Create: `scripts/restore-from-s3.sh`
- Create: `docs/backup-restore.md`

**Step 1: Write the failing validation**

Add a docs test or shell validation that expects backup and restore assets.

**Step 2: Run test to verify it fails**

Run: `test -f docs/backup-restore.md`
Expected: FAIL because the docs do not exist.

**Step 3: Write minimal implementation**

Document and script:
- hiqlite backup trigger
- MinIO bucket versioning assumptions
- full cluster restore using `HQL_BACKUP_RESTORE=s3:...`
- artifact restore verification

**Step 4: Run test to verify it passes**

Run: `test -f docs/backup-restore.md && bash -n scripts/backup-trigger.sh && bash -n scripts/restore-from-s3.sh`
Expected: PASS.
