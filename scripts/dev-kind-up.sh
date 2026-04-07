#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
CLUSTER_NAME="${CLUSTER_NAME:-certd-dev}"
NAMESPACE="${NAMESPACE:-certd}"

CLUSTER_NAME="${CLUSTER_NAME}" \
NAMESPACE="${NAMESPACE}" \
RESET_CLUSTER="${RESET_CLUSTER:-0}" \
  bash "${ROOT_DIR}/scripts/kind-bootstrap.sh"
