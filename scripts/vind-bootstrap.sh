#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
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

redact_image_import_log() {
  local log_file="$1"

  sed -E \
    -e 's#(https?://)[^/@[:space:]]+@#\1[redacted]@#g' \
    -e 's#([Aa]uthorization:[[:space:]]*(Basic|Bearer)[[:space:]]+)[^[:space:]]+#\1[redacted]#g' \
    -e 's#([?&](access_token|password|token)=)[^&[:space:]]+#\1[redacted]#g' \
    "${log_file}" >&2
}

report_image_import_failure() {
  local image="$1"
  local step="$2"
  local log_file="$3"

  echo "failed to import image into vcluster runtime during ${step}: ${image}" >&2
  if [[ -s "${log_file}" ]]; then
    redact_image_import_log "${log_file}"
  fi
}

import_image_with_ctr_vcluster() (
  local image="$1"
  local container_name
  import_log=""
  tar_file=""

  # shellcheck disable=SC2329 # Invoked by the EXIT trap.
  cleanup_image_import() {
    if [[ -n "${tar_file}" ]]; then
      rm -f "${tar_file}"
    fi
    if [[ -n "${import_log}" ]]; then
      rm -f "${import_log}"
    fi
  }
  trap cleanup_image_import EXIT
  trap 'exit 129' HUP
  trap 'exit 130' INT
  trap 'exit 143' TERM

  container_name="$(cluster_container_name)"
  docker inspect "${container_name}" >/dev/null 2>&1 || {
    echo "missing vcluster container ${container_name}" >&2
    exit 1
  }

  tar_file="$(mktemp "${TMPDIR:-/tmp}/${CLUSTER_NAME}.image.XXXXXX")"
  import_log="$(mktemp "${TMPDIR:-/tmp}/${CLUSTER_NAME}.image-import.XXXXXX")"

  if ! docker save "${image}" -o "${tar_file}" >"${import_log}" 2>&1; then
    report_image_import_failure "${image}" "Docker archive creation" "${import_log}"
    return 1
  fi

  if ! docker exec -i "${container_name}" ctr -n k8s.io images import - >"${import_log}" 2>&1 <"${tar_file}"; then
    report_image_import_failure "${image}" "containerd archive import" "${import_log}"
    return 1
  fi
)

is_digest_qualified_image() {
  [[ "$1" =~ @sha256:[0-9a-f]{64}$ ]]
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
    echo "image archive import completed but the runtime reference is missing: ${image}" >&2
    exit 1
  }

  rm -f "${refs_file}"
}

ensure_rauthy_runtime_image() {
  if is_digest_qualified_image "${RAUTHY_IMAGE}"; then
    echo "[vind] leaving digest-qualified Rauthy image for authenticated kubelet pull: ${RAUTHY_IMAGE}"
    return 0
  fi

  ensure_runtime_image "${RAUTHY_IMAGE}"
}

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

wait_for_resource statefulset rauthy
if ! kubectl -n "${NAMESPACE}" rollout status statefulset/rauthy --timeout=15m; then
  dump_rauthy_debug
  exit 1
fi

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
Rauthy Image: ${RAUTHY_IMAGE}

Next:
  export GRANT_MASTER_KEY=${GRANT_MASTER_KEY}
  export GRANT_APP_AUTH_KEY=${GRANT_APP_AUTH_KEY}
  kubectl -n ${NAMESPACE} port-forward svc/grant 18080:80
EOF
