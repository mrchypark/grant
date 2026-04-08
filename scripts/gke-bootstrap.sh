#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
KUBE_CONTEXT="${KUBE_CONTEXT:-gke_ied-cluster}"
NAMESPACE="${NAMESPACE:-certd-chaos}"
KUSTOMIZE_DIR="${KUSTOMIZE_DIR:-$ROOT_DIR/deploy/k8s}"
DRY_RUN="${DRY_RUN:-0}"
APPLY_INGRESS="${APPLY_INGRESS:-0}"
RENDER_ONLY="${RENDER_ONLY:-0}"
RENDER_OUTPUT_DIR="${RENDER_OUTPUT_DIR:-}"

IMAGE_NAME="${IMAGE_NAME:-certd:local}"
TRAILBASE_IMAGE="${TRAILBASE_IMAGE:-trailbase-auth-bridge:local}"
LITESTREAM_IMAGE="${LITESTREAM_IMAGE:-litestream/litestream:0.3.14}"

CERTD_MASTER_KEY="${CERTD_MASTER_KEY:-local-dev-master-key}"
HQL_SECRET_RAFT="${HQL_SECRET_RAFT:-SuperSecureSecret1337}"
HQL_SECRET_API="${HQL_SECRET_API:-AnotherSecureSecret42}"
TRAILBASE_CERTD_INTERNAL_SECRET="${TRAILBASE_CERTD_INTERNAL_SECRET:-dev-trailbase-internal-secret}"
ENC_KEYS="${ENC_KEYS:-devkey01/MDEyMzQ1Njc4OWFiY2RlZjAxMjM0NTY3ODlhYmNkZWY=}"
ENC_KEY_ACTIVE="${ENC_KEY_ACTIVE:-devkey01}"
MINIO_ROOT_USER="${MINIO_ROOT_USER:-minioadmin}"
MINIO_ROOT_PASSWORD="${MINIO_ROOT_PASSWORD:-minioadmin}"

need() {
  command -v "$1" >/dev/null 2>&1 || {
    echo "missing command: $1" >&2
    exit 1
  }
}

need kubectl
need jq
need base64
need python3

kubectl_ctx() {
  kubectl --context "${KUBE_CONTEXT}" "$@"
}

render_or_apply_secret_generic() {
  local name="$1"
  shift

  if [[ "${RENDER_ONLY}" == "1" ]]; then
    kubectl create secret generic "${name}" \
      -n "${NAMESPACE}" \
      "$@" \
      --dry-run=client -o yaml > "${RENDER_OUTPUT_DIR}/${name}.yaml"
    return 0
  fi

  kubectl_ctx -n "${NAMESPACE}" create secret generic "${name}" \
    "$@" \
    --dry-run=client -o yaml | kubectl_ctx apply "${kubectl_apply_args[@]}" -f -
}

apply_namespaced() {
  local file_path="$1"
  if [[ "${RENDER_ONLY}" == "1" ]]; then
    local output_dir="${RENDER_OUTPUT_DIR}"
    if [[ -z "${output_dir}" ]]; then
      output_dir="$(mktemp -d "${TMPDIR:-/tmp}/gke-bootstrap-render.XXXXXX")"
      RENDER_OUTPUT_DIR="${output_dir}"
    fi
    render_manifest "${file_path}" > "${output_dir}/$(basename "${file_path}")"
    return 0
  fi
  render_manifest "${file_path}" | kubectl_ctx -n "${NAMESPACE}" apply "${kubectl_apply_args[@]}" -f -
}

render_manifest() {
  local file_path="$1"
  python3 -c '
from pathlib import Path
import sys
path = Path(sys.argv[1])
namespace = sys.argv[2]
image_name = sys.argv[3]
trailbase_image = sys.argv[4]
litestream_image = sys.argv[5]
text = path.read_text()
text = text.replace(".certd.svc.cluster.local", f".{namespace}.svc.cluster.local")
text = text.replace("http://minio.certd.svc.cluster.local:9000", f"http://minio.{namespace}.svc.cluster.local:9000")
text = text.replace("http://trailbase.certd.svc.cluster.local:4000", f"http://trailbase.{namespace}.svc.cluster.local:4000")
text = text.replace("image: certd:local", f"image: {image_name}")
text = text.replace("image: trailbase-auth-bridge:local", f"image: {trailbase_image}")
text = text.replace("image: litestream/litestream:0.3.14", f"image: {litestream_image}")
sys.stdout.write(text)
' "$file_path" "${NAMESPACE}" "${IMAGE_NAME}" "${TRAILBASE_IMAGE}" "${LITESTREAM_IMAGE}"
}

