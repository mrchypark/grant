#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
NAMESPACE="${NAMESPACE:-certd}"
LOCAL_PORT="${LOCAL_PORT:-18080}"
LEARNER_LOCAL_PORT="${LEARNER_LOCAL_PORT:-18081}"
TRAILBASE_USER_EMAIL="${TRAILBASE_USER_EMAIL:-trailbase-user@example.com}"
TRAILBASE_USER_PASSWORD="${TRAILBASE_USER_PASSWORD:-secret}"
BOOTSTRAP_ORGANIZATION_ID="${BOOTSTRAP_ORGANIZATION_ID:-conalog-account}"
PLANT_FACILITY_CODE="${PLANT_FACILITY_CODE:-TB-CHAOS-001}"
PLANT_NAME="${PLANT_NAME:-TrailBase Baseline Plant}"
PLANT_CONTRACT_NO="${PLANT_CONTRACT_NO:-TB-CHAOS-CONTRACT-001}"
PLANT_ADDRESS="${PLANT_ADDRESS:-Seoul}"
PLANT_INSTALLED_KW="${PLANT_INSTALLED_KW:-50.0}"
PLANT_REC_START_DATE="${PLANT_REC_START_DATE:-2026-01-01}"
PLANT_CONFIRM_ISSUED="${PLANT_CONFIRM_ISSUED:-Y}"
CERT_DIR="${CERT_DIR:-$ROOT_DIR/secrets/cert}"
CERT_PASSWORD="${CERT_PASSWORD:-}"
CERT_BUSINESS_NO="${CERT_BUSINESS_NO:-}"
CERTD_MASTER_KEY="${CERTD_MASTER_KEY:-local-dev-master-key}"
CERTD_TRAILBASE_CACHE_TTL_SECONDS="${CERTD_TRAILBASE_CACHE_TTL_SECONDS:-}"
CACHE_TTL_GRACE_SECONDS="${CACHE_TTL_GRACE_SECONDS:-5}"
POST_RESTART_STABILIZE_SECONDS="${POST_RESTART_STABILIZE_SECONDS:-10}"
VOTER_POD="${VOTER_POD:-certd-voter-0}"
FOLLOWER_VOTER_POD="${FOLLOWER_VOTER_POD:-}"
LEADER_VOTER_POD="${LEADER_VOTER_POD:-}"
LEARNER_DEPLOYMENT="${LEARNER_DEPLOYMENT:-certd-learner-1}"
CERTD_CHAOS_INCLUDE_MINIO="${CERTD_CHAOS_INCLUDE_MINIO:-0}"
CERTD_CHAOS_INCLUDE_LEARNER_PARTITION="${CERTD_CHAOS_INCLUDE_LEARNER_PARTITION:-0}"
CERTD_CHAOS_WITH_LOAD="${CERTD_CHAOS_WITH_LOAD:-0}"
CERTD_CHAOS_LOAD_LOG_FILE="${CERTD_CHAOS_LOAD_LOG_FILE:-}"
CERTD_CHAOS_LOAD_PHASE_FILE="${CERTD_CHAOS_LOAD_PHASE_FILE:-}"
LEARNER_REMOTE_API_PORT="${LEARNER_REMOTE_API_PORT:-8200}"
LEARNER_SERVICE_CURL_TIMEOUT_SECONDS="${LEARNER_SERVICE_CURL_TIMEOUT_SECONDS:-5}"
HTTP_CURL_TIMEOUT_SECONDS="${HTTP_CURL_TIMEOUT_SECONDS:-5}"

need() {
  command -v "$1" >/dev/null 2>&1 || {
    echo "missing command: $1" >&2
    exit 1
  }
}

need kubectl
need curl
need jq
need base64

retry_capture() {
  local attempts="$1"
  local delay="$2"
  shift 2

  local output=""
  for _ in $(seq 1 "${attempts}"); do
    if output="$("$@")"; then
      printf '%s' "${output}"
      return 0
    fi
    sleep "${delay}"
  done

  return 1
}

if [[ -z "${CERTD_TRAILBASE_CACHE_TTL_SECONDS}" ]]; then
  CERTD_TRAILBASE_CACHE_TTL_SECONDS="$(
    kubectl -n "${NAMESPACE}" get configmap certd-config \
      -o jsonpath='{.data.CERTD_TRAILBASE_CACHE_TTL_SECONDS}' 2>/dev/null || true
  )"
fi
CERTD_TRAILBASE_CACHE_TTL_SECONDS="${CERTD_TRAILBASE_CACHE_TTL_SECONDS:-45}"

pf_pid=""
port_forward_log="$(mktemp)"
learner_pf_pid=""
learner_port_forward_log="$(mktemp)"
chaos_load_pid=""
chaos_phase_file=""

cleanup() {
  if [[ -n "${chaos_load_pid}" ]]; then
    kill "${chaos_load_pid}" >/dev/null 2>&1 || true
  fi
  if [[ -n "${pf_pid}" ]]; then
    kill "${pf_pid}" >/dev/null 2>&1 || true
  fi
  if [[ -n "${learner_pf_pid}" ]]; then
    kill "${learner_pf_pid}" >/dev/null 2>&1 || true
  fi
  if [[ -n "${chaos_phase_file}" ]]; then
    rm -f "${chaos_phase_file}"
  fi
  rm -f "${port_forward_log}"
  rm -f "${learner_port_forward_log}"
}
trap cleanup EXIT

