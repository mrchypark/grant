#!/usr/bin/env bash
set -euo pipefail

CLUSTER_NAME="${CLUSTER_NAME:-grant-ci}"

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

echo "vind cluster ${CLUSTER_NAME} not found via vcluster" >&2
exit 1
