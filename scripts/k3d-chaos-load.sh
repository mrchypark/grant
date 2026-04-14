#!/usr/bin/env bash
set -euo pipefail

NAMESPACE="${NAMESPACE:-certd}"
TRAILBASE_USER_EMAIL="${TRAILBASE_USER_EMAIL:-trailbase-user@example.com}"
TRAILBASE_USER_PASSWORD="${TRAILBASE_USER_PASSWORD:-secret}"
BOOTSTRAP_ORGANIZATION_ID="${BOOTSTRAP_ORGANIZATION_ID:-conalog-account}"
CERTD_MASTER_KEY="${CERTD_MASTER_KEY:-local-dev-master-key}"
WORKLOAD_READ_FACILITY_CODE="${WORKLOAD_READ_FACILITY_CODE:-TB-CHAOS-001}"
WORKLOAD_READ_WRITE_ITERATIONS="${WORKLOAD_READ_WRITE_ITERATIONS:-10}"
WORKLOAD_CYCLE_INTERVAL_SECONDS="${WORKLOAD_CYCLE_INTERVAL_SECONDS:-2}"
WORKLOAD_MAX_CYCLES="${WORKLOAD_MAX_CYCLES:-0}"
WORKLOAD_RUN_ID="${WORKLOAD_RUN_ID:-$(date +%s)}"
WORKLOAD_VOTER_LOCAL_PORT="${WORKLOAD_VOTER_LOCAL_PORT:-18090}"
WORKLOAD_LEARNER_LOCAL_PORT="${WORKLOAD_LEARNER_LOCAL_PORT:-18091}"
WORKLOAD_TRAILBASE_LOCAL_PORT="${WORKLOAD_TRAILBASE_LOCAL_PORT:-14090}"
WORKLOAD_VOTER_SERVICE="${WORKLOAD_VOTER_SERVICE:-certd}"
WORKLOAD_LEARNER_SERVICE="${WORKLOAD_LEARNER_SERVICE:-certd-learner-1}"
WORKLOAD_MANAGE_PORT_FORWARD="${WORKLOAD_MANAGE_PORT_FORWARD:-1}"
WORKLOAD_LOG_FILE="${WORKLOAD_LOG_FILE:-$(mktemp "${TMPDIR:-/tmp}/k3d-chaos-load.XXXXXX")}"
WORKLOAD_PHASE_FILE="${WORKLOAD_PHASE_FILE:-}"
HTTP_CURL_TIMEOUT_SECONDS="${HTTP_CURL_TIMEOUT_SECONDS:-5}"
HTTP_CONNECT_TIMEOUT_SECONDS="${HTTP_CONNECT_TIMEOUT_SECONDS:-2}"
SANITIZED_RUN_ID="${WORKLOAD_RUN_ID//[^[:alnum:]]/}"

declare -a RESERVED_LOCAL_PORTS=()

need() {
  command -v "$1" >/dev/null 2>&1 || {
    echo "missing command: $1" >&2
    exit 1
  }
}

need kubectl
need curl
need jq

port_is_reserved() {
  local port="$1"
  local reserved_port
  for reserved_port in "${RESERVED_LOCAL_PORTS[@]:-}"; do
    if [[ "${reserved_port}" == "${port}" ]]; then
      return 0
    fi
  done
  return 1
}

port_is_available() {
  local port="$1"
  if port_is_reserved "${port}"; then
    return 1
  fi
  if command -v lsof >/dev/null 2>&1; then
    if lsof -nP -iTCP:"${port}" -sTCP:LISTEN >/dev/null 2>&1; then
      return 1
    fi
    return 0
  fi
  python3 - "$port" <<'PY'
import socket
import sys

port = int(sys.argv[1])
with socket.socket() as sock:
    sock.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
    try:
        sock.bind(("127.0.0.1", port))
    except OSError:
        raise SystemExit(1)
raise SystemExit(0)
PY
}

reserve_local_port() {
  local requested_port="$1"
  local label="$2"
  local candidate_port=""
  local offset
  for offset in $(seq 0 49); do
    candidate_port="$((requested_port + offset))"
    if port_is_available "${candidate_port}"; then
      RESERVED_LOCAL_PORTS+=("${candidate_port}")
      if [[ "${candidate_port}" != "${requested_port}" ]]; then
        echo "[ports] ${label} local port ${requested_port} busy; using ${candidate_port}" >&2
      fi
      printf '%s' "${candidate_port}"
      return 0
    fi
  done
  echo "failed to reserve local port for ${label} near ${requested_port}" >&2
  exit 1
}