hiqlite_api_secret() {
  kubectl -n "${NAMESPACE}" get secret certd-secrets -o jsonpath='{.data.HQL_SECRET_API}' | base64 -d
}

wait_for_http() {
  local url="$1"
  local attempts="${2:-60}"
  local delay="${3:-2}"
  for _ in $(seq 1 "${attempts}"); do
    if curl -fsS --max-time "${HTTP_CURL_TIMEOUT_SECONDS}" "${url}" >/dev/null 2>&1; then
      return 0
    fi
    sleep "${delay}"
  done
  echo "timed out waiting for ${url}" >&2
  return 1
}

stop_port_forward() {
  local pid="$1"
  if [[ -n "${pid}" ]]; then
    kill "${pid}" >/dev/null 2>&1 || true
    wait "${pid}" 2>/dev/null || true
  fi
}

retry_json_query() {
  local attempts="$1"
  local delay="$2"
  shift 2

  local output=""
  for _ in $(seq 1 "${attempts}"); do
    if output="$("$@")" && [[ -n "${output}" ]]; then
      printf '%s' "${output}"
      return 0
    fi
    sleep "${delay}"
  done

  return 1
}

ensure_local_api() {
  if curl -fsS --max-time "${HTTP_CURL_TIMEOUT_SECONDS}" "http://127.0.0.1:${LOCAL_PORT}/healthz" >/dev/null 2>&1; then
    return 0
  fi
  if [[ -n "${pf_pid}" ]] && kill -0 "${pf_pid}" >/dev/null 2>&1; then
    return 0
  fi
  if ! curl -fsS --max-time "${HTTP_CURL_TIMEOUT_SECONDS}" "http://127.0.0.1:${LOCAL_PORT}/healthz" >/dev/null 2>&1; then
    start_port_forward
  fi
}

ensure_learner_api() {
  if curl -fsS --max-time "${HTTP_CURL_TIMEOUT_SECONDS}" "http://127.0.0.1:${LEARNER_LOCAL_PORT}/healthz" >/dev/null 2>&1; then
    return 0
  fi
  if [[ -n "${learner_pf_pid}" ]] && kill -0 "${learner_pf_pid}" >/dev/null 2>&1; then
    return 0
  fi
  if ! curl -fsS --max-time "${HTTP_CURL_TIMEOUT_SECONDS}" "http://127.0.0.1:${LEARNER_LOCAL_PORT}/healthz" >/dev/null 2>&1; then
    start_learner_port_forward
  fi
}

ensure_trailbase_online() {
  local replicas
  replicas="$(kubectl -n "${NAMESPACE}" get deployment trailbase -o jsonpath='{.spec.replicas}' 2>/dev/null || echo "")"
  if [[ "${replicas}" != "1" ]]; then
    kubectl -n "${NAMESPACE}" scale deployment/trailbase --replicas=1 >/dev/null
  fi
  kubectl -n "${NAMESPACE}" rollout status deployment/trailbase --timeout=15m >/dev/null
}

