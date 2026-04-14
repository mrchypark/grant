#!/usr/bin/env bash
set -euo pipefail

CLUSTER_NAME="${CLUSTER_NAME:-certd-ci}"

if [[ "$#" -eq 0 ]]; then
  echo "usage: scripts/vind-exec.sh <command> [args...]" >&2
  exit 1
fi

vcluster_container_name() {
  printf 'vcluster.cp.%s' "${CLUSTER_NAME}"
}

if docker inspect "$(vcluster_container_name)" >/dev/null 2>&1; then
  exec env VIND_EXEC_ACTIVE=1 vcluster connect "${CLUSTER_NAME}" \
    --driver docker \
    --background-proxy=false \
    -- "$@"
fi

if command -v k3d >/dev/null 2>&1 && k3d cluster get "${CLUSTER_NAME}" >/dev/null 2>&1; then
  context_name="k3d-${CLUSTER_NAME}"
  kubectl config use-context "k3d-${CLUSTER_NAME}" >/dev/null
  exec env \
    VIND_EXEC_ACTIVE=1 \
    KUBECONTEXT="${context_name}" \
    KUBE_CONTEXT="${context_name}" \
    "$@"
fi

echo "vind cluster ${CLUSTER_NAME} not found via vcluster or k3d" >&2
exit 1