wait_for_resource() {
  local kind="$1"
  local name="$2"
  local attempts="${3:-60}"
  local delay="${4:-2}"
  for _ in $(seq 1 "${attempts}"); do
    if kubectl_ctx -n "${NAMESPACE}" get "${kind}" "${name}" >/dev/null 2>&1; then
      return 0
    fi
    sleep "${delay}"
  done
  echo "timed out waiting for ${kind}/${name} in namespace ${NAMESPACE}" >&2
  exit 1
}

if [[ "${DRY_RUN}" == "1" ]]; then
  kubectl_apply_args=(--dry-run=client -o yaml)
else
  kubectl_apply_args=()
fi

if [[ "${RENDER_ONLY}" == "1" ]]; then
  if [[ -z "${RENDER_OUTPUT_DIR}" ]]; then
    RENDER_OUTPUT_DIR="$(mktemp -d "${TMPDIR:-/tmp}/gke-bootstrap-render.XXXXXX")"
  else
    mkdir -p "${RENDER_OUTPUT_DIR}"
  fi
fi

if [[ "${RENDER_ONLY}" != "1" && "${KUBE_CONTEXT}" != "$(kubectl config current-context 2>/dev/null || true)" ]]; then
  kubectl config use-context "${KUBE_CONTEXT}" >/dev/null
fi

if [[ "${RENDER_ONLY}" == "1" ]]; then
  cat > "${RENDER_OUTPUT_DIR}/namespace.yaml" <<EOF
apiVersion: v1
kind: Namespace
metadata:
  name: ${NAMESPACE}
EOF
else
  kubectl_ctx create namespace "${NAMESPACE}" --dry-run=client -o yaml | kubectl_ctx apply "${kubectl_apply_args[@]}" -f -
fi

render_or_apply_secret_generic certd-secrets \
  --from-literal=CERTD_MASTER_KEY="${CERTD_MASTER_KEY}" \
  --from-literal=HQL_SECRET_RAFT="${HQL_SECRET_RAFT}" \
  --from-literal=HQL_SECRET_API="${HQL_SECRET_API}" \
  --from-literal=TRAILBASE_CERTD_INTERNAL_SECRET="${TRAILBASE_CERTD_INTERNAL_SECRET}" \
  --from-literal=ENC_KEYS="${ENC_KEYS}" \
  --from-literal=ENC_KEY_ACTIVE="${ENC_KEY_ACTIVE}" \
  --from-literal=HQL_S3_KEY="${MINIO_ROOT_USER}" \
  --from-literal=HQL_S3_SECRET="${MINIO_ROOT_PASSWORD}"

render_or_apply_secret_generic trailbase-backup-secret \
  --from-literal=TRAILBASE_CERTD_INTERNAL_SECRET="${TRAILBASE_CERTD_INTERNAL_SECRET}" \
  --from-literal=LITESTREAM_ACCESS_KEY_ID="${MINIO_ROOT_USER}" \
  --from-literal=LITESTREAM_SECRET_ACCESS_KEY="${MINIO_ROOT_PASSWORD}"

render_or_apply_secret_generic minio-secrets \
  --from-literal=MINIO_ROOT_USER="${MINIO_ROOT_USER}" \
  --from-literal=MINIO_ROOT_PASSWORD="${MINIO_ROOT_PASSWORD}"

apply_namespaced "${KUSTOMIZE_DIR}/minio-headless-service.yaml"
apply_namespaced "${KUSTOMIZE_DIR}/minio-service.yaml"
apply_namespaced "${KUSTOMIZE_DIR}/minio-pdb.yaml"
apply_namespaced "${KUSTOMIZE_DIR}/minio-statefulset.yaml"

if [[ "${DRY_RUN}" != "1" && "${RENDER_ONLY}" != "1" ]]; then
  wait_for_resource statefulset minio
  kubectl_ctx -n "${NAMESPACE}" rollout status statefulset/minio --timeout=15m
fi

if [[ "${RENDER_ONLY}" != "1" ]]; then
  kubectl_ctx -n "${NAMESPACE}" delete job minio-bootstrap --ignore-not-found >/dev/null 2>&1 || true
  kubectl_ctx -n "${NAMESPACE}" apply "${kubectl_apply_args[@]}" -f - <<EOF