ensure_no_restore_mode() {
  local restore_value
  restore_value="$(
    kubectl -n "${NAMESPACE}" get statefulset certd-voter -o json \
      | jq -r '
          (.spec.template.spec.containers[] | select(.name == "certd") | .env // [])
          | map(select(.name == "HQL_BACKUP_RESTORE") | .value // "")
          | .[0] // empty
        '
  )"

  if [[ -n "${restore_value}" ]]; then
    cat >&2 <<EOF
chaos preflight failed: statefulset/certd-voter still has HQL_BACKUP_RESTORE=${restore_value}.
With emptyDir voters, deleting a voter pod in this state re-enters restore mode and pollutes the chaos result.
Rebuild a clean disposable baseline with:
  ${ROOT_DIR}/scripts/k3d-bootstrap.sh
or run the chained disposable harness:
  ${ROOT_DIR}/scripts/k3d-backup-restore-chaos.sh
EOF
    exit 1
  fi
}

set_chaos_phase() {
  local phase="$1"
  if [[ -n "${chaos_phase_file}" ]]; then
    printf '%s' "${phase}" >"${chaos_phase_file}"
  fi
}

start_chaos_load() {
  if [[ "${CERTD_CHAOS_WITH_LOAD}" != "1" ]]; then
    return 0
  fi

  if [[ -n "${chaos_load_pid}" ]]; then
    return 0
  fi

  chaos_phase_file="${CERTD_CHAOS_LOAD_PHASE_FILE:-$(mktemp "${TMPDIR:-/tmp}/k3d-chaos-phase.XXXXXX")}"
  set_chaos_phase "pre-disruption"

  WORKLOAD_LOG_FILE="${CERTD_CHAOS_LOAD_LOG_FILE}" \
  WORKLOAD_PHASE_FILE="${chaos_phase_file}" \
  WORKLOAD_VOTER_LOCAL_PORT="${LOCAL_PORT}" \
  WORKLOAD_LEARNER_LOCAL_PORT="${LEARNER_LOCAL_PORT}" \
  WORKLOAD_MANAGE_PORT_FORWARD=0 \
  CERTD_MASTER_KEY="${CERTD_MASTER_KEY}" \
  TRAILBASE_USER_EMAIL="${TRAILBASE_USER_EMAIL}" \
  TRAILBASE_USER_PASSWORD="${TRAILBASE_USER_PASSWORD}" \
  BOOTSTRAP_ORGANIZATION_ID="${BOOTSTRAP_ORGANIZATION_ID}" \
  NAMESPACE="${NAMESPACE}" \
    bash "${ROOT_DIR}/scripts/k3d-chaos-load.sh" &
  chaos_load_pid=$!
}

start_port_forward() {
  if [[ -n "${pf_pid}" ]]; then
    stop_port_forward "${pf_pid}"
    pf_pid=""
  fi
  : >"${port_forward_log}"
  kubectl -n "${NAMESPACE}" port-forward svc/certd "${LOCAL_PORT}:80" >"${port_forward_log}" 2>&1 &
  pf_pid=$!
  if ! wait_for_http "http://127.0.0.1:${LOCAL_PORT}/healthz"; then
    echo "[chaos] voter port-forward failed" >&2
    tail -n 50 "${port_forward_log}" >&2 || true
    return 1
  fi
}

start_learner_port_forward() {
  if [[ -n "${learner_pf_pid}" ]]; then
    stop_port_forward "${learner_pf_pid}"
    learner_pf_pid=""
  fi
  : >"${learner_port_forward_log}"
  kubectl -n "${NAMESPACE}" port-forward "svc/${LEARNER_DEPLOYMENT}" "${LEARNER_LOCAL_PORT}:8080" >"${learner_port_forward_log}" 2>&1 &
  learner_pf_pid=$!
  if ! wait_for_http "http://127.0.0.1:${LEARNER_LOCAL_PORT}/healthz"; then
    echo "[chaos] learner port-forward failed" >&2
    tail -n 50 "${learner_port_forward_log}" >&2 || true
    return 1
  fi
}

fetch_voter_metrics() {
  local pod_name="$1"
  local api_secret="$2"
  local metrics_port="${METRICS_LOCAL_PORT:-18200}"
  local metrics_log
  local metrics_pf_pid=""
  local ready=0
  local output=""
  local status=0
  metrics_log="$(mktemp)"

  cleanup_metrics_port_forward() {
    if [[ -n "${metrics_pf_pid}" ]]; then
      kill "${metrics_pf_pid}" >/dev/null 2>&1 || true
      wait "${metrics_pf_pid}" 2>/dev/null || true
    fi
    rm -f "${metrics_log}"
  }

  kubectl -n "${NAMESPACE}" port-forward "pod/${pod_name}" "${metrics_port}:8200" >"${metrics_log}" 2>&1 &
  metrics_pf_pid=$!

  for _ in $(seq 1 30); do
    if output="$(
      curl -fsS --max-time "${HTTP_CURL_TIMEOUT_SECONDS}" \
        -H "X-API-SECRET: ${api_secret}" \
        -H 'accept: application/json' \
        "http://127.0.0.1:${metrics_port}/cluster/metrics/sqlite" 2>/dev/null
    )"; then
      ready=1
      break
    fi
    sleep 1
  done

  if [[ "${ready}" -eq 1 ]]; then
    status=0
  else
    status=1
  fi
  cleanup_metrics_port_forward
  if [[ "${status}" -eq 0 ]]; then
    printf '%s' "${output}"
  fi
  return "${status}"
}

resolve_voter_role_pods() {
  local api_secret pods pod metrics_json leader_addr
  api_secret="$(hiqlite_api_secret)"
  pods="$(kubectl -n "${NAMESPACE}" get pods -l app=certd,role=voter -o jsonpath='{.items[*].metadata.name}')"

  RESOLVED_LEADER_VOTER_POD=""
  RESOLVED_FOLLOWER_VOTER_POD=""

  for pod in ${pods}; do
    metrics_json="$(retry_json_query 10 1 fetch_voter_metrics "${pod}" "${api_secret}" || true)"
    if [[ -z "${metrics_json}" ]]; then
      continue
    fi
    leader_addr="$(
      jq -r '.membership_config.membership.nodes[(.current_leader | tostring)].addr_api // empty' \
        <<<"${metrics_json}"
    )"
    if [[ -n "${leader_addr}" ]]; then
      RESOLVED_LEADER_VOTER_POD="${leader_addr%%.*}"
      break
    fi
  done

  if [[ -z "${RESOLVED_LEADER_VOTER_POD}" ]]; then
    echo "failed to resolve current leader voter pod" >&2
    exit 1
  fi

  for pod in ${pods}; do
    if [[ "${pod}" != "${RESOLVED_LEADER_VOTER_POD}" ]]; then
      RESOLVED_FOLLOWER_VOTER_POD="${pod}"
      break
    fi
  done

  if [[ -z "${RESOLVED_FOLLOWER_VOTER_POD}" ]]; then
    echo "failed to resolve follower voter pod from voter set: ${pods}" >&2
    exit 1
  fi
}

