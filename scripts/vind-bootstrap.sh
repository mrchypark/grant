#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT_DIR="$(cd "${SCRIPT_DIR}/.." && pwd)"
CLUSTER_NAME="${CLUSTER_NAME:-grant-ci}"
NAMESPACE="${NAMESPACE:-grant}"
IMAGE_NAME="${IMAGE_NAME:-grant:local}"
DEFAULT_RAUTHY_IMAGE="$(sed -n 's/^[[:space:]]*image:[[:space:]]*\(ghcr.io\/conalog\/grant-rauthy-launcher[^[:space:]]*\).*/\1/p' "${ROOT_DIR}/deploy/k8s/rauthy-statefulset.yaml")"
RAUTHY_IMAGE="${RAUTHY_IMAGE:-${DEFAULT_RAUTHY_IMAGE}}"
KUSTOMIZE_DIR="${KUSTOMIZE_DIR:-$ROOT_DIR/deploy/overlays/ephemeral}"
KUBECONTEXT="${KUBECONTEXT:-}"
VIND_EXEC_ACTIVE="${VIND_EXEC_ACTIVE:-0}"

if [[ "${NAMESPACE}" != "grant" ]]; then
  echo "vind bootstrap uses a Kustomization fixed to namespace grant; got NAMESPACE=${NAMESPACE}" >&2
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

# shellcheck source=scripts/vind-image-helpers.sh
. "${SCRIPT_DIR}/vind-image-helpers.sh"

GRANT_MASTER_KEY="${GRANT_MASTER_KEY:-local-dev-master-key}"
GRANT_APP_AUTH_KEY="${GRANT_APP_AUTH_KEY:-local-dev-app-auth-key}"
HQL_SECRET_RAFT="${HQL_SECRET_RAFT:-SuperSecureSecret1337}"
HQL_SECRET_API="${HQL_SECRET_API:-AnotherSecureSecret42}"
GRANT_RAUTHY_CLIENT_ID="${GRANT_RAUTHY_CLIENT_ID:-grant}"
GRANT_RAUTHY_CLIENT_SECRET="${GRANT_RAUTHY_CLIENT_SECRET:-devRauthyClientSecret0000000000000000000000000000000000000000000}"
GRANT_RAUTHY_CLIENT_READ_API_KEY="${GRANT_RAUTHY_CLIENT_READ_API_KEY:-grant-client-read\$devRauthyClientReadApiKeySecret00000000000000000000000000000000000}"
ENC_KEYS="${ENC_KEYS:-devkey01/MDEyMzQ1Njc4OWFiY2RlZjAxMjM0NTY3ODlhYmNkZWY=}"
ENC_KEY_ACTIVE="${ENC_KEY_ACTIVE:-devkey01}"

# shellcheck source=scripts/k8s-secret-helpers.sh
. "${ROOT_DIR}/scripts/k8s-secret-helpers.sh"

cleanup_registry_auth() {
  if [[ -n "${RAUTHY_REGISTRY_CONFIG_DIR:-}" ]]; then
    rm -rf "${RAUTHY_REGISTRY_CONFIG_DIR}"
  fi
}
trap cleanup_registry_auth EXIT

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

prepare_rauthy_registry_auth
docker image inspect "${RAUTHY_IMAGE}" >/dev/null 2>&1 \
  || docker --context "${RAUTHY_DOCKER_CONTEXT}" --config "${RAUTHY_REGISTRY_CONFIG_DIR}" pull "${RAUTHY_IMAGE}"

ensure_runtime_image "${IMAGE_NAME}"
ensure_rauthy_runtime_image

if [[ "$(cd "${KUSTOMIZE_DIR}" && pwd -P)" == "${ROOT_DIR}/deploy/overlays/ephemeral" ]]; then
  object_store_arch="$(kubectl get nodes -o 'jsonpath={.items[0].status.nodeInfo.architecture}')"
  object_store_image="$(sed -n 's/^[[:space:]]*image:[[:space:]]*\(docker.io\/versity\/versitygw[^[:space:]]*\).*/\1/p' "${KUSTOMIZE_DIR}/rhiza-object-store.yaml" | head -n 1)"
  # Fail immediately on an unavailable fixture image, before creating workloads.
  docker pull --platform "linux/${object_store_arch}" "${object_store_image}"
fi

dump_rauthy_debug() {
  echo "[rauthy-debug] statefulset state" >&2
  kubectl -n "${NAMESPACE}" get deploy,pods >&2 || true
  kubectl -n "${NAMESPACE}" get statefulset,pods >&2 || true
  kubectl -n "${NAMESPACE}" describe statefulset/rauthy >&2 || true
  kubectl -n "${NAMESPACE}" describe pod -l app=rauthy >&2 || true
  kubectl -n "${NAMESPACE}" logs -l app=rauthy --all-containers --tail=200 >&2 || true
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
apply_rauthy_registry_pull_secret
apply_grant_secret
kubectl apply -k "${KUSTOMIZE_DIR}"

if [[ "$(cd "${KUSTOMIZE_DIR}" && pwd -P)" == "${ROOT_DIR}/deploy/overlays/ephemeral" ]]; then
  kubectl -n "${NAMESPACE}" rollout status deployment/grant-object-store --timeout=120s
fi

wait_for_resource statefulset rauthy
if ! kubectl -n "${NAMESPACE}" rollout status statefulset/rauthy --timeout=15m; then
  dump_rauthy_debug
  exit 1
fi

wait_for_resource statefulset grant-voter
kubectl -n "${NAMESPACE}" set image statefulset/grant-voter "grant=${IMAGE_NAME}" >/dev/null
applied_grant_image="$(kubectl -n "${NAMESPACE}" get statefulset/grant-voter \
  -o 'jsonpath={.spec.template.spec.containers[?(@.name=="grant")].image}')"
if [[ "${applied_grant_image}" != "${IMAGE_NAME}" ]]; then
  echo "Grant image override did not apply: expected ${IMAGE_NAME}, got ${applied_grant_image:-<empty>}" >&2
  exit 1
fi
if ! kubectl -n "${NAMESPACE}" rollout status statefulset/grant-voter --timeout=15m; then
  dump_grant_debug
  exit 1
fi

cat <<EOF
vind bootstrap completed.

Context: ${KUBECONTEXT:-$(kubectl config current-context)}
Namespace: ${NAMESPACE}
Image: ${IMAGE_NAME}
Rauthy Image: ${RAUTHY_IMAGE}

Next:
  load GRANT_MASTER_KEY and GRANT_APP_AUTH_KEY from caller-owned 0600 secret files without echoing them
  kubectl -n ${NAMESPACE} port-forward svc/grant 18080:80
EOF