validate_certd_health() {
  local body="$1"
  jq -e '
    .status == "ok" and
    (.readReady | type == "boolean") and
    (.writeReady | type == "boolean") and
    (.runtimeMode | type == "string") and
    (.defaultReadConsistency | type == "string")
  ' >/dev/null <<<"${body}"
}

validate_trailbase_health() {
  local body="${1//$'\r'/}"
  [[ "$(printf '%s' "${body}" | tr '[:upper:]' '[:lower:]')" == "ok" ]]
}

voter_pf_pid=""
learner_pf_pid=""
trailbase_pf_pid=""
if [[ "${WORKLOAD_MANAGE_PORT_FORWARD}" == "1" ]]; then
  WORKLOAD_VOTER_LOCAL_PORT="$(reserve_local_port "${WORKLOAD_VOTER_LOCAL_PORT}" "voter")"
  WORKLOAD_LEARNER_LOCAL_PORT="$(reserve_local_port "${WORKLOAD_LEARNER_LOCAL_PORT}" "learner")"
  WORKLOAD_TRAILBASE_LOCAL_PORT="$(reserve_local_port "${WORKLOAD_TRAILBASE_LOCAL_PORT}" "trailbase")"
fi
voter_pf_log="$(mktemp)"
learner_pf_log="$(mktemp)"
trailbase_pf_log="$(mktemp)"
workload_should_stop=0

cleanup() {
  workload_should_stop=1
  if [[ -n "${voter_pf_pid}" ]]; then
    kill "${voter_pf_pid}" >/dev/null 2>&1 || true
  fi
  if [[ -n "${learner_pf_pid}" ]]; then
    kill "${learner_pf_pid}" >/dev/null 2>&1 || true
  fi
  if [[ -n "${trailbase_pf_pid}" ]]; then
    kill "${trailbase_pf_pid}" >/dev/null 2>&1 || true
  fi
  rm -f "${voter_pf_log}" "${learner_pf_log}" "${trailbase_pf_log}"
}
trap cleanup EXIT
trap 'workload_should_stop=1' INT TERM

wait_for_http() {
  local url="$1"
  local validator="${2:-}"
  local attempts="${3:-60}"
  local delay="${4:-1}"
  local body=""
  for _ in $(seq 1 "${attempts}"); do
    if body="$(
      curl -fsS \
      --connect-timeout "${HTTP_CONNECT_TIMEOUT_SECONDS}" \
      --max-time "${HTTP_CURL_TIMEOUT_SECONDS}" \
      "${url}" 2>/dev/null
    )"; then
      if [[ -z "${validator}" ]] || "${validator}" "${body}"; then
        return 0
      fi
    fi
    sleep "${delay}"
  done
  return 1
}

stop_port_forward() {
  local pid="$1"
  if [[ -n "${pid}" ]]; then
    kill "${pid}" >/dev/null 2>&1 || true
    wait "${pid}" 2>/dev/null || true
  fi
}

start_port_forward() {
  local service_name="$1"
  local local_port="$2"
  local target_port="$3"
  local log_file="$4"
  local pid_var_name="$5"
  local current_pid="${!pid_var_name:-}"

  if [[ -n "${current_pid}" ]]; then
    stop_port_forward "${current_pid}"
  fi

  : >"${log_file}"
  kubectl -n "${NAMESPACE}" port-forward "svc/${service_name}" "${local_port}:${target_port}" >"${log_file}" 2>&1 &
  printf -v "${pid_var_name}" '%s' "$!"
  if ! wait_for_http "http://127.0.0.1:${local_port}/healthz" validate_certd_health; then
    echo "[workload] port-forward failed for svc/${service_name}" >&2
    tail -n 50 "${log_file}" >&2 || true
    return 1
  fi
}

start_trailbase_port_forward() {
  local current_pid="${trailbase_pf_pid:-}"

  if [[ -n "${current_pid}" ]]; then
    stop_port_forward "${current_pid}"
  fi

  : >"${trailbase_pf_log}"
  kubectl -n "${NAMESPACE}" port-forward svc/trailbase "${WORKLOAD_TRAILBASE_LOCAL_PORT}:4000" >"${trailbase_pf_log}" 2>&1 &
  trailbase_pf_pid="$!"
  if ! wait_for_http "http://127.0.0.1:${WORKLOAD_TRAILBASE_LOCAL_PORT}/api/healthcheck" validate_trailbase_health; then
    echo "[workload] port-forward failed for svc/trailbase" >&2
    tail -n 50 "${trailbase_pf_log}" >&2 || true
    return 1
  fi
}