delete_voter_pod_and_verify() {
  local phase="$1"
  local pod_name="$2"
  local auth_token="$3"
  local active_org="$4"
  local facility_code="$5"

  set_chaos_phase "${phase}"
  echo "[chaos] ${phase}: ${pod_name}"
  kubectl -n "${NAMESPACE}" delete pod "${pod_name}" --wait=false
  kubectl -n "${NAMESPACE}" rollout status statefulset/certd-voter --timeout=10m
  sleep "${POST_RESTART_STABILIZE_SECONDS}"
  retry_capture 20 2 observe_core_api "${phase}" "${auth_token}" "${active_org}"
  retry_capture 20 2 inspect_plant "${auth_token}" "${active_org}" "${facility_code}" | jq
  set_chaos_phase "post-${phase}"
}

curl_certd() {
  local output
  local status=0

  set +e
  output="$(curl -fsS "$@" 2>/tmp/k3d-chaos.curl.err)"
  status=$?
  set -e
  if [[ "${status}" -eq 0 ]]; then
    printf '%s' "${output}"
    return 0
  fi

  start_port_forward
  set +e
  output="$(curl -fsS "$@" 2>/tmp/k3d-chaos.curl.err)"
  status=$?
  set -e
  if [[ "${status}" -eq 0 ]]; then
    printf '%s' "${output}"
    return 0
  fi

  return "${status}"
}

seed_trailbase_user() {
  TRAILBASE_USER_EMAIL="${TRAILBASE_USER_EMAIL}" \
  TRAILBASE_USER_PASSWORD="${TRAILBASE_USER_PASSWORD}" \
  NAMESPACE="${NAMESPACE}" \
    bash "${ROOT_DIR}/scripts/k3d-seed-trailbase-user.sh"
}

login_trailbase() {
  ensure_local_api
  curl_certd -X POST \
    "http://127.0.0.1:${LOCAL_PORT}/api/auth/v1/login" \
    -H 'content-type: application/json' \
    --data-binary "$(jq -n \
      --arg email "${TRAILBASE_USER_EMAIL}" \
      --arg password "${TRAILBASE_USER_PASSWORD}" \
      '{email: $email, password: $password}')" \
    | jq -r '.auth_token // .authToken // empty'
}

bootstrap_owner() {
  local auth_token="$1"
  ensure_local_api
  curl_certd -X POST \
    "http://127.0.0.1:${LOCAL_PORT}/api/v1/admin/bootstrap/trailbase-owner" \
    -H "authorization: Bearer ${auth_token}" \
    -H "x-certd-master-key: ${CERTD_MASTER_KEY}"
}

current_user() {
  local auth_token="$1"
  local active_org="$2"
  ensure_local_api
  curl_certd \
    "http://127.0.0.1:${LOCAL_PORT}/api/v1/me" \
    -H "authorization: Bearer ${auth_token}" \
    -H "x-organization-id: ${active_org}"
}

list_plants() {
  local auth_token="$1"
  local active_org="$2"
  ensure_local_api
  curl_certd \
    "http://127.0.0.1:${LOCAL_PORT}/api/v1/plants" \
    -H "authorization: Bearer ${auth_token}" \
    -H "x-organization-id: ${active_org}"
}

create_plant() {
  local auth_token="$1"
  local active_org="$2"
  ensure_local_api
  curl_certd -X POST \
    "http://127.0.0.1:${LOCAL_PORT}/api/v1/organizations/${active_org}/plants" \
    -H 'content-type: application/json' \
    -H "authorization: Bearer ${auth_token}" \
    -H "x-organization-id: ${active_org}" \
    --data-binary "$(jq -n \
      --arg facilityCode "${PLANT_FACILITY_CODE}" \
      --arg name "${PLANT_NAME}" \
      --arg contractNo "${PLANT_CONTRACT_NO}" \
      --arg address "${PLANT_ADDRESS}" \
      --arg recStartDate "${PLANT_REC_START_DATE}" \
      --arg confirmIssued "${PLANT_CONFIRM_ISSUED}" \
      --arg ownerOrganizationId "${active_org}" \
      --argjson installedKw "${PLANT_INSTALLED_KW}" \
      '{
        facilityCode: $facilityCode,
        name: $name,
        contractNo: $contractNo,
        address: $address,
        installedKw: $installedKw,
        recStartDate: $recStartDate,
        confirmIssued: $confirmIssued,
        ownerOrganizationId: $ownerOrganizationId
      }')"
}

inspect_plant() {
  local auth_token="$1"
  local active_org="$2"
  local facility_code="$3"
  ensure_local_api
  curl_certd \
    "http://127.0.0.1:${LOCAL_PORT}/api/v1/plants/${facility_code}" \
    -H "authorization: Bearer ${auth_token}" \
    -H "x-organization-id: ${active_org}"
}

sync_plant() {
  local auth_token="$1"
  local active_org="$2"
  local facility_code="$3"
  ensure_local_api
  curl_certd -X POST \
    "http://127.0.0.1:${LOCAL_PORT}/api/v1/organizations/${active_org}/sync" \
    -H 'content-type: application/json' \
    -H "authorization: Bearer ${auth_token}" \
    -H "x-organization-id: ${active_org}" \
    --data-binary "$(jq -n \
      --arg facilityCode "${facility_code}" \
      '{facilityCode: $facilityCode, mode: "full"}')"
}

