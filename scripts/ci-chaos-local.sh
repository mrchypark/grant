#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
CLUSTER_NAME="${CLUSTER_NAME:-certd-ci}"
CI_CLUSTER_BACKEND="${CI_CLUSTER_BACKEND:-kind}"
NAMESPACE="${NAMESPACE:-certd}"
IMAGE_NAME="${IMAGE_NAME:-certd:local}"
TRAILBASE_IMAGE="${TRAILBASE_IMAGE:-trailbase-auth-bridge:local}"
CERTD_MASTER_KEY="${CERTD_MASTER_KEY:-local-dev-master-key}"
RESET_CLUSTER="${RESET_CLUSTER:-1}"
KEEP_CLUSTER="${KEEP_CLUSTER:-0}"
KUBE_CONTEXT="${KUBE_CONTEXT:-}"

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

cleanup() {
  if [[ "${CI_CLUSTER_BACKEND}" == "kind" && "${KEEP_CLUSTER}" != "1" ]]; then
    kind delete cluster --name "${CLUSTER_NAME}" >/dev/null 2>&1 || true
  fi
}
trap cleanup EXIT

case "${CI_CLUSTER_BACKEND}" in
  kind)
    need kind
    kubectl_context="kind-${CLUSTER_NAME}"
    ;;
  vind)
    kubectl_context="${KUBE_CONTEXT:-$(kubectl config current-context)}"
    ;;
  *)
    echo "unknown CI_CLUSTER_BACKEND=${CI_CLUSTER_BACKEND}" >&2
    exit 1
    ;;
esac

docker buildx build --load -t "${IMAGE_NAME}" .
docker buildx build --load -f Dockerfile.trailbase-auth-bridge -t "${TRAILBASE_IMAGE}" .

if [[ "${CI_CLUSTER_BACKEND}" == "kind" ]]; then
  CLUSTER_NAME="${CLUSTER_NAME}" \
  NAMESPACE="${NAMESPACE}" \
  IMAGE_NAME="${IMAGE_NAME}" \
  TRAILBASE_IMAGE="${TRAILBASE_IMAGE}" \
  CERTD_MASTER_KEY="${CERTD_MASTER_KEY}" \
  RESET_CLUSTER="${RESET_CLUSTER}" \
  APPLY_INGRESS=0 \
    bash "${ROOT_DIR}/scripts/kind-bootstrap.sh"
else
  KUBECONTEXT="${kubectl_context}" \
  NAMESPACE="${NAMESPACE}" \
  IMAGE_NAME="${IMAGE_NAME}" \
  TRAILBASE_IMAGE="${TRAILBASE_IMAGE}" \
  CERTD_MASTER_KEY="${CERTD_MASTER_KEY}" \
    bash "${ROOT_DIR}/scripts/vind-bootstrap.sh"
fi

kubectl config use-context "${kubectl_context}" >/dev/null

NAMESPACE="${NAMESPACE}" \
CERTD_MASTER_KEY="${CERTD_MASTER_KEY}" \
  bash "${ROOT_DIR}/scripts/k3d-e2e.sh"

if [[ "${CI_CLUSTER_BACKEND}" == "kind" ]]; then
  CLUSTER_NAME="${CLUSTER_NAME}" \
  NAMESPACE="${NAMESPACE}" \
  CERTD_CHAOS_WITH_LOAD="${CERTD_CHAOS_WITH_LOAD:-1}" \
    bash "${ROOT_DIR}/scripts/kind-chaos.sh"
else
  KUBE_CONTEXT="${kubectl_context}" \
  NAMESPACE="${NAMESPACE}" \
  CERTD_CHAOS_WITH_LOAD="${CERTD_CHAOS_WITH_LOAD:-1}" \
    bash "${ROOT_DIR}/scripts/vind-chaos.sh"
fi