ensure_target_api() {
  local target="$1"
  case "${target}" in
    voter)
      local body=""
      if ! body="$(
        curl -fsS \
        --connect-timeout "${HTTP_CONNECT_TIMEOUT_SECONDS}" \
        --max-time "${HTTP_CURL_TIMEOUT_SECONDS}" \
        "http://127.0.0.1:${WORKLOAD_VOTER_LOCAL_PORT}/healthz"
      )" || ! validate_certd_health "${body}"; then
        if [[ "${WORKLOAD_MANAGE_PORT_FORWARD}" != "1" ]]; then
          return 1
        fi
        start_port_forward "${WORKLOAD_VOTER_SERVICE}" "${WORKLOAD_VOTER_LOCAL_PORT}" 80 "${voter_pf_log}" voter_pf_pid
      fi
      ;;
    learner)
      local body=""
      if ! body="$(
        curl -fsS \
        --connect-timeout "${HTTP_CONNECT_TIMEOUT_SECONDS}" \
        --max-time "${HTTP_CURL_TIMEOUT_SECONDS}" \
        "http://127.0.0.1:${WORKLOAD_LEARNER_LOCAL_PORT}/healthz"
      )" || ! validate_certd_health "${body}"; then
        if [[ "${WORKLOAD_MANAGE_PORT_FORWARD}" != "1" ]]; then
          return 1
        fi
        start_port_forward "${WORKLOAD_LEARNER_SERVICE}" "${WORKLOAD_LEARNER_LOCAL_PORT}" 8080 "${learner_pf_log}" learner_pf_pid
      fi
      ;;
    *)
      echo "unknown target: ${target}" >&2
      exit 1
      ;;
  esac
}

trailbase_base_url() {
  printf 'http://127.0.0.1:%s' "${WORKLOAD_TRAILBASE_LOCAL_PORT}"
}

current_phase() {
  if [[ -n "${WORKLOAD_PHASE_FILE}" && -f "${WORKLOAD_PHASE_FILE}" ]]; then
    cat "${WORKLOAD_PHASE_FILE}"
  else
    printf '%s' "unspecified"
  fi
}

utc_now() {
  date -u +"%Y-%m-%dT%H:%M:%SZ"
}

json_escape_arg() {
  local value="${1:-}"
  if [[ -z "${value}" ]]; then
    printf '%s' "null"
  else
    jq -Rn --arg value "${value}" '$value'
  fi
}

log_request() {
  local target="$1"
  local cycle="$2"
  local iteration="$3"
  local operation="$4"
  local method="$5"
  local endpoint="$6"
  local status="$7"
  local latency_seconds="$8"
  local facility_code="$9"
  local error_message="${10:-}"

  jq -nc \
    --arg ts "$(utc_now)" \
    --arg phase "$(current_phase)" \
    --arg target "${target}" \
    --arg operation "${operation}" \
    --arg method "${method}" \
    --arg endpoint "${endpoint}" \
    --argjson cycle "${cycle}" \
    --argjson iteration "${iteration}" \
    --argjson status "${status}" \
    --arg latency_seconds "${latency_seconds}" \
    --argjson facilityCode "$(json_escape_arg "${facility_code}")" \
    --argjson error "$(json_escape_arg "${error_message}")" \
    '{
      ts: $ts,
      phase: $phase,
      target: $target,
      operation: $operation,
      method: $method,
      endpoint: $endpoint,
      cycle: $cycle,
      iteration: $iteration,
      status: $status,
      latencySeconds: ($latency_seconds | tonumber? // 0),
      facilityCode: $facilityCode,
      error: $error
    }' >>"${WORKLOAD_LOG_FILE}"
}

HTTP_STATUS="0"
HTTP_LATENCY_SECONDS="0"
HTTP_BODY=""
HTTP_ERROR=""

