# K3s HA Chaos Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** `certd`를 K3s 계열 환경에 HA로 배포하고 chaos 및 backup/restore를 실제 검증 가능한 형태로 만든다.

**Architecture:** 로컬에서는 `k3d` 클러스터를 만들고, `certd` 3 replicas + MinIO shared object storage 조합으로 배포한다. `hiqlite` backup은 MinIO 버킷에 저장하고, restore는 `HQL_BACKUP_RESTORE`를 주입해 검증한다.

**Tech Stack:** k3d, k3s, kubectl, kustomize, helm, MinIO, Rust, hiqlite

---

### Task 1: K3s 로컬 배포 자산 정리

**Files:**
- Modify: `/Users/cypark/Documents/work/cert/deploy/k8s/*.yaml`
- Create: `/Users/cypark/Documents/work/cert/deploy/k8s/local/*`

### Task 2: MinIO 및 bucket bootstrap 추가

**Files:**
- Create: `/Users/cypark/Documents/work/cert/deploy/k8s/minio-*.yaml`
- Create: `/Users/cypark/Documents/work/cert/scripts/k3d-bootstrap.sh`

### Task 3: app HA / PDB / spread / probes 보강

**Files:**
- Modify: `/Users/cypark/Documents/work/cert/deploy/k8s/app-statefulset.yaml`
- Create: `/Users/cypark/Documents/work/cert/deploy/k8s/app-pdb.yaml`

### Task 4: E2E / chaos / restore 자동화 스크립트 추가

**Files:**
- Create: `/Users/cypark/Documents/work/cert/scripts/k3d-e2e.sh`
- Create: `/Users/cypark/Documents/work/cert/scripts/k3d-chaos.sh`
- Create: `/Users/cypark/Documents/work/cert/scripts/k3d-backup-restore.sh`

### Task 5: README / 운영 문서 갱신

**Files:**
- Modify: `/Users/cypark/Documents/work/cert/README.md`
- Modify: `/Users/cypark/Documents/work/cert/docs/backup-restore.md`

### Task 6: 실제 로컬 K3d 검증

**Run:**
- `docker build -t certd:local .`
- `scripts/k3d-bootstrap.sh`
- `scripts/k3d-e2e.sh`
- `scripts/k3d-chaos.sh`
- `scripts/k3d-backup-restore.sh`
