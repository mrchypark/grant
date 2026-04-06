#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
KUBE_CONTEXT="${KUBE_CONTEXT:-gke_ied-cluster}"
NAMESPACE="${NAMESPACE:-certd-chaos}"
LOCAL_PORT="${LOCAL_PORT:-18080}"
LEARNER_LOCAL_PORT="${LEARNER_LOCAL_PORT:-18081}"
CERTD_CHAOS_INCLUDE_LEARNER_PARTITION="${CERTD_CHAOS_INCLUDE_LEARNER_PARTITION:-0}"
CERTD_CHAOS_INCLUDE_MINIO="${CERTD_CHAOS_INCLUDE_MINIO:-0}"

need() {
  command -v "$1" >/dev/null 2>&1 || {
    echo "missing command: $1" >&2
    exit 1
  }
}

need kubectl

if [[ "${KUBE_CONTEXT}" != "$(kubectl config current-context 2>/dev/null || true)" ]]; then
  kubectl config use-context "${KUBE_CONTEXT}" >/dev/null
fi

NAMESPACE="${NAMESPACE}" \
LOCAL_PORT="${LOCAL_PORT}" \
LEARNER_LOCAL_PORT="${LEARNER_LOCAL_PORT}" \
CERTD_CHAOS_INCLUDE_LEARNER_PARTITION=0 \
CERTD_CHAOS_INCLUDE_MINIO="${CERTD_CHAOS_INCLUDE_MINIO}" \
  bash "${ROOT_DIR}/scripts/k3d-chaos.sh"

if [[ "${CERTD_CHAOS_INCLUDE_LEARNER_PARTITION}" == "1" ]]; then
  KUBE_CONTEXT="${KUBE_CONTEXT}" \
  NAMESPACE="${NAMESPACE}" \
  LOCAL_PORT="${LOCAL_PORT}" \
    bash "${ROOT_DIR}/scripts/gke-chaos-partition.sh"
fi