validate_response_body() {
  local operation="$1"
  local body="$2"
  local facility_code="$3"

  case "${operation}" in
    login)
      jq -e '
        (.auth_token // .authToken // "") != "" and
        (.refresh_token // .refreshToken // "") != "" and
        (.csrf_token // .csrfToken // "") != ""
      ' >/dev/null <<<"${body}" || return 1
      ;;
    bootstrap-owner)
      jq -e '
        (.userId // .user_id // "") != "" and
        (.organizationId // .organization_id // "") != ""
      ' >/dev/null <<<"${body}" || return 1
      ;;
    current-user|current-user-strong)
      jq -e '
        (.user.id // "") != "" and
        ((.activeOrganizationId // .active_organization_id // "") != "")
      ' >/dev/null <<<"${body}" || return 1
      ;;
    list-plants|list-plants-strong)
      jq -e --arg facility_code "${facility_code}" '
        type == "array" and
        (
          ($facility_code == "") or
          any(.[]; (.facilityCode // .facility_code // "") == $facility_code)
        )
      ' >/dev/null <<<"${body}" || return 1
      ;;
    inspect-plant|inspect-plant-strong)
      jq -e --arg facility_code "${facility_code}" '
        (.plant.facilityCode // .plant.facility_code // "") == $facility_code
      ' >/dev/null <<<"${body}" || return 1
      ;;
    create-plant)
      jq -e --arg facility_code "${facility_code}" '
        (.facilityCode // .facility_code // "") == $facility_code
      ' >/dev/null <<<"${body}" || return 1
      ;;
    logout)
      return 0
      ;;
    *)
      return 0
      ;;
  esac
}

validate_expected_error_body() {
  local operation="$1"
  local status="$2"
  local body="$3"

  case "${operation}" in
    reject-create-plant)
      [[ "${status}" == "503" ]] || return 1
      jq -e '.error == "endpoint requires voter node"' >/dev/null <<<"${body}" || return 1
      ;;
    *)
      return 1
      ;;
  esac
}

request_api() {
  local target="$1"
  local base_url="$2"
  local cycle="$3"
  local iteration="$4"
  local operation="$5"
  local method="$6"
  local endpoint="$7"
  local facility_code="$8"
  shift 8

  local body_file err_file curl_meta curl_status
  body_file="$(mktemp)"
  err_file="$(mktemp)"

  ensure_target_api "${target}" || return 1

  set +e
  curl_meta="$(
    curl -sS -o "${body_file}" -w '%{http_code} %{time_total}' \
      --connect-timeout "${HTTP_CONNECT_TIMEOUT_SECONDS}" \
      --max-time "${HTTP_CURL_TIMEOUT_SECONDS}" \
      -X "${method}" "${base_url}${endpoint}" "$@" 2>"${err_file}"
  )"
  curl_status=$?
  set -e

  if [[ "${curl_status}" -ne 0 ]]; then
    ensure_target_api "${target}" || return 1
    set +e
    curl_meta="$(
      curl -sS -o "${body_file}" -w '%{http_code} %{time_total}' \
        --connect-timeout "${HTTP_CONNECT_TIMEOUT_SECONDS}" \
        --max-time "${HTTP_CURL_TIMEOUT_SECONDS}" \
        -X "${method}" "${base_url}${endpoint}" "$@" 2>"${err_file}"
    )"
    curl_status=$?
    set -e
  fi

  HTTP_BODY="$(cat "${body_file}")"
  if [[ "${curl_status}" -eq 0 ]]; then
    HTTP_STATUS="${curl_meta%% *}"
    HTTP_LATENCY_SECONDS="${curl_meta#* }"
    if [[ "${HTTP_STATUS}" -ge 200 && "${HTTP_STATUS}" -lt 300 ]]; then
      if ! validate_response_body "${operation}" "${HTTP_BODY}" "${facility_code}"; then
        HTTP_ERROR="response body validation failed"
        log_request "${target}" "${cycle}" "${iteration}" "${operation}" "${method}" "${endpoint}" "${HTTP_STATUS}" "${HTTP_LATENCY_SECONDS}" "${facility_code}" "${HTTP_ERROR}"
        rm -f "${body_file}" "${err_file}"
        return 1
      fi
      HTTP_ERROR=""
      log_request "${target}" "${cycle}" "${iteration}" "${operation}" "${method}" "${endpoint}" "${HTTP_STATUS}" "${HTTP_LATENCY_SECONDS}" "${facility_code}"
      rm -f "${body_file}" "${err_file}"
      return 0
    fi
    HTTP_ERROR="$(cat "${body_file}")"
    log_request "${target}" "${cycle}" "${iteration}" "${operation}" "${method}" "${endpoint}" "${HTTP_STATUS}" "${HTTP_LATENCY_SECONDS}" "${facility_code}" "${HTTP_ERROR}"
    rm -f "${body_file}" "${err_file}"
    return 1
  fi

  HTTP_STATUS="0"
  HTTP_LATENCY_SECONDS="0"
  HTTP_ERROR="$(cat "${err_file}")"
  log_request "${target}" "${cycle}" "${iteration}" "${operation}" "${method}" "${endpoint}" 0 0 "${facility_code}" "${HTTP_ERROR}"
  rm -f "${body_file}" "${err_file}"
  return 1
}

