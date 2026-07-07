#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
CLUSTER_NAME="${CLUSTER_NAME:-grant-ci}"
NAMESPACE="${NAMESPACE:-grant}"
IMAGE_NAME="${IMAGE_NAME:-grant:local}"
TRAILBASE_IMAGE="${TRAILBASE_IMAGE:-trailbase-auth-bridge:local}"
KUSTOMIZE_DIR="${KUSTOMIZE_DIR:-$ROOT_DIR/deploy/k8s}"
KUBECONTEXT="${KUBECONTEXT:-}"
VIND_EXEC_ACTIVE="${VIND_EXEC_ACTIVE:-0}"

need() {
  command -v "$1" >/dev/null 2>&1 || {
    echo "missing command: $1" >&2
    exit 1
  }
}

need docker
need kubectl

cluster_container_name() {
  printf 'vcluster.cp.%s' "${CLUSTER_NAME}"
}

image_refs_to_match() {
  local image="$1"

  printf '%s\n' "${image}"
  if [[ "${image%%/*}" == "${image}" ]]; then
    printf 'docker.io/library/%s\n' "${image}"
  fi
}

import_image_with_ctr_vcluster() {
  local image="$1"
  local container_name
  local tar_file

  container_name="$(cluster_container_name)"
  docker inspect "${container_name}" >/dev/null 2>&1 || {
    echo "missing vcluster container ${container_name}" >&2
    exit 1
  }

  tar_file="$(mktemp "${TMPDIR:-/tmp}/${CLUSTER_NAME}.image.XXXXXX")"
  docker save "${image}" -o "${tar_file}"
  docker exec -i "${container_name}" ctr -n k8s.io images import - >/dev/null <"${tar_file}"
  rm -f "${tar_file}"
}

ensure_runtime_image() {
  local image="$1"
  local container_name
  local refs_file

  container_name="$(cluster_container_name)"
  docker inspect "${container_name}" >/dev/null 2>&1 || {
    echo "missing vind runtime for ${CLUSTER_NAME}; expected vcluster" >&2
    exit 1
  }

  refs_file="$(mktemp "${TMPDIR:-/tmp}/${CLUSTER_NAME}.refs.XXXXXX")"
  image_refs_to_match "${image}" >"${refs_file}"

  if docker exec "${container_name}" ctr -n k8s.io images ls -q \
    | grep -F -x -f "${refs_file}" >/dev/null 2>&1; then
    echo "[vind] image already present in runtime: ${image}"
    rm -f "${refs_file}"
    return 0
  fi

  echo "[vind] importing image into runtime: ${image}"
  import_image_with_ctr_vcluster "${image}"
  docker exec "${container_name}" ctr -n k8s.io images ls -q \
    | grep -F -x -f "${refs_file}" >/dev/null 2>&1 || {
    rm -f "${refs_file}"
    echo "failed to import image into vcluster runtime: ${image}" >&2
    exit 1
  }

  rm -f "${refs_file}"
}

GRANT_MASTER_KEY="${GRANT_MASTER_KEY:-local-dev-master-key}"
HQL_SECRET_RAFT="${HQL_SECRET_RAFT:-SuperSecureSecret1337}"
HQL_SECRET_API="${HQL_SECRET_API:-AnotherSecureSecret42}"
TRAILBASE_GRANT_INTERNAL_SECRET="${TRAILBASE_GRANT_INTERNAL_SECRET:-dev-trailbase-internal-secret}"
ENC_KEYS="${ENC_KEYS:-devkey01/MDEyMzQ1Njc4OWFiY2RlZjAxMjM0NTY3ODlhYmNkZWY=}"
ENC_KEY_ACTIVE="${ENC_KEY_ACTIVE:-devkey01}"

# shellcheck source=scripts/k8s-secret-helpers.sh
. "${ROOT_DIR}/scripts/k8s-secret-helpers.sh"

if [[ -n "${KUBECONTEXT}" ]]; then
  kubectl config use-context "${KUBECONTEXT}" >/dev/null
  export KUBE_CONTEXT="${KUBECONTEXT}"
elif [[ "${VIND_EXEC_ACTIVE}" != "1" ]]; then
  echo "missing KUBECONTEXT; run via scripts/vind-exec.sh or pass KUBECONTEXT explicitly" >&2
  exit 1
