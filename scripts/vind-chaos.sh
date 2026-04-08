#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
KUBE_CONTEXT="${KUBE_CONTEXT:-}"
NAMESPACE="${NAMESPACE:-certd}"
LOCAL_PORT="${LOCAL_PORT:-18080}"
LEARNER_LOCAL_PORT="${LEARNER_LOCAL_PORT:-18081}"
CERTD_CHAOS_WITH_LOAD="${CERTD_CHAOS_WITH_LOAD:-1}"
VIND_EXEC_ACTIVE="${VIND_EXEC_ACTIVE:-0}"

need() {
  command -v "$1" >/dev/null 2>&1 || {
    echo "missing command: $1" >&2
    exit 1
  }
}

need kubectl

if [[ -z "${KUBE_CONTEXT}" ]]; then
  if [[ "${VIND_EXEC_ACTIVE}" != "1" ]]; then
    echo "missing KUBE_CONTEXT; run via scripts/vind-exec.sh or pass KUBE_CONTEXT explicitly" >&2
    exit 1
  fi
  KUBE_CONTEXT="$(kubectl config current-context)"
fi

kubectl config use-context "${KUBE_CONTEXT}" >/dev/null

NAMESPACE="${NAMESPACE}" \
LOCAL_PORT="${LOCAL_PORT}" \
LEARNER_LOCAL_PORT="${LEARNER_LOCAL_PORT}" \
CERTD_CHAOS_INCLUDE_LEARNER_PARTITION=0 \
CERTD_CHAOS_INCLUDE_MINIO=0 \
CERTD_CHAOS_WITH_LOAD="${CERTD_CHAOS_WITH_LOAD}" \
  bash "${ROOT_DIR}/scripts/k3d-chaos.sh"