request_api_expect_error() {
  local target="$1"
  local base_url="$2"
  local cycle="$3"
  local iteration="$4"
  local operation="$5"
  local method="$6"
  local endpoint="$7"
  local facility_code="$8"
  shift 8

  local body_file err_file curl_meta curl_status
  body_file="$(mktemp)"
  err_file="$(mktemp)"

  ensure_target_api "${target}" || return 1

  set +e
  curl_meta="$(
    curl -sS -o "${body_file}" -w '%{http_code} %{time_total}' \
      --connect-timeout "${HTTP_CONNECT_TIMEOUT_SECONDS}" \
      --max-time "${HTTP_CURL_TIMEOUT_SECONDS}" \
      -X "${method}" "${base_url}${endpoint}" "$@" 2>"${err_file}"
  )"
  curl_status=$?
  set -e

  if [[ "${curl_status}" -ne 0 ]]; then
    ensure_target_api "${target}" || return 1
    set +e
    curl_meta="$(
      curl -sS -o "${body_file}" -w '%{http_code} %{time_total}' \
        --connect-timeout "${HTTP_CONNECT_TIMEOUT_SECONDS}" \
        --max-time "${HTTP_CURL_TIMEOUT_SECONDS}" \
        -X "${method}" "${base_url}${endpoint}" "$@" 2>"${err_file}"
    )"
    curl_status=$?
    set -e
  fi

  HTTP_BODY="$(cat "${body_file}")"
  if [[ "${curl_status}" -eq 0 ]]; then
    HTTP_STATUS="${curl_meta%% *}"
    HTTP_LATENCY_SECONDS="${curl_meta#* }"
    if validate_expected_error_body "${operation}" "${HTTP_STATUS}" "${HTTP_BODY}"; then
      HTTP_ERROR=""
      log_request "${target}" "${cycle}" "${iteration}" "${operation}" "${method}" "${endpoint}" "${HTTP_STATUS}" "${HTTP_LATENCY_SECONDS}" "${facility_code}"
      rm -f "${body_file}" "${err_file}"
      return 0
    fi
    HTTP_ERROR="$(cat "${body_file}")"
    log_request "${target}" "${cycle}" "${iteration}" "${operation}" "${method}" "${endpoint}" "${HTTP_STATUS}" "${HTTP_LATENCY_SECONDS}" "${facility_code}" "${HTTP_ERROR}"
    rm -f "${body_file}" "${err_file}"
    return 1
  fi

  HTTP_STATUS="0"
  HTTP_LATENCY_SECONDS="0"
  HTTP_ERROR="$(cat "${err_file}")"
  log_request "${target}" "${cycle}" "${iteration}" "${operation}" "${method}" "${endpoint}" 0 0 "${facility_code}" "${HTTP_ERROR}"
  rm -f "${body_file}" "${err_file}"
  return 1
}

login_trailbase() {
  local target="$1"
  local _base_url="$2"
  local cycle="$3"
  local trailbase_url
  trailbase_url="$(trailbase_base_url)"

  request_api \
    "${target}" "${trailbase_url}" "${cycle}" 0 "login" POST "/api/auth/v1/login" "" \
    -H 'content-type: application/json' \
    --data-binary "$(jq -nc \
      --arg email "${TRAILBASE_USER_EMAIL}" \
      --arg password "${TRAILBASE_USER_PASSWORD}" \
      '{email: $email, password: $password}')"
}

bootstrap_owner() {
  local target="$1"
  local base_url="$2"
  local cycle="$3"
  local auth_token="$4"

  request_api \
    "${target}" "${base_url}" "${cycle}" 0 "bootstrap-owner" POST "/api/v1/admin/bootstrap/trailbase-owner" "" \
    -H "authorization: Bearer ${auth_token}" \
    -H "x-certd-master-key: ${CERTD_MASTER_KEY}"
}

