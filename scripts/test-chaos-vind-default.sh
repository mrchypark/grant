#!/usr/bin/env bash
# shellcheck disable=SC2016
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

ci_run_e2e_pattern="CI_RUN_E2E=\"\${CI_RUN_E2E:-1}\""
ci_run_chaos_pattern="CI_RUN_CHAOS=\"\${CI_RUN_CHAOS:-1}\""
ci_skip_cluster_delete_pattern="CI_SKIP_CLUSTER_DELETE_ON_FAILURE=\"\${CI_SKIP_CLUSTER_DELETE_ON_FAILURE:-0}\""
trailbase_rollout_failfast_pattern="echo \"[trailbase] rollout did not complete within \${TRAILBASE_RECOVERY_ROLLOUT_TIMEOUT_SECONDS}s; failing fast\" >&2"
trailbase_data_dir_chown_pattern="chown -R 65532:65532 \"\${TRAILBASE_DATA_DIR}\""
trailbase_data_dir_chmod_pattern="chmod -R 0770 \"\${TRAILBASE_DATA_DIR}\""

# shellcheck disable=SC2016
require_contains "scripts/ci-chaos-local.sh" 'CI_CLUSTER_BACKEND="${CI_CLUSTER_BACKEND:-vind}"'
# shellcheck disable=SC2016
require_contains "scripts/ci-chaos-local.sh" 'VIND_MINIO_STORAGE_MODE="${VIND_MINIO_STORAGE_MODE:-ephemeral}"'
# shellcheck disable=SC2016
require_contains "scripts/ci-chaos-local.sh" 'CI_DOCKER_BUILDER_PRUNE="${CI_DOCKER_BUILDER_PRUNE:-0}"'
require_contains "scripts/ci-chaos-local.sh" "${ci_skip_cluster_delete_pattern}"
require_contains "scripts/ci-chaos-local.sh" "${ci_run_e2e_pattern}"
require_contains "scripts/ci-chaos-local.sh" "${ci_run_chaos_pattern}"
require_contains "scripts/ci-chaos-local.sh" 'docker builder prune -af >/dev/null || true'
require_contains "scripts/ci-chaos-local.sh" 'docker image prune -f >/dev/null || true'
require_contains "scripts/ci-chaos-local.sh" 'log_storage_state'
require_contains "scripts/ci-chaos-local.sh" 'run_phase()'
require_contains "scripts/ci-chaos-local.sh" '[phase] starting'
require_contains "scripts/ci-chaos-local.sh" '[phase] completed'
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
require_contains ".github/workflows/chaos-local.yml" 'CI_RUN_E2E: "0"'
require_contains ".github/workflows/chaos-local.yml" 'CI_RUN_CHAOS: "1"'
require_contains ".github/workflows/chaos-local.yml" 'CI_SKIP_CLUSTER_DELETE_ON_FAILURE: "1"'
require_not_contains ".github/workflows/chaos-local.yml" 'Install kind'
require_not_contains ".github/workflows/chaos-local.yml" 'setup-vind'