maybe_upload_certificate_and_sync() {
  local auth_token="$1"
  local active_org="$2"
  local facility_code="$3"
  local sign_cert="${CERT_DIR}/signCert.der"
  local sign_pri="${CERT_DIR}/signPri.key"
  local km_cert="${CERT_DIR}/kmCert.der"
  local km_pri="${CERT_DIR}/kmPri.key"

  if [[ ! -f "${sign_cert}" || ! -f "${sign_pri}" || ! -f "${km_cert}" || ! -f "${km_pri}" ]]; then
    echo "certificate bundle not present; skipping cert upload and sync"
    return 0
  fi

  if [[ -z "${CERT_PASSWORD}" ]]; then
    echo "CERT_PASSWORD not set; skipping cert upload and sync"
    return 0
  fi

  local payload
  payload="$(jq -n \
    --arg businessNo "${CERT_BUSINESS_NO}" \
    --arg password "${CERT_PASSWORD}" \
    --arg signCertDerBase64 "$(base64 < "${sign_cert}" | tr -d '\n')" \
    --arg signPriKeyBase64 "$(base64 < "${sign_pri}" | tr -d '\n')" \
    --arg kmCertDerBase64 "$(base64 < "${km_cert}" | tr -d '\n')" \
    --arg kmPriKeyBase64 "$(base64 < "${km_pri}" | tr -d '\n')" \
    '{
      businessNo: ($businessNo | select(length > 0)),
      password: $password,
      signCertDerBase64: $signCertDerBase64,
      signPriKeyBase64: $signPriKeyBase64,
      kmCertDerBase64: $kmCertDerBase64,
      kmPriKeyBase64: $kmPriKeyBase64
    }')"

  echo "[write] uploading cert bundle for organization ${active_org}"
  ensure_local_api
  curl_certd -X PUT \
    "http://127.0.0.1:${LOCAL_PORT}/api/v1/organizations/${active_org}/certificate" \
    -H 'content-type: application/json' \
    -H "authorization: Bearer ${auth_token}" \
    -H "x-organization-id: ${active_org}" \
    --data-binary "${payload}" | jq

  echo "[write] syncing facility ${facility_code}"
  sync_plant "${auth_token}" "${active_org}" "${facility_code}" | jq
}

trigger_backup() {
  ensure_local_api
  echo "[backup] trigger"
  curl_certd -X POST "http://127.0.0.1:${LOCAL_PORT}/api/v1/admin/backups" \
    -H "x-certd-master-key: ${CERTD_MASTER_KEY}" | jq
  echo "[backup] local"
  curl_certd "http://127.0.0.1:${LOCAL_PORT}/api/v1/admin/backups" \
    -H "x-certd-master-key: ${CERTD_MASTER_KEY}" | jq
  echo "[backup] s3"
  curl_certd "http://127.0.0.1:${LOCAL_PORT}/api/v1/admin/backups/s3" \
    -H "x-certd-master-key: ${CERTD_MASTER_KEY}" | jq
}