current_user() {
  local target="$1"
  local base_url="$2"
  local cycle="$3"
  local iteration="$4"
  local auth_token="$5"
  local active_org="$6"
  local operation="${7:-current-user}"
  local endpoint="${8:-/api/v1/me}"

  request_api \
    "${target}" "${base_url}" "${cycle}" "${iteration}" "${operation}" GET "${endpoint}" "" \
    -H "authorization: Bearer ${auth_token}" \
    -H "x-organization-id: ${active_org}"
}

list_plants() {
  local target="$1"
  local base_url="$2"
  local cycle="$3"
  local iteration="$4"
  local auth_token="$5"
  local active_org="$6"
  local operation="${7:-list-plants}"
  local endpoint="${8:-/api/v1/plants}"

  request_api \
    "${target}" "${base_url}" "${cycle}" "${iteration}" "${operation}" GET "${endpoint}" "${WORKLOAD_READ_FACILITY_CODE}" \
    -H "authorization: Bearer ${auth_token}" \
    -H "x-organization-id: ${active_org}"
}

create_plant() {
  local target="$1"
  local base_url="$2"
  local cycle="$3"
  local iteration="$4"
  local auth_token="$5"
  local active_org="$6"
  local facility_code="$7"

  request_api \
    "${target}" "${base_url}" "${cycle}" "${iteration}" "create-plant" POST "/api/v1/organizations/${active_org}/plants" "${facility_code}" \
    -H 'content-type: application/json' \
    -H "authorization: Bearer ${auth_token}" \
    -H "x-organization-id: ${active_org}" \
    --data-binary "$(jq -nc \
      --arg facilityCode "${facility_code}" \
      --arg name "${target}-${facility_code}" \
      --arg contractNo "${target}-${facility_code}" \
      --arg address "Seoul" \
      --arg recStartDate "2026-01-01" \
      --arg confirmIssued "Y" \
      --arg ownerOrganizationId "${active_org}" \
      '{
        facilityCode: $facilityCode,
        name: $name,
        contractNo: $contractNo,
        address: $address,
        installedKw: 50.0,
        recStartDate: $recStartDate,
        confirmIssued: $confirmIssued,
        ownerOrganizationId: $ownerOrganizationId
      }')"
}

expect_rejected_create_plant() {
  local target="$1"
  local base_url="$2"
  local cycle="$3"
  local iteration="$4"
  local auth_token="$5"
  local active_org="$6"
  local facility_code="$7"

  request_api_expect_error \
    "${target}" "${base_url}" "${cycle}" "${iteration}" "reject-create-plant" POST "/api/v1/organizations/${active_org}/plants" "${facility_code}" \
    -H 'content-type: application/json' \
    -H "authorization: Bearer ${auth_token}" \
    -H "x-organization-id: ${active_org}" \
    --data-binary "$(jq -nc \
      --arg facilityCode "${facility_code}" \
      --arg name "${target}-${facility_code}" \
      --arg contractNo "${target}-${facility_code}" \
      --arg address "Seoul" \
      --arg recStartDate "2026-01-01" \
      --arg confirmIssued "Y" \
      --arg ownerOrganizationId "${active_org}" \
      '{
        facilityCode: $facilityCode,
        name: $name,
        contractNo: $contractNo,
        address: $address,
        installedKw: 50.0,
        recStartDate: $recStartDate,
        confirmIssued: $confirmIssued,
        ownerOrganizationId: $ownerOrganizationId
      }')"
}

inspect_plant() {
  local target="$1"
  local base_url="$2"
  local cycle="$3"
  local iteration="$4"
  local auth_token="$5"
  local active_org="$6"
  local facility_code="$7"
  local operation="${8:-inspect-plant}"
  local endpoint="${9:-/api/v1/plants/${facility_code}}"

  request_api \
    "${target}" "${base_url}" "${cycle}" "${iteration}" "${operation}" GET "${endpoint}" "${facility_code}" \
    -H "authorization: Bearer ${auth_token}" \
    -H "x-organization-id: ${active_org}"
}

logout_trailbase() {
  local target="$1"
  local _base_url="$2"
  local cycle="$3"
  local auth_token="$4"
  local refresh_token="$5"
  local csrf_token="$6"
  local trailbase_url
  trailbase_url="$(trailbase_base_url)"

  request_api \
    "${target}" "${trailbase_url}" "${cycle}" 0 "logout" POST "/api/auth/v1/logout" "" \
    -H 'content-type: application/json' \
    -H "authorization: Bearer ${auth_token}" \
    -H "x-csrf-token: ${csrf_token}" \
    --data-binary "$(jq -nc --arg refreshToken "${refresh_token}" '{refresh_token: $refreshToken}')"
}

