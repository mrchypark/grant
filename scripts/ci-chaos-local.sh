#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
CLUSTER_NAME="${CLUSTER_NAME:-certd-ci}"
CI_CLUSTER_BACKEND="${CI_CLUSTER_BACKEND:-vind}"
NAMESPACE="${NAMESPACE:-certd}"
IMAGE_NAME="${IMAGE_NAME:-certd:local}"
TRAILBASE_IMAGE="${TRAILBASE_IMAGE:-trailbase-auth-bridge:local}"
CERTD_MASTER_KEY="${CERTD_MASTER_KEY:-local-dev-master-key}"
BUILD_NETWORK="${BUILD_NETWORK:-default}"
RESET_CLUSTER="${RESET_CLUSTER:-1}"
KEEP_CLUSTER="${KEEP_CLUSTER:-0}"
VIND_COLIMA_AUTOFIX="${VIND_COLIMA_AUTOFIX:-1}"
VIND_MINIO_STORAGE_MODE="${VIND_MINIO_STORAGE_MODE:-ephemeral}"
CI_DOCKER_BUILDER_PRUNE="${CI_DOCKER_BUILDER_PRUNE:-0}"
CI_RUN_E2E="${CI_RUN_E2E:-1}"
CI_RUN_CHAOS="${CI_RUN_CHAOS:-1}"

need() {
  command -v "$1" >/dev/null 2>&1 || {
    echo "missing command: $1" >&2
    exit 1
  }
}

need docker
need kubectl
need curl
need jq
need vcluster

log_storage_state() {
  echo "[storage] filesystem usage"
  df -h || true
  echo "[storage] docker usage"
  docker system df || true
}

run_phase() {
  local phase="$1"
  local started_at finished_at elapsed_seconds
  shift

  started_at="$(date +%s)"
  echo "[phase] starting ${phase}"
  "$@"
  finished_at="$(date +%s)"
  elapsed_seconds="$((finished_at - started_at))"
  echo "[phase] completed ${phase} in ${elapsed_seconds}s"
}

if [[ "${CI_CLUSTER_BACKEND}" != "vind" ]]; then
  echo "unsupported CI_CLUSTER_BACKEND=${CI_CLUSTER_BACKEND}; only vind is supported" >&2
  exit 1
fi

cleanup() {
  if [[ "${KEEP_CLUSTER}" != "1" ]]; then
    CLUSTER_NAME="${CLUSTER_NAME}" \
    KEEP_CLUSTER="${KEEP_CLUSTER}" \
      bash "${ROOT_DIR}/scripts/vind-cluster.sh" delete >/dev/null 2>&1 || true
  fi
}
trap cleanup EXIT

log_storage_state
run_phase build-certd-image \
  docker buildx build --load --network "${BUILD_NETWORK}" -t "${IMAGE_NAME}" .
run_phase build-trailbase-image \
  docker buildx build --load -f Dockerfile.trailbase-auth-bridge --network "${BUILD_NETWORK}" -t "${TRAILBASE_IMAGE}" .
if [[ "${CI_DOCKER_BUILDER_PRUNE}" == "1" ]]; then
  echo "[storage] pruning buildkit cache before cluster bootstrap"
  docker builder prune -af >/dev/null || true
  docker image prune -f >/dev/null || true
fi
log_storage_state

run_phase ensure-vind-context \
  env \
  CLUSTER_NAME="${CLUSTER_NAME}" \
  RESET_CLUSTER="${RESET_CLUSTER}" \
  VIND_COLIMA_AUTOFIX="${VIND_COLIMA_AUTOFIX}" \
  bash "${ROOT_DIR}/scripts/vind-cluster.sh" ensure-context

run_phase bootstrap-vind-cluster \
  env \
  CLUSTER_NAME="${CLUSTER_NAME}" \
  NAMESPACE="${NAMESPACE}" \
  IMAGE_NAME="${IMAGE_NAME}" \
  TRAILBASE_IMAGE="${TRAILBASE_IMAGE}" \
  CERTD_MASTER_KEY="${CERTD_MASTER_KEY}" \
  VIND_MINIO_STORAGE_MODE="${VIND_MINIO_STORAGE_MODE}" \
  bash "${ROOT_DIR}/scripts/vind-exec.sh" bash "${ROOT_DIR}/scripts/vind-bootstrap.sh"

if [[ "${CI_RUN_E2E}" == "1" ]]; then
  run_phase local-e2e \
    env \
    CLUSTER_NAME="${CLUSTER_NAME}" \
    NAMESPACE="${NAMESPACE}" \
    CERTD_MASTER_KEY="${CERTD_MASTER_KEY}" \
    bash "${ROOT_DIR}/scripts/vind-exec.sh" bash "${ROOT_DIR}/scripts/k3d-e2e.sh"
else
  echo "[skip] local-e2e disabled via CI_RUN_E2E=${CI_RUN_E2E}"
fi

if [[ "${CI_RUN_CHAOS}" == "1" ]]; then
  run_phase local-chaos \
    env \
    CLUSTER_NAME="${CLUSTER_NAME}" \
    NAMESPACE="${NAMESPACE}" \
    CERTD_CHAOS_WITH_LOAD="${CERTD_CHAOS_WITH_LOAD:-1}" \
    bash "${ROOT_DIR}/scripts/vind-exec.sh" bash "${ROOT_DIR}/scripts/vind-chaos.sh"
else
  echo "[skip] local-chaos disabled via CI_RUN_CHAOS=${CI_RUN_CHAOS}"
fi