require_contains ".github/workflows/e2e-local.yml" 'name: e2e-local'
require_contains ".github/workflows/e2e-local.yml" 'Install vCluster CLI'
require_contains ".github/workflows/e2e-local.yml" 'CI_CLUSTER_BACKEND: vind'
require_contains ".github/workflows/e2e-local.yml" 'CI_DOCKER_BUILDER_PRUNE: "1"'
require_contains ".github/workflows/e2e-local.yml" 'CI_RUN_E2E: "1"'
require_contains ".github/workflows/e2e-local.yml" 'CI_RUN_CHAOS: "0"'
require_not_contains ".github/workflows/e2e-local.yml" 'Install kind'
# shellcheck disable=SC2016
require_contains "scripts/k3d-chaos-load.sh" 'HTTP_CURL_TIMEOUT_SECONDS="${HTTP_CURL_TIMEOUT_SECONDS:-5}"'
# shellcheck disable=SC2016
require_contains "scripts/k3d-chaos-load.sh" '--max-time "${HTTP_CURL_TIMEOUT_SECONDS}"'
# shellcheck disable=SC2016
require_contains "scripts/k3d-chaos-load.sh" '--connect-timeout "${HTTP_CONNECT_TIMEOUT_SECONDS}"'
require_contains "scripts/k3d-chaos-load.sh" 'WORKLOAD_TRAILBASE_LOCAL_PORT="${WORKLOAD_TRAILBASE_LOCAL_PORT:-14090}"'
require_contains "scripts/k3d-chaos-load.sh" '"/api/auth/v1/login"'
require_contains "scripts/k3d-chaos-load.sh" 'http://127.0.0.1:${WORKLOAD_TRAILBASE_LOCAL_PORT}'
require_contains "scripts/k3d-chaos-load.sh" 'reserve_local_port()'
require_contains "scripts/k3d-chaos-load.sh" 'validate_certd_health()'
require_contains "scripts/k3d-chaos-load.sh" 'lsof -nP -iTCP:'
require_contains "scripts/k3d-chaos-load.sh" 'WORKLOAD_VOTER_LOCAL_PORT="$(reserve_local_port "${WORKLOAD_VOTER_LOCAL_PORT}" "voter")"'
require_contains "scripts/k3d-chaos-load.sh" 'WORKLOAD_LEARNER_LOCAL_PORT="$(reserve_local_port "${WORKLOAD_LEARNER_LOCAL_PORT}" "learner")"'
require_contains "scripts/k3d-chaos-load.sh" 'WORKLOAD_TRAILBASE_LOCAL_PORT="$(reserve_local_port "${WORKLOAD_TRAILBASE_LOCAL_PORT}" "trailbase")"'
# shellcheck disable=SC2016
require_contains "scripts/k3d-chaos.sh" 'HTTP_CONNECT_TIMEOUT_SECONDS="${HTTP_CONNECT_TIMEOUT_SECONDS:-2}"'
# shellcheck disable=SC2016
require_contains "scripts/k3d-chaos.sh" '--max-time "${HTTP_CURL_TIMEOUT_SECONDS}"'
# shellcheck disable=SC2016
require_contains "scripts/k3d-chaos.sh" '--connect-timeout "${HTTP_CONNECT_TIMEOUT_SECONDS}"'
require_contains "scripts/k3d-chaos.sh" 'TRAILBASE_LOCAL_PORT="${TRAILBASE_LOCAL_PORT:-14000}"'
require_contains "scripts/k3d-chaos.sh" 'port-forward svc/trailbase "${TRAILBASE_LOCAL_PORT}:4000"'
require_contains "scripts/k3d-chaos.sh" 'http://127.0.0.1:${TRAILBASE_LOCAL_PORT}/api/auth/v1/login'
require_not_contains "scripts/k3d-chaos.sh" 'TRAILBASE_RECOVERED_AUTH_TOKEN=""'
require_not_contains "scripts/k3d-chaos.sh" 'auth_token="${TRAILBASE_RECOVERED_AUTH_TOKEN}"'
require_not_contains "scripts/k3d-chaos.sh" 'auth_token="$(scenario_trailbase_cache_ttl'
require_contains "scripts/k3d-chaos.sh" 'scenario_trailbase_cache_ttl recovered_auth_token "${auth_token}" "${active_org}" "${baseline_facility_code}"'
require_contains "scripts/k3d-chaos.sh" "printf -v \"\${__result_var}\" '%s' \"\${recovered_token}\""
require_contains "scripts/k3d-chaos.sh" 'reserve_local_port()'
require_contains "scripts/k3d-chaos.sh" 'validate_certd_health()'
require_contains "scripts/k3d-chaos.sh" 'validate_trailbase_health()'
require_contains "scripts/k3d-chaos.sh" 'lsof -nP -iTCP:'
require_contains "scripts/k3d-chaos.sh" 'LOCAL_PORT="$(reserve_local_port "${LOCAL_PORT}" "certd")"'
require_contains "scripts/k3d-chaos.sh" 'LEARNER_LOCAL_PORT="$(reserve_local_port "${LEARNER_LOCAL_PORT}" "learner")"'
require_contains "scripts/k3d-chaos.sh" 'TRAILBASE_LOCAL_PORT="$(reserve_local_port "${TRAILBASE_LOCAL_PORT}" "trailbase")"'
# shellcheck disable=SC2016
require_contains "scripts/k3d-chaos.sh" 'capture_file="$(mktemp "${TMPDIR:-/tmp}/retry-capture.XXXXXX")"'
# shellcheck disable=SC2016
require_contains "scripts/k3d-chaos.sh" 'if "$@" >"${capture_file}"; then'
# shellcheck disable=SC2016
require_contains "scripts/k3d-chaos.sh" 'TRAILBASE_RECOVERY_ROLLOUT_TIMEOUT_SECONDS="${TRAILBASE_RECOVERY_ROLLOUT_TIMEOUT_SECONDS:-180}"'
require_contains "scripts/k3d-chaos.sh" 'dump_trailbase_debug()'
require_contains "scripts/k3d-chaos.sh" "${trailbase_rollout_failfast_pattern}"
require_contains "scripts/k3d-chaos.sh" 'echo "[trailbase] login did not recover after rollout; failing fast" >&2'
# shellcheck disable=SC2016
require_not_contains "scripts/k3d-chaos.sh" 'recovered_token="$(login_trailbase || true)"'
require_not_contains "scripts/k3d-chaos.sh" '[trailbase] restored user login failed; attempting reseed fallback'
require_not_contains "scripts/k3d-chaos.sh" 'retry_json_query()'
require_contains "deploy/k8s/trailbase-deployment.yaml" "${trailbase_data_dir_chown_pattern}"
require_contains "deploy/k8s/trailbase-deployment.yaml" "${trailbase_data_dir_chmod_pattern}"
require_contains "scripts/vind-bootstrap.sh" 'minio-statefulset-ephemeral.yaml'
require_contains "scripts/vind-bootstrap.sh" 'dump_minio_debug'
require_contains "scripts/vind-bootstrap.sh" 'kubectl get nodes -o custom-columns=NAME:.metadata.name,TAINTS:.spec.taints --no-headers'
require_contains "scripts/vind-bootstrap.sh" "vcluster.cp.%s"
require_contains "scripts/vind-bootstrap.sh" 'ctr -n k8s.io images import -'
require_contains "scripts/vind-bootstrap.sh" 'k3d image import'
# shellcheck disable=SC2016
require_contains "scripts/vind-bootstrap.sh" 'ensure_runtime_image "${IMAGE_NAME}"'
# shellcheck disable=SC2016
require_contains "scripts/vind-bootstrap.sh" 'ensure_runtime_image "${TRAILBASE_IMAGE}"'
# shellcheck disable=SC2016
require_contains "scripts/vind-cluster.sh" 'VIND_UNTAINT_CONTROL_PLANE_NODES="${VIND_UNTAINT_CONTROL_PLANE_NODES:-1}"'
require_contains "scripts/vind-cluster.sh" 'VIND_ALLOW_K3D_FALLBACK="${VIND_ALLOW_K3D_FALLBACK:-0}"'
require_contains "scripts/vind-cluster.sh" 'kubectl taint nodes --all node-role.kubernetes.io/control-plane:NoSchedule-'
require_contains "scripts/vind-cluster.sh" 'kubectl taint nodes --all node-role.kubernetes.io/master:NoSchedule-'
require_contains "scripts/vind-cluster.sh" 'remain NoSchedule-tainted after normalization'
# shellcheck disable=SC2016
require_contains "scripts/vind-cluster.sh" 'vcluster create "${CLUSTER_NAME}" --driver docker --connect=false'
require_contains "scripts/vind-cluster.sh" 'ensure_arm64_probe_image()'
require_contains "scripts/vind-cluster.sh" 'docker pull --platform linux/arm64 alpine:latest'
require_not_contains "scripts/vind-cluster.sh" 'echo "[vind] vcluster docker driver failed on this host; falling back to k3d" >&2'
require_contains "scripts/vind-exec.sh" '--background-proxy=false'
require_contains "scripts/vind-exec.sh" 'VIND_EXEC_ACTIVE=1'
require_contains "scripts/vind-exec.sh" 'kubectl config use-context "k3d-${CLUSTER_NAME}"'
require_contains "scripts/k3d-e2e.sh" 'TRAILBASE_LOCAL_PORT="${TRAILBASE_LOCAL_PORT:-14000}"'
require_contains "scripts/k3d-e2e.sh" 'port-forward svc/trailbase "${TRAILBASE_LOCAL_PORT}:4000"'
require_contains "scripts/k3d-e2e.sh" 'http://127.0.0.1:${TRAILBASE_LOCAL_PORT}/api/auth/v1/login'
require_contains "scripts/k3d-e2e.sh" 'reserve_local_port()'
require_contains "scripts/k3d-e2e.sh" 'validate_certd_health()'
require_contains "scripts/k3d-e2e.sh" 'validate_trailbase_health()'
require_contains "scripts/k3d-e2e.sh" 'lsof -nP -iTCP:'
require_contains "scripts/k3d-e2e.sh" 'LOCAL_PORT="$(reserve_local_port "${LOCAL_PORT}" "certd")"'
require_contains "scripts/k3d-e2e.sh" 'TRAILBASE_LOCAL_PORT="$(reserve_local_port "${TRAILBASE_LOCAL_PORT}" "trailbase")"'

