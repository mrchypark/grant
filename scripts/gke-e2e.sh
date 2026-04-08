#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
KUBE_CONTEXT="${KUBE_CONTEXT:-gke_ied-cluster}"
NAMESPACE="${NAMESPACE:-certd-chaos}"
LOCAL_PORT="${LOCAL_PORT:-18082}"

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

NAMESPACE="${NAMESPACE}" LOCAL_PORT="${LOCAL_PORT}" bash "${ROOT_DIR}/scripts/k3d-e2e.sh"