target_base_url() {
  local target="$1"
  case "${target}" in
    voter) printf 'http://127.0.0.1:%s' "${WORKLOAD_VOTER_LOCAL_PORT}" ;;
    learner) printf 'http://127.0.0.1:%s' "${WORKLOAD_LEARNER_LOCAL_PORT}" ;;
    *)
      echo "unknown target: ${target}" >&2
      exit 1
      ;;
  esac
}

cycle_facility_code() {
  local target="$1"
  local cycle="$2"
  local iteration="$3"
  case "${target}" in
    voter) printf 'VT%sC%04dI%02d' "${SANITIZED_RUN_ID}" "${cycle}" "${iteration}" ;;
    learner) printf 'LR%sC%04dI%02d' "${SANITIZED_RUN_ID}" "${cycle}" "${iteration}" ;;
    *)
      echo "unknown target: ${target}" >&2
      exit 1
      ;;
  esac
}

prepare_target() {
  local target="$1"
  local base_url="$2"

  if ! login_trailbase "${target}" "${base_url}" 0; then
    return 1
  fi

  local auth_token refresh_token csrf_token active_org
  auth_token="$(jq -r '.auth_token // .authToken // empty' <<<"${HTTP_BODY}")"
  refresh_token="$(jq -r '.refresh_token // .refreshToken // empty' <<<"${HTTP_BODY}")"
  csrf_token="$(jq -r '.csrf_token // .csrfToken // empty' <<<"${HTTP_BODY}")"
  if [[ -z "${auth_token}" || -z "${refresh_token}" || -z "${csrf_token}" ]]; then
    return 1
  fi

  if [[ "${target}" == "voter" ]]; then
    bootstrap_owner "${target}" "${base_url}" 0 "${auth_token}" || true
  fi
  if ! current_user "${target}" "${base_url}" 0 0 "${auth_token}" "${BOOTSTRAP_ORGANIZATION_ID}"; then
    return 1
  fi
  active_org="$(jq -r '.activeOrganizationId // .active_organization_id // empty' <<<"${HTTP_BODY}")"
  if [[ -z "${active_org}" ]]; then
    active_org="${BOOTSTRAP_ORGANIZATION_ID}"
  fi

  current_user "${target}" "${base_url}" 0 0 "${auth_token}" "${active_org}" || true
  current_user "${target}" "${base_url}" 0 0 "${auth_token}" "${active_org}" "current-user-strong" "/api/v1/me?consistency=strong" || true
  list_plants "${target}" "${base_url}" 0 0 "${auth_token}" "${active_org}" || true
  list_plants "${target}" "${base_url}" 0 0 "${auth_token}" "${active_org}" "list-plants-strong" "/api/v1/plants?consistency=strong" || true
  inspect_plant "${target}" "${base_url}" 0 0 "${auth_token}" "${active_org}" "${WORKLOAD_READ_FACILITY_CODE}" || true
  inspect_plant "${target}" "${base_url}" 0 0 "${auth_token}" "${active_org}" "${WORKLOAD_READ_FACILITY_CODE}" "inspect-plant-strong" "/api/v1/plants/${WORKLOAD_READ_FACILITY_CODE}?consistency=strong" || true
  logout_trailbase "${target}" "${base_url}" 0 "${auth_token}" "${refresh_token}" "${csrf_token}" || true
  printf '%s' "${active_org}"
}

