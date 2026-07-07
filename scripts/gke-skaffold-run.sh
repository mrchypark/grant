#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
KUBE_CONTEXT="${KUBE_CONTEXT:-gke_ied-cluster}"
NAMESPACE="${NAMESPACE:-grant-chaos}"
SKAFFOLD_PROFILE="${SKAFFOLD_PROFILE:-ied}"
SKAFFOLD_MODULE="${SKAFFOLD_MODULE:-grant}"
SKAFFOLD_EXTRA_ARGS="${SKAFFOLD_EXTRA_ARGS:-}"

GRANT_MASTER_KEY="${GRANT_MASTER_KEY:-local-dev-master-key}"
HQL_SECRET_RAFT="${HQL_SECRET_RAFT:-SuperSecureSecret1337}"
HQL_SECRET_API="${HQL_SECRET_API:-AnotherSecureSecret42}"
GRANT_RAUTHY_CLIENT_ID="${GRANT_RAUTHY_CLIENT_ID:-grant}"
GRANT_RAUTHY_CLIENT_SECRET="${GRANT_RAUTHY_CLIENT_SECRET:-dev-rauthy-client-secret-000000000000000000000000000000000000000000}"
ENC_KEYS="${ENC_KEYS:-devkey01/MDEyMzQ1Njc4OWFiY2RlZjAxMjM0NTY3ODlhYmNkZWY=}"
ENC_KEY_ACTIVE="${ENC_KEY_ACTIVE:-devkey01}"

# shellcheck source=scripts/k8s-secret-helpers.sh
. "${ROOT_DIR}/scripts/k8s-secret-helpers.sh"

need() {
  command -v "$1" >/dev/null 2>&1 || {
    echo "missing command: $1" >&2
    exit 1
  }
}

need kubectl
need skaffold

if [[ "${SKAFFOLD_PROFILE}" == "ied" && "${NAMESPACE}" != "grant-chaos" ]]; then
  echo "the ied Skaffold profile deploys the grant-chaos overlay; set NAMESPACE=grant-chaos or add a dedicated overlay" >&2
  exit 1
fi

kubectl config use-context "${KUBE_CONTEXT}" >/dev/null
kubectl --context "${KUBE_CONTEXT}" create namespace "${NAMESPACE}" --dry-run=client -o yaml \
  | kubectl --context "${KUBE_CONTEXT}" apply -f -
apply_grant_secret

read -r -a extra_args <<<"${SKAFFOLD_EXTRA_ARGS}"

skaffold run \
  -f "${ROOT_DIR}/skaffold.yaml" \
  --module "${SKAFFOLD_MODULE}" \
  --profile "${SKAFFOLD_PROFILE}" \
  --kube-context "${KUBE_CONTEXT}" \
  "${extra_args[@]}"
