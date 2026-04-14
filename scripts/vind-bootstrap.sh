#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
CLUSTER_NAME="${CLUSTER_NAME:-certd-ci}"
NAMESPACE="${NAMESPACE:-certd}"
IMAGE_NAME="${IMAGE_NAME:-certd:local}"
TRAILBASE_IMAGE="${TRAILBASE_IMAGE:-trailbase-auth-bridge:local}"
KUSTOMIZE_DIR="${KUSTOMIZE_DIR:-$ROOT_DIR/deploy/k8s}"
VIND_SKIP_MINIO="${VIND_SKIP_MINIO:-0}"
VIND_MINIO_STORAGE_MODE="${VIND_MINIO_STORAGE_MODE:-persistent}"
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

k3d_context_name() {
  printf 'k3d-%s' "${CLUSTER_NAME}"
}

runtime_kind() {
  if docker inspect "$(cluster_container_name)" >/dev/null 2>&1; then
    printf 'vcluster'
  elif command -v k3d >/dev/null 2>&1 && k3d cluster get "${CLUSTER_NAME}" >/dev/null 2>&1; then
    printf 'k3d'
  else
    return 1
  fi
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

k3d_node_names() {
  docker ps --format '{{.Names}}' | grep -E "^k3d-${CLUSTER_NAME}-(server|agent)-" || true
}

import_image_with_ctr_k3d() {
  local image="$1"
  local tar_file
  local nodes

  nodes="$(k3d_node_names)"
  if [[ -z "${nodes}" ]]; then
    echo "failed to find k3d node containers for ${CLUSTER_NAME}" >&2
    exit 1
  fi

  tar_file="$(mktemp "${TMPDIR:-/tmp}/${CLUSTER_NAME}.image.XXXXXX")"
  docker save "${image}" -o "${tar_file}"
  for node in ${nodes}; do
    docker cp "${tar_file}" "${node}:/tmp/k3d-image-import.tar"
    docker exec "${node}" ctr -n k8s.io images import /tmp/k3d-image-import.tar >/dev/null
    docker exec "${node}" rm -f /tmp/k3d-image-import.tar >/dev/null 2>&1 || true
  done
  rm -f "${tar_file}"
}

ensure_runtime_image() {
  local image="$1"
  local kind
  local refs_file

  kind="$(runtime_kind)" || {
    echo "missing vind runtime for ${CLUSTER_NAME}; expected vcluster or k3d" >&2
    exit 1
  }

  refs_file="$(mktemp "${TMPDIR:-/tmp}/${CLUSTER_NAME}.refs.XXXXXX")"
  image_refs_to_match "${image}" >"${refs_file}"

  if [[ "${kind}" == "vcluster" ]]; then
    local container_name
    container_name="$(cluster_container_name)"

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
  else
    local nodes
    nodes="$(k3d_node_names)"
    if [[ -z "${nodes}" ]]; then
      rm -f "${refs_file}"
      echo "failed to find k3d node containers for ${CLUSTER_NAME}" >&2
      exit 1
    fi

    if docker exec "$(printf '%s\n' "${nodes}" | head -n1)" ctr -n k8s.io images ls -q \
      | grep -F -x -f "${refs_file}" >/dev/null 2>&1; then
      echo "[vind] image already present in runtime: ${image}"
      rm -f "${refs_file}"
      return 0
    fi

    echo "[vind] importing image into k3d runtime: ${image}"
    if ! k3d image import "${image}" -c "${CLUSTER_NAME}" >/dev/null 2>&1; then
      import_image_with_ctr_k3d "${image}"
    fi
    docker exec "$(printf '%s\n' "${nodes}" | head -n1)" ctr -n k8s.io images ls -q \
      | grep -F -x -f "${refs_file}" >/dev/null 2>&1 || {
      rm -f "${refs_file}"
      echo "failed to import image into k3d runtime: ${image}" >&2
      exit 1
    }
  fi

  rm -f "${refs_file}"
}

CERTD_MASTER_KEY="${CERTD_MASTER_KEY:-local-dev-master-key}"
HQL_SECRET_RAFT="${HQL_SECRET_RAFT:-SuperSecureSecret1337}"
HQL_SECRET_API="${HQL_SECRET_API:-AnotherSecureSecret42}"
TRAILBASE_CERTD_INTERNAL_SECRET="${TRAILBASE_CERTD_INTERNAL_SECRET:-dev-trailbase-internal-secret}"
ENC_KEYS="${ENC_KEYS:-devkey01/MDEyMzQ1Njc4OWFiY2RlZjAxMjM0NTY3ODlhYmNkZWY=}"
ENC_KEY_ACTIVE="${ENC_KEY_ACTIVE:-devkey01}"
MINIO_ROOT_USER="${MINIO_ROOT_USER:-minioadmin}"
MINIO_ROOT_PASSWORD="${MINIO_ROOT_PASSWORD:-minioadmin}"

if [[ -n "${KUBECONTEXT}" ]]; then
  kubectl config use-context "${KUBECONTEXT}" >/dev/null
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

dump_minio_debug() {
  echo "[minio-debug] storage mode=${VIND_MINIO_STORAGE_MODE}" >&2
  kubectl get storageclass >&2 || true
  kubectl get nodes -o wide >&2 || true
  kubectl get nodes -o custom-columns=NAME:.metadata.name,TAINTS:.spec.taints --no-headers >&2 || true
  kubectl describe nodes >&2 || true
  kubectl -n "${NAMESPACE}" get pods,pvc >&2 || true
  kubectl -n "${NAMESPACE}" describe statefulset/minio >&2 || true
  kubectl -n "${NAMESPACE}" describe pod -l app=minio >&2 || true
  kubectl -n "${NAMESPACE}" logs -l app=minio --all-containers --tail=200 >&2 || true
  kubectl -n "${NAMESPACE}" get events --sort-by=.lastTimestamp | tail -n 40 >&2 || true
}

dump_trailbase_debug() {
  echo "[trailbase-debug] deployment state" >&2
  kubectl -n "${NAMESPACE}" get deploy,pods >&2 || true
  kubectl -n "${NAMESPACE}" describe deployment/trailbase >&2 || true
  kubectl -n "${NAMESPACE}" describe pod -l app=trailbase >&2 || true
  kubectl -n "${NAMESPACE}" logs -l app=trailbase --all-containers --tail=200 >&2 || true
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

kubectl -n "${NAMESPACE}" create secret generic certd-secrets \
  --from-literal=CERTD_MASTER_KEY="${CERTD_MASTER_KEY}" \
  --from-literal=HQL_SECRET_RAFT="${HQL_SECRET_RAFT}" \
  --from-literal=HQL_SECRET_API="${HQL_SECRET_API}" \
  --from-literal=TRAILBASE_CERTD_INTERNAL_SECRET="${TRAILBASE_CERTD_INTERNAL_SECRET}" \
  --from-literal=ENC_KEYS="${ENC_KEYS}" \
  --from-literal=ENC_KEY_ACTIVE="${ENC_KEY_ACTIVE}" \
  --from-literal=HQL_S3_KEY="${MINIO_ROOT_USER}" \
  --from-literal=HQL_S3_SECRET="${MINIO_ROOT_PASSWORD}" \
  --dry-run=client -o yaml | kubectl apply -f -

kubectl -n "${NAMESPACE}" create secret generic trailbase-backup-secret \
  --from-literal=TRAILBASE_CERTD_INTERNAL_SECRET="${TRAILBASE_CERTD_INTERNAL_SECRET}" \
  --from-literal=LITESTREAM_ACCESS_KEY_ID="${MINIO_ROOT_USER}" \
  --from-literal=LITESTREAM_SECRET_ACCESS_KEY="${MINIO_ROOT_PASSWORD}" \
  --dry-run=client -o yaml | kubectl apply -f -

kubectl -n "${NAMESPACE}" create secret generic minio-secrets \
  --from-literal=MINIO_ROOT_USER="${MINIO_ROOT_USER}" \
  --from-literal=MINIO_ROOT_PASSWORD="${MINIO_ROOT_PASSWORD}" \
  --dry-run=client -o yaml | kubectl apply -f -

apply_resource "${KUSTOMIZE_DIR}/namespace.yaml"

if [[ "${VIND_SKIP_MINIO}" != "1" ]]; then
  minio_statefulset_manifest="${KUSTOMIZE_DIR}/minio-statefulset.yaml"
  if [[ "${VIND_MINIO_STORAGE_MODE}" == "ephemeral" ]]; then
    minio_statefulset_manifest="${KUSTOMIZE_DIR}/minio-statefulset-ephemeral.yaml"
  fi
  apply_resource "${KUSTOMIZE_DIR}/minio-headless-service.yaml"
  apply_resource "${KUSTOMIZE_DIR}/minio-service.yaml"
  apply_resource "${KUSTOMIZE_DIR}/minio-pdb.yaml"
  apply_resource "${minio_statefulset_manifest}"
fi

if [[ "${VIND_SKIP_MINIO}" != "1" ]]; then
  wait_for_resource statefulset minio
  if ! kubectl -n "${NAMESPACE}" rollout status statefulset/minio --timeout=15m; then
    dump_minio_debug
    exit 1
  fi

  kubectl -n "${NAMESPACE}" delete job minio-bootstrap --ignore-not-found
  for _ in $(seq 1 60); do
    if ! kubectl -n "${NAMESPACE}" get job minio-bootstrap >/dev/null 2>&1; then
      break
    fi
    sleep 1
  done
  if kubectl -n "${NAMESPACE}" get job minio-bootstrap >/dev/null 2>&1; then
    echo "timed out waiting for minio-bootstrap job deletion" >&2
    exit 1
  fi
  minio_job_file="$(mktemp "${TMPDIR:-/tmp}/minio-bootstrap.XXXXXX")"
  sed "s/__NAMESPACE__/${NAMESPACE}/g" "${KUSTOMIZE_DIR}/minio-bootstrap-job.yaml" >"${minio_job_file}"
  kubectl -n "${NAMESPACE}" apply -f "${minio_job_file}"
  rm -f "${minio_job_file}"

  kubectl -n "${NAMESPACE}" wait --for=condition=complete job/minio-bootstrap --timeout=10m
else
  echo "[skip] minio deployment"
fi

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
apply_resource "${KUSTOMIZE_DIR}/app-learner-1-service.yaml"
apply_resource "${KUSTOMIZE_DIR}/app-learner-2-service.yaml"
apply_resource "${KUSTOMIZE_DIR}/app-learner-3-service.yaml"
apply_resource "${KUSTOMIZE_DIR}/app-learner-1-deployment.yaml"
apply_resource "${KUSTOMIZE_DIR}/app-learner-2-deployment.yaml"
apply_resource "${KUSTOMIZE_DIR}/app-learner-3-deployment.yaml"

wait_for_resource statefulset certd-voter
kubectl -n "${NAMESPACE}" rollout status statefulset/certd-voter --timeout=15m
for deployment in certd-learner-1 certd-learner-2 certd-learner-3; do
  wait_for_resource deployment "${deployment}"
  kubectl -n "${NAMESPACE}" rollout status "deployment/${deployment}" --timeout=15m
done

cat <<EOF
vind bootstrap completed.

Context: ${KUBECONTEXT:-$(kubectl config current-context)}
Namespace: ${NAMESPACE}
Image: ${IMAGE_NAME}
TrailBase Image: ${TRAILBASE_IMAGE}

Next:
  export CERTD_MASTER_KEY=${CERTD_MASTER_KEY}
  ${ROOT_DIR}/scripts/k3d-e2e.sh
  ${ROOT_DIR}/scripts/vind-chaos.sh
EOF
