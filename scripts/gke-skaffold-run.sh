#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
KUBE_CONTEXT="${KUBE_CONTEXT:-gke_ied-cluster}"
NAMESPACE="${NAMESPACE:-certd-chaos}"
SKAFFOLD_PROFILE="${SKAFFOLD_PROFILE:-ied}"
SKAFFOLD_MODULE="${SKAFFOLD_MODULE:-certd}"
SKAFFOLD_EXTRA_ARGS="${SKAFFOLD_EXTRA_ARGS:-}"

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
need skaffold
need python3

generated_overlay_dir=""
generated_skaffold_file=""

cleanup() {
  if [[ -n "${generated_overlay_dir}" ]]; then
    rm -rf "${generated_overlay_dir}"
  fi
  if [[ -n "${generated_skaffold_file}" ]]; then
    rm -f "${generated_skaffold_file}"
  fi
}
trap cleanup EXIT

prepare_skaffold_config() {
  if [[ "${SKAFFOLD_PROFILE}" != "ied" || "${NAMESPACE}" == "certd-chaos" ]]; then
    printf '%s\n' "${ROOT_DIR}/skaffold.yaml"
    return 0
  fi

  generated_overlay_dir="$(mktemp -d "${ROOT_DIR}/deploy/overlays/ied-render.XXXXXX")"
  generated_skaffold_file="$(mktemp "${ROOT_DIR}/skaffold.ied-render.XXXXXX.yaml")"

  cp "${ROOT_DIR}/deploy/overlays/ied/kustomization.yaml" "${generated_overlay_dir}/kustomization.yaml"
  python3 -c '
from pathlib import Path
import sys
namespace = sys.argv[1]
overlay = Path(sys.argv[2])
overlay.write_text(overlay.read_text().replace("certd-chaos", namespace))
' "${NAMESPACE}" "${generated_overlay_dir}/kustomization.yaml"

  python3 -c '
from pathlib import Path
import sys
skaffold = Path(sys.argv[1])
destination = Path(sys.argv[2])
overlay_path = Path(sys.argv[3]).relative_to(Path(sys.argv[4]))
destination.write_text(
    skaffold.read_text().replace("- deploy/overlays/ied", f"- {overlay_path.as_posix()}")
)
' "${ROOT_DIR}/skaffold.yaml" "${generated_skaffold_file}" "${generated_overlay_dir}" "${ROOT_DIR}"

  printf '%s\n' "${generated_skaffold_file}"
}

kubectl config use-context "${KUBE_CONTEXT}" >/dev/null

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

read -r -a extra_args <<<"${SKAFFOLD_EXTRA_ARGS}"
skaffold_config="$(prepare_skaffold_config)"

skaffold run \
  -f "${skaffold_config}" \
  --module "${SKAFFOLD_MODULE}" \
  --profile "${SKAFFOLD_PROFILE}" \
  --kube-context "${KUBE_CONTEXT}" \
  "${extra_args[@]}"