require_contains "README.md" '로컬 검증 엔트리포인트는 `vind`만 지원한다.'
require_contains "README.md" 'bash ./scripts/ci-chaos-local.sh'
require_contains "README.md" 'bash ./scripts/vind-cluster.sh ensure-context'
require_not_contains "README.md" './scripts/dev-empty-up.sh'
require_not_contains "README.md" 'docker-compose.empty.yml'
require_not_contains "README.md" 'docker-compose-chaos.sh'

expect_direct_invocation_failure "scripts/vind-bootstrap.sh" 'run via scripts/vind-exec.sh or pass KUBECONTEXT explicitly'
expect_direct_invocation_failure "scripts/vind-chaos.sh" 'run via scripts/vind-exec.sh or pass KUBE_CONTEXT explicitly'

if [[ -f ".github/workflows/chaos-vind.yml" ]]; then
  fail ".github/workflows/chaos-vind.yml should be removed once vind is the default workflow"
fi

for stale_file in \
  "scripts/kind-bootstrap.sh" \
  "scripts/kind-chaos.sh" \
  "scripts/dev-kind-up.sh" \
  "scripts/dev-kind-down.sh" \
  "scripts/dev-empty-up.sh" \
  "scripts/dev-empty-seed-user.sh" \
  "scripts/dev-empty-bootstrap-owner.sh" \
  "scripts/dev-empty-down.sh" \
  "scripts/docker-compose-chaos.sh" \
  "docker-compose.empty.yml" \
  "Dockerfile.dev"
do
  if [[ -f "${stale_file}" ]]; then
    fail "${stale_file} should be removed once vind is the only local chaos backend"
  fi
done

echo "vind default chaos wiring looks correct"
