#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
CLUSTER_NAME="${CLUSTER_NAME:-certd-k3d}"

need() {
  command -v "$1" >/dev/null 2>&1 || {
    echo "missing command: $1" >&2
    exit 1
  }
}

need k3d

echo "[sequence] bootstrap disposable baseline"
bash "${ROOT_DIR}/scripts/k3d-bootstrap.sh"

echo "[sequence] verify backup/restore on disposable baseline"
bash "${ROOT_DIR}/scripts/k3d-backup-restore.sh"

echo "[sequence] reset disposable cluster ${CLUSTER_NAME} before chaos"
if k3d cluster get "${CLUSTER_NAME}" >/dev/null 2>&1; then
  k3d cluster delete "${CLUSTER_NAME}"
fi

echo "[sequence] rebuild disposable baseline after restore verification"
bash "${ROOT_DIR}/scripts/k3d-bootstrap.sh"

echo "[sequence] run chaos on clean baseline"
bash "${ROOT_DIR}/scripts/k3d-chaos.sh"