run_target_cycle() {
  local target="$1"
  local cycle="$2"
  local active_org="$3"
  local base_url
  base_url="$(target_base_url "${target}")"

  if ! login_trailbase "${target}" "${base_url}" "${cycle}"; then
    return 0
  fi

  local auth_token refresh_token csrf_token cycle_facility
  auth_token="$(jq -r '.auth_token // .authToken // empty' <<<"${HTTP_BODY}")"
  refresh_token="$(jq -r '.refresh_token // .refreshToken // empty' <<<"${HTTP_BODY}")"
  csrf_token="$(jq -r '.csrf_token // .csrfToken // empty' <<<"${HTTP_BODY}")"
  if [[ -z "${auth_token}" || -z "${refresh_token}" || -z "${csrf_token}" ]]; then
    log_request "${target}" "${cycle}" 0 "login-parse" "POST" "/api/auth/v1/login" 0 0 "" "missing auth token fields"
    return 0
  fi

  if [[ "${target}" == "voter" ]]; then
    bootstrap_owner "${target}" "${base_url}" "${cycle}" "${auth_token}" || true
  fi
  current_user "${target}" "${base_url}" "${cycle}" 0 "${auth_token}" "${active_org}" || true
  current_user "${target}" "${base_url}" "${cycle}" 0 "${auth_token}" "${active_org}" "current-user-strong" "/api/v1/me?consistency=strong" || true

  if [[ "${target}" == "voter" ]]; then
    cycle_facility="$(cycle_facility_code "${target}" "${cycle}" 0)"
    create_plant "${target}" "${base_url}" "${cycle}" 0 "${auth_token}" "${active_org}" "${cycle_facility}" || true
  else
    cycle_facility="$(cycle_facility_code "${target}" "${cycle}" 0)"
    expect_rejected_create_plant "${target}" "${base_url}" "${cycle}" 0 "${auth_token}" "${active_org}" "${cycle_facility}" || true
  fi

  for iteration in $(seq 1 "${WORKLOAD_READ_WRITE_ITERATIONS}"); do
    if [[ "${workload_should_stop}" -ne 0 ]]; then
      break
    fi
    list_plants "${target}" "${base_url}" "${cycle}" "${iteration}" "${auth_token}" "${active_org}" || true
    list_plants "${target}" "${base_url}" "${cycle}" "${iteration}" "${auth_token}" "${active_org}" "list-plants-strong" "/api/v1/plants?consistency=strong" || true
    inspect_plant "${target}" "${base_url}" "${cycle}" "${iteration}" "${auth_token}" "${active_org}" "${WORKLOAD_READ_FACILITY_CODE}" || true
    inspect_plant "${target}" "${base_url}" "${cycle}" "${iteration}" "${auth_token}" "${active_org}" "${WORKLOAD_READ_FACILITY_CODE}" "inspect-plant-strong" "/api/v1/plants/${WORKLOAD_READ_FACILITY_CODE}?consistency=strong" || true
    if [[ "${target}" == "voter" ]]; then
      create_plant \
        "${target}" "${base_url}" "${cycle}" "${iteration}" "${auth_token}" "${active_org}" \
        "$(cycle_facility_code "${target}" "${cycle}" "${iteration}")" || true
    else
      expect_rejected_create_plant \
        "${target}" "${base_url}" "${cycle}" "${iteration}" "${auth_token}" "${active_org}" \
        "$(cycle_facility_code "${target}" "${cycle}" "${iteration}")" || true
    fi
  done

  logout_trailbase "${target}" "${base_url}" "${cycle}" "${auth_token}" "${refresh_token}" "${csrf_token}" || true
}

main() {
  mkdir -p "$(dirname "${WORKLOAD_LOG_FILE}")"
  : >"${WORKLOAD_LOG_FILE}"

  if [[ "${WORKLOAD_MANAGE_PORT_FORWARD}" == "1" ]]; then
    start_port_forward "${WORKLOAD_VOTER_SERVICE}" "${WORKLOAD_VOTER_LOCAL_PORT}" 80 "${voter_pf_log}" voter_pf_pid
    start_port_forward "${WORKLOAD_LEARNER_SERVICE}" "${WORKLOAD_LEARNER_LOCAL_PORT}" 8080 "${learner_pf_log}" learner_pf_pid
    start_trailbase_port_forward
  fi

  local voter_base learner_base voter_org learner_org
  voter_base="$(target_base_url voter)"
  learner_base="$(target_base_url learner)"

  voter_org="$(prepare_target voter "${voter_base}")"
  learner_org="$(prepare_target learner "${learner_base}")"

  echo "chaos load log file: ${WORKLOAD_LOG_FILE}"
  echo "starting workload loops: voter=${voter_base} learner=${learner_base}"

  local cycle=0
  while [[ "${workload_should_stop}" -eq 0 ]]; do
    cycle=$((cycle + 1))
    if [[ "${WORKLOAD_MAX_CYCLES}" -gt 0 && "${cycle}" -gt "${WORKLOAD_MAX_CYCLES}" ]]; then
      break
    fi

    run_target_cycle voter "${cycle}" "${voter_org}"
    if [[ "${workload_should_stop}" -ne 0 ]]; then
      break
    fi

    run_target_cycle learner "${cycle}" "${learner_org}"
    if [[ "${workload_should_stop}" -ne 0 ]]; then
      break
    fi

    sleep "${WORKLOAD_CYCLE_INTERVAL_SECONDS}"
  done
}

main "$@"
