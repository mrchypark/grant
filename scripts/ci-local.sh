#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
CLUSTER_NAME="${CLUSTER_NAME:-grant-ci}"
CI_CLUSTER_BACKEND="${CI_CLUSTER_BACKEND:-vind}"
NAMESPACE="${NAMESPACE:-grant}"
IMAGE_NAME="${IMAGE_NAME:-grant:local}"
RAUTHY_IMAGE="${RAUTHY_IMAGE:-ghcr.io/sebadob/rauthy:0.36.0}"
GRANT_MASTER_KEY="${GRANT_MASTER_KEY:-local-dev-master-key}"
GRANT_APP_AUTH_KEY="${GRANT_APP_AUTH_KEY:-local-dev-app-auth-key}"
GRANT_RAUTHY_CLIENT_ID="${GRANT_RAUTHY_CLIENT_ID:-grant}"
GRANT_RAUTHY_CLIENT_SECRET="${GRANT_RAUTHY_CLIENT_SECRET:-devRauthyClientSecret0000000000000000000000000000000000000000000}"
GRANT_RAUTHY_CLIENT_READ_API_KEY="${GRANT_RAUTHY_CLIENT_READ_API_KEY:-grant-client-read\$devRauthyClientReadApiKeySecret00000000000000000000000000000000000}"
KUSTOMIZE_DIR="${KUSTOMIZE_DIR:-$ROOT_DIR/deploy/overlays/ephemeral}"
BUILD_NETWORK="${BUILD_NETWORK:-default}"
RESET_CLUSTER="${RESET_CLUSTER:-1}"
KEEP_CLUSTER="${KEEP_CLUSTER:-0}"
VIND_COLIMA_AUTOFIX="${VIND_COLIMA_AUTOFIX:-1}"
CI_DOCKER_BUILDER_PRUNE="${CI_DOCKER_BUILDER_PRUNE:-0}"
CI_SKIP_CLUSTER_DELETE_ON_FAILURE="${CI_SKIP_CLUSTER_DELETE_ON_FAILURE:-0}"

if [[ "${NAMESPACE}" != "grant" ]]; then
  echo "local validation uses a Kustomization fixed to namespace grant; got NAMESPACE=${NAMESPACE}" >&2
  exit 1
fi

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
  local exit_code="$?"
  if [[ "${KEEP_CLUSTER}" != "1" ]]; then
    if [[ "${exit_code}" -ne 0 && "${CI_SKIP_CLUSTER_DELETE_ON_FAILURE}" == "1" ]]; then
      echo "[cleanup] skipping cluster delete after failure"
      return
    fi
    CLUSTER_NAME="${CLUSTER_NAME}" \
    KEEP_CLUSTER="${KEEP_CLUSTER}" \
      bash "${ROOT_DIR}/scripts/vind-cluster.sh" delete >/dev/null 2>&1 || true
  fi
}
trap cleanup EXIT

run_phase verify-kubernetes-storage-profiles \
  sh "${ROOT_DIR}/scripts/verify-ephemeral-k8s.sh" static

log_storage_state
run_phase build-grant-image \
  docker buildx build --load --network "${BUILD_NETWORK}" -t "${IMAGE_NAME}" .
run_phase pull-rauthy-image \
  docker pull "${RAUTHY_IMAGE}"
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
  RAUTHY_IMAGE="${RAUTHY_IMAGE}" \
  GRANT_MASTER_KEY="${GRANT_MASTER_KEY}" \
  GRANT_APP_AUTH_KEY="${GRANT_APP_AUTH_KEY}" \
  GRANT_RAUTHY_CLIENT_ID="${GRANT_RAUTHY_CLIENT_ID}" \
  GRANT_RAUTHY_CLIENT_SECRET="${GRANT_RAUTHY_CLIENT_SECRET}" \
  GRANT_RAUTHY_CLIENT_READ_API_KEY="${GRANT_RAUTHY_CLIENT_READ_API_KEY}" \
  KUSTOMIZE_DIR="${KUSTOMIZE_DIR}" \
  bash "${ROOT_DIR}/scripts/vind-exec.sh" bash "${ROOT_DIR}/scripts/vind-bootstrap.sh"

run_phase verify-ephemeral-runtime \
  env \
  CLUSTER_NAME="${CLUSTER_NAME}" \
  NAMESPACE="${NAMESPACE}" \
  GRANT_RAUTHY_CLIENT_ID="${GRANT_RAUTHY_CLIENT_ID}" \
  GRANT_RAUTHY_CLIENT_SECRET="${GRANT_RAUTHY_CLIENT_SECRET}" \
  GRANT_RAUTHY_CLIENT_READ_API_KEY="${GRANT_RAUTHY_CLIENT_READ_API_KEY}" \
  REPLACE_RAUTHY_POD=1 \
  bash "${ROOT_DIR}/scripts/vind-exec.sh" sh "${ROOT_DIR}/scripts/verify-ephemeral-k8s.sh" live
