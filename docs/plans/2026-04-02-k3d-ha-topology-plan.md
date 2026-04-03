# K3d HA Topology Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** `cert` 레포를 `3 voter StatefulSet + 3 learner Deployment + MinIO backup/object store` 토폴로지로 전환한다.

**Architecture:** voter는 stable identity를 가진 `StatefulSet` quorum으로 운영하고, learner는 `split` 런타임의 read replica `Deployment`로 분리한다. artifact 저장과 hiqlite backup은 모두 MinIO에 두고, bootstrap/backup/restore/chaos 스크립트가 이 토폴로지를 직접 다루게 만든다.

**Tech Stack:** Rust, hiqlite, Kubernetes manifests, kustomize, k3d, MinIO, bash

---

### Task 1: 문서와 기준 토폴로지 고정

**Files:**
- Create: `docs/plans/2026-04-02-k3d-ha-topology-design.md`
- Create: `docs/plans/2026-04-02-k3d-ha-topology-plan.md`

**Step 1: 디자인 문서 작성**

`StatefulSet voter`, `Deployment learner`, `MinIO backup/artifact`, `chaos/restore` 범위를 짧게 기록한다.

**Step 2: 구현 계획 문서 작성**

매니페스트, 스크립트, 문서, 검증 순서를 적는다.

### Task 2: Kubernetes 토폴로지 전환

**Files:**
- Modify: `deploy/k8s/app-configmap.yaml`
- Delete: `deploy/k8s/app-deployment.yaml`
- Create: `deploy/k8s/app-headless-service.yaml`
- Create: `deploy/k8s/app-pdb.yaml`
- Create: `deploy/k8s/app-statefulset.yaml`
- Create: `deploy/k8s/app-learner-1-service.yaml`
- Create: `deploy/k8s/app-learner-2-service.yaml`
- Create: `deploy/k8s/app-learner-3-service.yaml`
- Create: `deploy/k8s/app-learner-1-deployment.yaml`
- Create: `deploy/k8s/app-learner-2-deployment.yaml`
- Create: `deploy/k8s/app-learner-3-deployment.yaml`
- Modify: `deploy/k8s/app-service.yaml`
- Modify: `deploy/k8s/kustomization.yaml`

**Step 1: voter manifest 도입**

단일 `Deployment`를 제거하고, `certd-voter` `StatefulSet`과 headless service/PDB를 추가한다.

**Step 2: learner manifest 도입**

`certd-learner-{1,2,3}` deployment/service를 추가하고 `publishNotReadyAddresses`를 켠다.

**Step 3: 공통 config 정렬**

`HQL_NODES`, `CERTD_RUNTIME_MODE`, `CERTD_LOCAL_NODE_ROLE`, `CERTD_REMOTE_NODES` 등이 새 토폴로지와 맞는지 확인한다.

### Task 3: 운영 스크립트 전환

**Files:**
- Modify: `scripts/k3d-bootstrap.sh`
- Modify: `scripts/k3d-backup-restore.sh`
- Modify: `scripts/k3d-chaos.sh`
- Modify: `scripts/backup-trigger.sh`
- Modify: `scripts/restore-from-s3.sh`

**Step 1: bootstrap 스크립트 전환**

`statefulset/certd-voter`와 `deployment/certd-learner-*` rollout을 기다리게 바꾼다.

**Step 2: backup/restore 절차 전환**

restore 시 learner scale down -> voter scale down -> restore env 주입 -> voter 복구 -> learner 복구 순서를 강제한다.

**Step 3: chaos 스크립트 전환**

voter pod 삭제, learner rollout restart, MinIO pod 삭제 시나리오를 새 리소스 이름으로 맞춘다.

### Task 4: 문서 정리

**Files:**
- Modify: `README.md`
- Modify: `docs/backup-restore.md`

**Step 1: README 정리**

배포 구조, 환경변수, bootstrap 순서를 `StatefulSet + learner` 구조 기준으로 수정한다.

**Step 2: backup/restore 문서 정리**

복구 대상과 순서를 `statefulset/certd-voter` 기준으로 바꾼다.

### Task 5: 검증

**Files:**
- Verify only

**Step 1: kustomize 검증**

Run: `kustomize build deploy/k8s`

Expected: exit 0

**Step 2: shell 문법 검증**

Run: `bash -n scripts/k3d-bootstrap.sh scripts/k3d-backup-restore.sh scripts/k3d-chaos.sh scripts/backup-trigger.sh scripts/restore-from-s3.sh`

Expected: exit 0

**Step 3: Rust 테스트 재확인**

Run: `cargo test --workspace -- --nocapture`

Expected: exit 0
