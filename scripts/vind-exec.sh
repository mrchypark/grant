#!/usr/bin/env bash
set -euo pipefail

CLUSTER_NAME="${CLUSTER_NAME:-certd-ci}"

if [[ "$#" -eq 0 ]]; then
  echo "usage: scripts/vind-exec.sh <command> [args...]" >&2
  exit 1
fi

exec env VIND_EXEC_ACTIVE=1 vcluster connect "${CLUSTER_NAME}" \
  --driver docker \
  --background-proxy=false \
  -- "$@"
