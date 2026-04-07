#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
KUBE_CONTEXT="${KUBE_CONTEXT:-}"
NAMESPACE="${NAMESPACE:-certd}"
LOCAL_PORT="${LOCAL_PORT:-18080}"
LEARNER_LOCAL_PORT="${LEARNER_LOCAL_PORT:-18081}"
CERTD_CHAOS_WITH_LOAD="${CERTD_CHAOS_WITH_LOAD:-1}"

need() {
  command -v "$1" >/dev/null 2>&1 || {
    echo "missing command: $1" >&2
    exit 1
  }
}

need kubectl

if [[ -z "${KUBE_CONTEXT}" ]]; then
  echo "missing KUBE_CONTEXT" >&2
  exit 1
fi

kubectl config use-context "${KUBE_CONTEXT}" >/dev/null

NAMESPACE="${NAMESPACE}" \
LOCAL_PORT="${LOCAL_PORT}" \
LEARNER_LOCAL_PORT="${LEARNER_LOCAL_PORT}" \
CERTD_CHAOS_INCLUDE_LEARNER_PARTITION=0 \
CERTD_CHAOS_INCLUDE_MINIO=0 \
CERTD_CHAOS_WITH_LOAD="${CERTD_CHAOS_WITH_LOAD}" \
  bash "${ROOT_DIR}/scripts/k3d-chaos.sh"
