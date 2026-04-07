#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "${ROOT_DIR}"

fail() {
  echo "FAIL: $*" >&2
  exit 1
}

require_contains() {
  local file="$1"
  local pattern="$2"
  if ! grep -Fq -- "${pattern}" "${file}"; then
    fail "${file} missing expected text: ${pattern}"
  fi
}

require_not_contains() {
  local file="$1"
  local pattern="$2"
  if grep -Fq -- "${pattern}" "${file}"; then
    fail "${file} unexpectedly contains: ${pattern}"
  fi
}

# shellcheck disable=SC2016
require_contains "scripts/ci-chaos-local.sh" 'CI_CLUSTER_BACKEND="${CI_CLUSTER_BACKEND:-vind}"'
# shellcheck disable=SC2016
require_contains "scripts/ci-chaos-local.sh" 'VIND_MINIO_STORAGE_MODE="${VIND_MINIO_STORAGE_MODE:-ephemeral}"'
# shellcheck disable=SC2016
require_contains "scripts/ci-chaos-local.sh" 'bash "${ROOT_DIR}/scripts/vind-bootstrap.sh"'
# shellcheck disable=SC2016
require_contains "scripts/ci-chaos-local.sh" 'bash "${ROOT_DIR}/scripts/vind-chaos.sh"'
# shellcheck disable=SC2016
require_not_contains "scripts/ci-chaos-local.sh" 'bash "${ROOT_DIR}/scripts/kind-bootstrap.sh"'
# shellcheck disable=SC2016
require_not_contains "scripts/ci-chaos-local.sh" 'bash "${ROOT_DIR}/scripts/kind-chaos.sh"'
require_not_contains "scripts/ci-chaos-local.sh" 'need kind'

require_contains ".github/workflows/chaos-local.yml" 'Install vCluster CLI'
require_contains ".github/workflows/chaos-local.yml" 'vcluster version'
require_contains ".github/workflows/chaos-local.yml" 'CI_CLUSTER_BACKEND: vind'
require_not_contains ".github/workflows/chaos-local.yml" 'Install kind'
require_not_contains ".github/workflows/chaos-local.yml" 'setup-vind'
# shellcheck disable=SC2016
require_contains "scripts/k3d-chaos-load.sh" 'HTTP_CURL_TIMEOUT_SECONDS="${HTTP_CURL_TIMEOUT_SECONDS:-5}"'
# shellcheck disable=SC2016
require_contains "scripts/k3d-chaos-load.sh" '--max-time "${HTTP_CURL_TIMEOUT_SECONDS}"'
# shellcheck disable=SC2016
require_contains "scripts/k3d-chaos-load.sh" '--connect-timeout "${HTTP_CONNECT_TIMEOUT_SECONDS}"'
require_contains "scripts/vind-bootstrap.sh" 'minio-statefulset-ephemeral.yaml'
require_contains "scripts/vind-bootstrap.sh" 'dump_minio_debug'

if [[ -f ".github/workflows/chaos-vind.yml" ]]; then
  fail ".github/workflows/chaos-vind.yml should be removed once vind is the default workflow"
fi

for stale_file in \
  "scripts/kind-bootstrap.sh" \
  "scripts/kind-chaos.sh" \
  "scripts/dev-kind-up.sh" \
  "scripts/dev-kind-down.sh"
do
  if [[ -f "${stale_file}" ]]; then
    fail "${stale_file} should be removed once vind is the only local chaos backend"
  fi
done

echo "vind default chaos wiring looks correct"