fi

docker image inspect "${IMAGE_NAME}" >/dev/null 2>&1 || {
  echo "missing image ${IMAGE_NAME}. run: docker build -t ${IMAGE_NAME} ." >&2
  exit 1
}

docker image inspect "${TRAILBASE_IMAGE}" >/dev/null 2>&1 || {
  echo "missing image ${TRAILBASE_IMAGE}. run: docker build -f Dockerfile.trailbase-auth-bridge -t ${TRAILBASE_IMAGE} ." >&2
  exit 1
}

ensure_runtime_image "${IMAGE_NAME}"
ensure_runtime_image "${TRAILBASE_IMAGE}"

apply_resource() {
  kubectl -n "${NAMESPACE}" apply -f "$1"
}

dump_trailbase_debug() {
  echo "[trailbase-debug] deployment state" >&2
  kubectl -n "${NAMESPACE}" get deploy,pods >&2 || true
  kubectl -n "${NAMESPACE}" describe deployment/trailbase >&2 || true
  kubectl -n "${NAMESPACE}" describe pod -l app=trailbase >&2 || true
  kubectl -n "${NAMESPACE}" logs -l app=trailbase --all-containers --tail=200 >&2 || true
  kubectl -n "${NAMESPACE}" get events --sort-by=.lastTimestamp | tail -n 60 >&2 || true
}

dump_grant_debug() {
  echo "[grant-debug] statefulset state" >&2
  kubectl -n "${NAMESPACE}" get statefulset,pods >&2 || true
  kubectl -n "${NAMESPACE}" describe statefulset/grant-voter >&2 || true
  kubectl -n "${NAMESPACE}" describe pod -l app=grant,role=voter >&2 || true
  kubectl -n "${NAMESPACE}" logs -l app=grant,role=voter --all-containers --tail=200 >&2 || true
  kubectl -n "${NAMESPACE}" get events --sort-by=.lastTimestamp | tail -n 60 >&2 || true
}

wait_for_resource() {
  local kind_name="$1"
  local name="$2"
  local attempts="${3:-60}"
  local delay="${4:-2}"
  for _ in $(seq 1 "${attempts}"); do
    if kubectl -n "${NAMESPACE}" get "${kind_name}" "${name}" >/dev/null 2>&1; then
      return 0
    fi
    sleep "${delay}"
  done
  echo "timed out waiting for ${kind_name}/${name} to exist" >&2
  exit 1
}

kubectl create namespace "${NAMESPACE}" --dry-run=client -o yaml | kubectl apply -f -
apply_grant_secret

apply_resource "${KUSTOMIZE_DIR}/trailbase-configmap.yaml"
apply_resource "${KUSTOMIZE_DIR}/trailbase-service.yaml"
apply_resource "${KUSTOMIZE_DIR}/trailbase-deployment.yaml"
wait_for_resource deployment trailbase
if ! kubectl -n "${NAMESPACE}" rollout status deployment/trailbase --timeout=15m; then
  dump_trailbase_debug
  exit 1
fi

apply_resource "${KUSTOMIZE_DIR}/app-configmap.yaml"
apply_resource "${KUSTOMIZE_DIR}/app-headless-service.yaml"
apply_resource "${KUSTOMIZE_DIR}/app-service.yaml"
apply_resource "${KUSTOMIZE_DIR}/app-pdb.yaml"
apply_resource "${KUSTOMIZE_DIR}/app-statefulset.yaml"

wait_for_resource statefulset grant-voter
if ! kubectl -n "${NAMESPACE}" rollout status statefulset/grant-voter --timeout=15m; then
  dump_grant_debug
  exit 1
fi

cat <<EOF
vind bootstrap completed.

Context: ${KUBECONTEXT:-$(kubectl config current-context)}
Namespace: ${NAMESPACE}
Image: ${IMAGE_NAME}
TrailBase Image: ${TRAILBASE_IMAGE}

Next:
  export GRANT_MASTER_KEY=${GRANT_MASTER_KEY}
  kubectl -n ${NAMESPACE} port-forward svc/grant 18080:8080
EOF