resolve_baseline_plant() {
  local auth_token="$1"
  local active_org="$2"
  local plants_json baseline_json
  plants_json="$(list_plants "${auth_token}" "${active_org}")"
  baseline_json="$(jq -c --arg code "${PLANT_FACILITY_CODE}" '
    [ .[] | select((.facilityCode // .facility_code) == $code) ] | .[0] // empty
  ' <<<"${plants_json}")"
  if [[ -n "${baseline_json}" && "${baseline_json}" != "null" ]]; then
    echo "${baseline_json}"
    return 0
  fi

  echo "[baseline] creating plant ${PLANT_FACILITY_CODE}" >&2
  create_plant "${auth_token}" "${active_org}"
}

expect_failure() {
  local description="$1"
  shift
  local status=0
  "$@" >/tmp/k3d-chaos.expect-failure.out 2>/tmp/k3d-chaos.expect-failure.err || status=$?
  if [[ "${status}" -eq 0 ]]; then
    echo "expected failure: ${description}" >&2
    echo "stdout:" >&2
    sed 's/^/  /' /tmp/k3d-chaos.expect-failure.out >&2 || true
    echo "stderr:" >&2
    sed 's/^/  /' /tmp/k3d-chaos.expect-failure.err >&2 || true
    exit 1
  fi
  echo "[expected-failure] ${description}" >&2
}

observe_core_api() {
  local label="$1"
  local auth_token="$2"
  local active_org="$3"
  local me_json plants_json observed_org visible_count

  me_json="$(current_user "${auth_token}" "${active_org}")"
  observed_org="$(jq -r '.activeOrganizationId // .active_organization_id // empty' <<<"${me_json}")"
  if [[ -z "${observed_org}" ]]; then
    observed_org="${active_org}"
  fi
  if [[ "${observed_org}" != "${active_org}" ]]; then
    echo "unexpected active organization for ${label}: expected ${active_org}, got ${observed_org}" >&2
    exit 1
  fi

  plants_json="$(list_plants "${auth_token}" "${active_org}")"
  visible_count="$(jq 'length' <<<"${plants_json}")"

  echo "[${label}] /api/v1/me"
  echo "${me_json}" | jq
  echo "[${label}] /api/v1/plants count=${visible_count}"
  echo "${plants_json}" | jq
}

wait_for_no_pods_with_label() {
  local label_selector="$1"
  local attempts="${2:-60}"
  local delay="${3:-2}"
  for _ in $(seq 1 "${attempts}"); do
    local names
    names="$(kubectl -n "${NAMESPACE}" get pods -l "${label_selector}" -o jsonpath='{.items[*].metadata.name}' 2>/dev/null || true)"
    if [[ -z "${names}" ]]; then
      return 0
    fi
    sleep "${delay}"
  done
  echo "timed out waiting for pods with selector ${label_selector} to disappear" >&2
  exit 1
}

wait_for_expected_failure() {
  local description="$1"
  local attempts="${2:-60}"
  local delay="${3:-2}"
  shift 3

  local status=0
  for _ in $(seq 1 "${attempts}"); do
    status=0
    "$@" >/tmp/k3d-chaos.wait-failure.out 2>/tmp/k3d-chaos.wait-failure.err || status=$?
    if [[ "${status}" -ne 0 ]]; then
      echo "[expected-failure] ${description}" >&2
      return 0
    fi
    sleep "${delay}"
  done

  echo "timed out waiting for expected failure: ${description}" >&2
  echo "stdout:" >&2
  sed 's/^/  /' /tmp/k3d-chaos.wait-failure.out >&2 || true
  echo "stderr:" >&2
  sed 's/^/  /' /tmp/k3d-chaos.wait-failure.err >&2 || true
  return 1
}

learner_pod_name() {
  kubectl -n "${NAMESPACE}" get pods \
    -l "app=certd,role=learner,instance=${LEARNER_DEPLOYMENT#certd-}" \
    -o jsonpath='{.items[?(@.status.phase=="Running")].metadata.name}' \
    | awk '{print $1}'
}

learner_pod_ip() {
  local pod_name="$1"
  kubectl -n "${NAMESPACE}" get pod "${pod_name}" -o jsonpath='{.status.podIP}'
}

learner_node_name() {
  local pod_name="$1"
  kubectl -n "${NAMESPACE}" get pod "${pod_name}" -o jsonpath='{.spec.nodeName}'
}

docker_node_name_for_k8s_node() {
  local k8s_node_name="$1"
  printf '%s' "${k8s_node_name}"
}

voter_pod_ips() {
  kubectl -n "${NAMESPACE}" get pods -l app=certd,role=voter -o jsonpath='{range .items[*]}{.status.podIP}{"\n"}{end}'
}

insert_partition_rule() {
  local docker_node_name="$1"
  local source_ip="$2"
  local destination_ip="$3"
  docker exec "${docker_node_name}" iptables -I FORWARD 1 \
    -s "${source_ip}" -d "${destination_ip}" -p tcp --dport "${LEARNER_REMOTE_API_PORT}" -j DROP
}

delete_partition_rule() {
  local docker_node_name="$1"
  local source_ip="$2"
  local destination_ip="$3"
  docker exec "${docker_node_name}" iptables -D FORWARD \
    -s "${source_ip}" -d "${destination_ip}" -p tcp --dport "${LEARNER_REMOTE_API_PORT}" -j DROP
}

drop_partition_connections() {
  local docker_node_name="$1"
  local source_ip="$2"
  local destination_ip="$3"

  docker exec "${docker_node_name}" conntrack -D \
    -s "${source_ip}" -d "${destination_ip}" -p tcp --dport "${LEARNER_REMOTE_API_PORT}" \
    >/dev/null 2>&1 || true
  docker exec "${docker_node_name}" conntrack -D \
    -s "${destination_ip}" -d "${source_ip}" -p tcp --sport "${LEARNER_REMOTE_API_PORT}" \
    >/dev/null 2>&1 || true
}

learner_current_user_via_service() {
  local auth_token="$1"
  local active_org="$2"
  curl_certd \
    "http://127.0.0.1:${LOCAL_PORT}/api/v1/me?consistency=strong" \
    -H "authorization: Bearer ${auth_token}" \
    -H "x-organization-id: ${active_org}"
}

learner_service_probe() {
  local path="$1"
  local auth_token="$2"
  local active_org="$3"
  ensure_learner_api
  curl -fsS --max-time "${LEARNER_SERVICE_CURL_TIMEOUT_SECONDS}" \
    "http://127.0.0.1:${LEARNER_LOCAL_PORT}${path}" \
    -H "authorization: Bearer ${auth_token}" \
    -H "x-organization-id: ${active_org}"
}

scenario_learner_voter_partition() {
  local auth_token="$1"
  local active_org="$2"
  local facility_code="$3"
  local learner_pod learner_ip learner_node docker_node
  local cleanup_needed=0
  local failure=0

  learner_pod="$(learner_pod_name)"
  if [[ -z "${learner_pod}" ]]; then
    echo "failed to resolve running learner pod for ${LEARNER_DEPLOYMENT}" >&2
    exit 1
  fi
  learner_ip="$(learner_pod_ip "${learner_pod}")"
  learner_node="$(learner_node_name "${learner_pod}")"
  docker_node="$(docker_node_name_for_k8s_node "${learner_node}")"

  set_chaos_phase "learner-voter-partition"
  echo "[chaos] learner-voter partition via node iptables: pod=${learner_pod} ip=${learner_ip} node=${docker_node}"

  while read -r voter_ip; do
    [[ -n "${voter_ip}" ]] || continue
    insert_partition_rule "${docker_node}" "${learner_ip}" "${voter_ip}"
    drop_partition_connections "${docker_node}" "${learner_ip}" "${voter_ip}"
    cleanup_needed=1
  done < <(voter_pod_ips)

  set +e
  if ! retry_capture 10 1 learner_service_probe "/api/v1/me" "${auth_token}" "${active_org}" >/dev/null; then
    failure=1
  fi
  if ! retry_capture 10 1 learner_service_probe "/api/v1/plants" "${auth_token}" "${active_org}" >/dev/null; then
    failure=1
  fi
  if ! retry_capture 10 1 learner_service_probe "/api/v1/plants/${facility_code}" "${auth_token}" "${active_org}" >/dev/null; then
    failure=1
  fi

  if ! wait_for_expected_failure \
    "learner current-user strong read while quorum is unreachable" 10 1 \
    learner_service_probe "/api/v1/me?consistency=strong" "${auth_token}" "${active_org}"; then
    failure=1
  fi
  if ! wait_for_expected_failure \
    "learner plant list strong read while quorum is unreachable" 10 1 \
    learner_service_probe "/api/v1/plants?consistency=strong" "${auth_token}" "${active_org}"; then
    failure=1
  fi
  if ! wait_for_expected_failure \
    "learner plant inspect strong read while quorum is unreachable" 10 1 \
    learner_service_probe "/api/v1/plants/${facility_code}?consistency=strong" "${auth_token}" "${active_org}"; then
    failure=1
  fi
  set -e

  if [[ "${cleanup_needed}" -eq 1 ]]; then
    while read -r voter_ip; do
      [[ -n "${voter_ip}" ]] || continue
      delete_partition_rule "${docker_node}" "${learner_ip}" "${voter_ip}" || true
    done < <(voter_pod_ips)
  fi

  set_chaos_phase "learner-voter-partition-recovery"
  retry_capture 20 1 learner_service_probe "/api/v1/me?consistency=strong" "${auth_token}" "${active_org}" | jq
  retry_capture 20 1 learner_service_probe "/api/v1/plants?consistency=strong" "${auth_token}" "${active_org}" | jq
  retry_capture 20 1 learner_service_probe "/api/v1/plants/${facility_code}?consistency=strong" "${auth_token}" "${active_org}" | jq
  set_chaos_phase "post-learner-voter-partition"

  if [[ "${failure}" -ne 0 ]]; then
    echo "learner-voter-partition checks failed" >&2
    exit 1
  fi
}

scenario_follower_delete() {
  local auth_token="$1"
  local active_org="$2"
  local facility_code="$3"
  local follower_pod="${FOLLOWER_VOTER_POD:-}"

  if [[ -z "${follower_pod}" ]]; then
    resolve_voter_role_pods
    follower_pod="${RESOLVED_FOLLOWER_VOTER_POD}"
  fi

  delete_voter_pod_and_verify "follower-delete" "${follower_pod}" "${auth_token}" "${active_org}" "${facility_code}"
}

scenario_leader_delete() {
  local auth_token="$1"
  local active_org="$2"
  local facility_code="$3"
  local leader_pod="${LEADER_VOTER_POD:-}"

  if [[ -z "${leader_pod}" ]]; then
    resolve_voter_role_pods
    leader_pod="${RESOLVED_LEADER_VOTER_POD}"
  fi

  delete_voter_pod_and_verify "leader-delete" "${leader_pod}" "${auth_token}" "${active_org}" "${facility_code}"
}

scenario_learner_restart() {
  local auth_token="$1"
  local active_org="$2"
  local facility_code="$3"
  local learner_pod

  set_chaos_phase "learner-restart"
  echo "[chaos] learner pod delete for deployment/${LEARNER_DEPLOYMENT}"
  learner_pod="$(kubectl -n "${NAMESPACE}" get pods -l "app=certd,role=learner,instance=${LEARNER_DEPLOYMENT#certd-}" -o jsonpath='{.items[0].metadata.name}')"
  kubectl -n "${NAMESPACE}" delete pod "${learner_pod}" --wait=false
  kubectl -n "${NAMESPACE}" rollout status "deployment/${LEARNER_DEPLOYMENT}" --timeout=10m
  sleep "${POST_RESTART_STABILIZE_SECONDS}"
  retry_capture 20 2 observe_core_api "learner-restart" "${auth_token}" "${active_org}"
  retry_capture 20 2 inspect_plant "${auth_token}" "${active_org}" "${facility_code}" | jq
  set_chaos_phase "post-learner-restart"
}

scenario_trailbase_cache_ttl() {
  local auth_token="$1"
  local active_org="$2"
  local facility_code="$3"

  set_chaos_phase "trailbase-down"
  echo "[chaos] trailbase scale down + auth cache ttl" >&2
  kubectl -n "${NAMESPACE}" scale deployment/trailbase --replicas=0 >/dev/null
  wait_for_expected_failure "new login while trailbase is offline" 120 2 login_trailbase
  observe_core_api "trailbase-down-cache-alive" "${auth_token}" "${active_org}" >&2

  local ttl_wait
  ttl_wait="$((CERTD_TRAILBASE_CACHE_TTL_SECONDS + CACHE_TTL_GRACE_SECONDS))"
  echo "[trailbase] waiting ${ttl_wait}s for auth cache expiry" >&2
  sleep "${ttl_wait}"

  expect_failure "cached bearer token after TTL" current_user "${auth_token}" "${active_org}"
  expect_failure "plant list after TTL" list_plants "${auth_token}" "${active_org}"

  set_chaos_phase "trailbase-recovery"
  echo "[trailbase] restoring deployment" >&2
  kubectl -n "${NAMESPACE}" scale deployment/trailbase --replicas=1 >/dev/null
  kubectl -n "${NAMESPACE}" rollout status deployment/trailbase --timeout=15m >/dev/null
  local recovered_token
  recovered_token="$(login_trailbase || true)"
  if [[ -z "${recovered_token}" || "${recovered_token}" == "null" ]]; then
    echo "[trailbase] restored user login failed; attempting reseed fallback" >&2
    seed_trailbase_user >&2
    recovered_token="$(login_trailbase)"
    if [[ -z "${recovered_token}" || "${recovered_token}" == "null" ]]; then
      echo "failed to login after trailbase recovery" >&2
      exit 1
    fi
  fi
  observe_core_api "trailbase-recovered" "${recovered_token}" "${active_org}" >&2
  inspect_plant "${recovered_token}" "${active_org}" "${facility_code}" | jq >&2
  set_chaos_phase "post-trailbase-recovery"

  printf '%s\n' "${recovered_token}"
}

scenario_minio_restart() {
  local auth_token="$1"
  local active_org="$2"
  local facility_code="$3"
  local minio_pod

  echo "[chaos] minio restart"
  minio_pod="$(kubectl -n "${NAMESPACE}" get pod -l app=minio -o jsonpath='{.items[0].metadata.name}')"
  kubectl -n "${NAMESPACE}" delete pod "${minio_pod}" --wait=false
  kubectl -n "${NAMESPACE}" rollout status statefulset/minio --timeout=15m
  sleep "${POST_RESTART_STABILIZE_SECONDS}"
  start_port_forward
  observe_core_api "minio-restart" "${auth_token}" "${active_org}"
  inspect_plant "${auth_token}" "${active_org}" "${facility_code}" | jq
  trigger_backup
}

main() {
  local auth_token me_json active_org baseline_plant baseline_facility_code

  ensure_no_restore_mode
  ensure_trailbase_online
  start_port_forward
  seed_trailbase_user

  auth_token="$(retry_capture 20 2 login_trailbase)"
  if [[ -z "${auth_token}" || "${auth_token}" == "null" ]]; then
    echo "failed to obtain TrailBase auth token" >&2
    exit 1
  fi

  local bootstrap_json
  bootstrap_json="$(retry_capture 20 2 bootstrap_owner "${auth_token}")"
  local bootstrap_active_org
  bootstrap_active_org="$(jq -r '.organizationId // .organization_id // empty' <<<"${bootstrap_json}")"
  if [[ -z "${bootstrap_active_org}" ]]; then
    bootstrap_active_org="${BOOTSTRAP_ORGANIZATION_ID}"
  fi

  me_json="$(retry_capture 20 2 current_user "${auth_token}" "${bootstrap_active_org}")"
  active_org="$(jq -r '.activeOrganizationId // .active_organization_id // empty' <<<"${me_json}")"
  if [[ -z "${active_org}" ]]; then
    active_org="${bootstrap_active_org}"
  fi

  echo "[baseline] /api/v1/me"
  echo "${me_json}" | jq

  baseline_plant="$(retry_capture 20 2 resolve_baseline_plant "${auth_token}" "${active_org}")"
  baseline_facility_code="$(jq -r '.facilityCode // .facility_code // empty' <<<"${baseline_plant}")"
  if [[ -z "${baseline_facility_code}" || "${baseline_facility_code}" == "null" ]]; then
    echo "failed to resolve baseline facility code" >&2
    exit 1
  fi

  echo "[baseline] plant"
  echo "${baseline_plant}" | jq
  retry_capture 20 2 inspect_plant "${auth_token}" "${active_org}" "${baseline_facility_code}" | jq
  retry_capture 20 2 trigger_backup
  maybe_upload_certificate_and_sync "${auth_token}" "${active_org}" "${baseline_facility_code}"
  start_chaos_load

  scenario_follower_delete "${auth_token}" "${active_org}" "${baseline_facility_code}"
  scenario_leader_delete "${auth_token}" "${active_org}" "${baseline_facility_code}"
  if [[ "${CERTD_CHAOS_INCLUDE_LEARNER_PARTITION}" == "1" ]]; then
    scenario_learner_voter_partition "${auth_token}" "${active_org}" "${baseline_facility_code}"
  else
    echo "[skip] learner-voter partition scenario disabled; set CERTD_CHAOS_INCLUDE_LEARNER_PARTITION=1 to include it"
  fi
  scenario_learner_restart "${auth_token}" "${active_org}" "${baseline_facility_code}"
  auth_token="$(scenario_trailbase_cache_ttl "${auth_token}" "${active_org}" "${baseline_facility_code}")"
  if [[ "${CERTD_CHAOS_INCLUDE_MINIO}" == "1" ]]; then
    scenario_minio_restart "${auth_token}" "${active_org}" "${baseline_facility_code}"
  else
    echo "[skip] minio restart scenario disabled; set CERTD_CHAOS_INCLUDE_MINIO=1 to include it"
  fi

  echo "chaos test completed"
}

main "$@"
