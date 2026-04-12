#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "${ROOT_DIR}"

fail() {
  echo "FAIL: $*" >&2
  exit 1
}

expect_direct_invocation_failure() {
  local script="$1"
  local expected_text="$2"
  local err_file
  err_file="$(mktemp)"

  if bash "${script}" >/dev/null 2>"${err_file}"; then
    rm -f "${err_file}"
    fail "${script} unexpectedly succeeded without explicit vcluster context"
  fi

  if ! grep -Fq -- "${expected_text}" "${err_file}"; then
    cat "${err_file}" >&2
    rm -f "${err_file}"
    fail "${script} did not fail with expected message: ${expected_text}"
  fi

  rm -f "${err_file}"
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
require_contains "scripts/ci-chaos-local.sh" 'CI_DOCKER_BUILDER_PRUNE="${CI_DOCKER_BUILDER_PRUNE:-0}"'
require_contains "scripts/ci-chaos-local.sh" 'docker builder prune -af >/dev/null || true'
require_contains "scripts/ci-chaos-local.sh" 'docker image prune -f >/dev/null || true'
require_contains "scripts/ci-chaos-local.sh" 'log_storage_state'
# shellcheck disable=SC2016
require_contains "scripts/ci-chaos-local.sh" 'bash "${ROOT_DIR}/scripts/vind-exec.sh" bash "${ROOT_DIR}/scripts/vind-bootstrap.sh"'
# shellcheck disable=SC2016
require_contains "scripts/ci-chaos-local.sh" 'bash "${ROOT_DIR}/scripts/vind-exec.sh" bash "${ROOT_DIR}/scripts/k3d-e2e.sh"'
# shellcheck disable=SC2016
require_contains "scripts/ci-chaos-local.sh" 'bash "${ROOT_DIR}/scripts/vind-exec.sh" bash "${ROOT_DIR}/scripts/vind-chaos.sh"'
# shellcheck disable=SC2016
require_not_contains "scripts/ci-chaos-local.sh" 'bash "${ROOT_DIR}/scripts/kind-bootstrap.sh"'
# shellcheck disable=SC2016
require_not_contains "scripts/ci-chaos-local.sh" 'bash "${ROOT_DIR}/scripts/kind-chaos.sh"'
require_not_contains "scripts/ci-chaos-local.sh" 'need kind'

require_contains ".github/workflows/chaos-local.yml" 'Install vCluster CLI'
require_contains ".github/workflows/chaos-local.yml" 'Reclaim runner disk'
require_contains ".github/workflows/chaos-local.yml" 'vcluster version'
require_contains ".github/workflows/chaos-local.yml" 'CI_CLUSTER_BACKEND: vind'
require_contains ".github/workflows/chaos-local.yml" 'CI_DOCKER_BUILDER_PRUNE: "1"'
require_not_contains ".github/workflows/chaos-local.yml" 'Install kind'
require_not_contains ".github/workflows/chaos-local.yml" 'setup-vind'
# shellcheck disable=SC2016
require_contains "scripts/k3d-chaos-load.sh" 'HTTP_CURL_TIMEOUT_SECONDS="${HTTP_CURL_TIMEOUT_SECONDS:-5}"'
# shellcheck disable=SC2016
require_contains "scripts/k3d-chaos-load.sh" '--max-time "${HTTP_CURL_TIMEOUT_SECONDS}"'
# shellcheck disable=SC2016
require_contains "scripts/k3d-chaos-load.sh" '--connect-timeout "${HTTP_CONNECT_TIMEOUT_SECONDS}"'
# shellcheck disable=SC2016
require_contains "scripts/k3d-chaos.sh" 'HTTP_CONNECT_TIMEOUT_SECONDS="${HTTP_CONNECT_TIMEOUT_SECONDS:-2}"'
# shellcheck disable=SC2016
require_contains "scripts/k3d-chaos.sh" '--max-time "${HTTP_CURL_TIMEOUT_SECONDS}"'
# shellcheck disable=SC2016
require_contains "scripts/k3d-chaos.sh" '--connect-timeout "${HTTP_CONNECT_TIMEOUT_SECONDS}"'
# shellcheck disable=SC2016
require_contains "scripts/k3d-chaos.sh" 'capture_file="$(mktemp "${TMPDIR:-/tmp}/retry-capture.XXXXXX")"'
# shellcheck disable=SC2016
require_contains "scripts/k3d-chaos.sh" 'if "$@" >"${capture_file}"; then'
require_contains "scripts/k3d-chaos.sh" 'TRAILBASE_RECOVERY_ROLLOUT_TIMEOUT_SECONDS="${TRAILBASE_RECOVERY_ROLLOUT_TIMEOUT_SECONDS:-180}"'
require_contains "scripts/k3d-chaos.sh" 'dump_trailbase_debug()'
require_contains "scripts/k3d-chaos.sh" 'retry_capture_into recovered_token'
require_not_contains "scripts/k3d-chaos.sh" 'recovered_token="$(login_trailbase || true)"'
require_not_contains "scripts/k3d-chaos.sh" 'retry_json_query()'
require_contains "scripts/vind-bootstrap.sh" 'minio-statefulset-ephemeral.yaml'
require_contains "scripts/vind-bootstrap.sh" 'dump_minio_debug'
require_contains "scripts/vind-bootstrap.sh" 'kubectl get nodes -o custom-columns=NAME:.metadata.name,TAINTS:.spec.taints --no-headers'
require_contains "scripts/vind-bootstrap.sh" "vcluster.cp.%s"
require_contains "scripts/vind-bootstrap.sh" 'ctr -n k8s.io images import -'
# shellcheck disable=SC2016
require_contains "scripts/vind-bootstrap.sh" 'ensure_runtime_image "${IMAGE_NAME}"'
# shellcheck disable=SC2016
require_contains "scripts/vind-bootstrap.sh" 'ensure_runtime_image "${TRAILBASE_IMAGE}"'
# shellcheck disable=SC2016
require_contains "scripts/vind-cluster.sh" 'VIND_UNTAINT_CONTROL_PLANE_NODES="${VIND_UNTAINT_CONTROL_PLANE_NODES:-1}"'
require_contains "scripts/vind-cluster.sh" 'kubectl taint nodes --all node-role.kubernetes.io/control-plane:NoSchedule-'
require_contains "scripts/vind-cluster.sh" 'kubectl taint nodes --all node-role.kubernetes.io/master:NoSchedule-'
require_contains "scripts/vind-cluster.sh" 'remain NoSchedule-tainted after normalization'
# shellcheck disable=SC2016
require_contains "scripts/vind-cluster.sh" 'vcluster create "${CLUSTER_NAME}" --driver docker --connect=false'
require_contains "scripts/vind-exec.sh" '--background-proxy=false'
require_contains "scripts/vind-exec.sh" 'VIND_EXEC_ACTIVE=1'

expect_direct_invocation_failure "scripts/vind-bootstrap.sh" 'run via scripts/vind-exec.sh or pass KUBECONTEXT explicitly'
expect_direct_invocation_failure "scripts/vind-chaos.sh" 'run via scripts/vind-exec.sh or pass KUBE_CONTEXT explicitly'

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
