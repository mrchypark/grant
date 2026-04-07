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

docker buildx build --load --network "${BUILD_NETWORK}" -t "${IMAGE_NAME}" .
docker buildx build --load -f Dockerfile.trailbase-auth-bridge --network "${BUILD_NETWORK}" -t "${TRAILBASE_IMAGE}" .

kubectl_context="$(
  CLUSTER_NAME="${CLUSTER_NAME}" \
  RESET_CLUSTER="${RESET_CLUSTER}" \
  VIND_COLIMA_AUTOFIX="${VIND_COLIMA_AUTOFIX}" \
    bash "${ROOT_DIR}/scripts/vind-cluster.sh" ensure-context
)"

KUBECONTEXT="${kubectl_context}" \
NAMESPACE="${NAMESPACE}" \
IMAGE_NAME="${IMAGE_NAME}" \
TRAILBASE_IMAGE="${TRAILBASE_IMAGE}" \
CERTD_MASTER_KEY="${CERTD_MASTER_KEY}" \
  bash "${ROOT_DIR}/scripts/vind-bootstrap.sh"

kubectl config use-context "${kubectl_context}" >/dev/null

NAMESPACE="${NAMESPACE}" \
CERTD_MASTER_KEY="${CERTD_MASTER_KEY}" \
  bash "${ROOT_DIR}/scripts/k3d-e2e.sh"

KUBE_CONTEXT="${kubectl_context}" \
NAMESPACE="${NAMESPACE}" \
CERTD_CHAOS_WITH_LOAD="${CERTD_CHAOS_WITH_LOAD:-1}" \
  bash "${ROOT_DIR}/scripts/vind-chaos.sh"
