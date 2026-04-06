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

skaffold run \
  --module "${SKAFFOLD_MODULE}" \
  --profile "${SKAFFOLD_PROFILE}" \
  --kube-context "${KUBE_CONTEXT}" \
  "${extra_args[@]}"