apiVersion: batch/v1
kind: Job
metadata:
  name: minio-bootstrap
spec:
  backoffLimit: 6
  template:
    spec:
      restartPolicy: OnFailure
      containers:
        - name: mc
          image: minio/mc:latest
          env:
            - name: MINIO_ROOT_USER
              valueFrom:
                secretKeyRef:
                  name: minio-secrets
                  key: MINIO_ROOT_USER
            - name: MINIO_ROOT_PASSWORD
              valueFrom:
                secretKeyRef:
                  name: minio-secrets
                  key: MINIO_ROOT_PASSWORD
          command: ["/bin/sh", "-lc"]
          args:
            - |
              set -e
              until mc alias set local http://minio.${NAMESPACE}.svc.cluster.local:9000 "\$MINIO_ROOT_USER" "\$MINIO_ROOT_PASSWORD"; do
                sleep 2
              done
              mc mb -p local/cert-artifacts || true
              mc mb -p local/cert-backups || true
              mc mb -p local/trailbase-backups || true
EOF
fi

if [[ "${DRY_RUN}" != "1" && "${RENDER_ONLY}" != "1" ]]; then
  kubectl_ctx -n "${NAMESPACE}" wait --for=condition=complete job/minio-bootstrap --timeout=10m
fi

apply_namespaced "${KUSTOMIZE_DIR}/trailbase-configmap.yaml"
apply_namespaced "${KUSTOMIZE_DIR}/trailbase-service.yaml"
apply_namespaced "${KUSTOMIZE_DIR}/trailbase-deployment.yaml"

if [[ "${DRY_RUN}" != "1" && "${RENDER_ONLY}" != "1" ]]; then
  wait_for_resource deployment trailbase
  kubectl_ctx -n "${NAMESPACE}" rollout status deployment/trailbase --timeout=15m
fi

apply_namespaced "${KUSTOMIZE_DIR}/app-configmap.yaml"
apply_namespaced "${KUSTOMIZE_DIR}/app-headless-service.yaml"
apply_namespaced "${KUSTOMIZE_DIR}/app-service.yaml"
apply_namespaced "${KUSTOMIZE_DIR}/app-pdb.yaml"
apply_namespaced "${KUSTOMIZE_DIR}/app-statefulset.yaml"
apply_namespaced "${KUSTOMIZE_DIR}/app-learner-1-service.yaml"
apply_namespaced "${KUSTOMIZE_DIR}/app-learner-2-service.yaml"
apply_namespaced "${KUSTOMIZE_DIR}/app-learner-3-service.yaml"
apply_namespaced "${KUSTOMIZE_DIR}/app-learner-1-deployment.yaml"
apply_namespaced "${KUSTOMIZE_DIR}/app-learner-2-deployment.yaml"
apply_namespaced "${KUSTOMIZE_DIR}/app-learner-3-deployment.yaml"

if [[ "${APPLY_INGRESS}" == "1" ]]; then
  apply_namespaced "${KUSTOMIZE_DIR}/ingress.yaml"
fi

if [[ "${DRY_RUN}" != "1" && "${RENDER_ONLY}" != "1" ]]; then
  wait_for_resource statefulset certd-voter
  kubectl_ctx -n "${NAMESPACE}" rollout status statefulset/certd-voter --timeout=15m
  for deployment in certd-learner-1 certd-learner-2 certd-learner-3; do
    wait_for_resource deployment "${deployment}"
    kubectl_ctx -n "${NAMESPACE}" rollout status "deployment/${deployment}" --timeout=15m
  done
fi

cat <<EOF
GKE bootstrap prepared.

Context: ${KUBE_CONTEXT}
Namespace: ${NAMESPACE}
Image: ${IMAGE_NAME}
TrailBase Image: ${TRAILBASE_IMAGE}
Litestream Image: ${LITESTREAM_IMAGE}
Dry Run: ${DRY_RUN}
Render Only: ${RENDER_ONLY}
Render Output Dir: ${RENDER_OUTPUT_DIR}

Next:
  KUBE_CONTEXT=${KUBE_CONTEXT} NAMESPACE=${NAMESPACE} CERTD_MASTER_KEY=${CERTD_MASTER_KEY} ${ROOT_DIR}/scripts/gke-e2e.sh
  KUBE_CONTEXT=${KUBE_CONTEXT} NAMESPACE=${NAMESPACE} CERTD_MASTER_KEY=${CERTD_MASTER_KEY} ${ROOT_DIR}/scripts/gke-chaos.sh
EOF
