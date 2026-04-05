#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
NAMESPACE="${NAMESPACE:-certd}"
LOCAL_PORT="${LOCAL_PORT:-18080}"
BOOTSTRAP_ORGANIZATION_ID="${BOOTSTRAP_ORGANIZATION_ID:-conalog-account}"
CERTD_MASTER_KEY="${CERTD_MASTER_KEY:-local-dev-master-key}"
CERTD_STABILIZE_SECONDS="${CERTD_STABILIZE_SECONDS:-60}"
RUN_ID="${RUN_ID:-$(date +%s)}"
PLANT_PREFIX="${PLANT_PREFIX:-TB-PROD-${RUN_ID}}"
OWNER_EMAIL="${OWNER_EMAIL:-trailbase-user@example.com}"
OWNER_PASSWORD="${OWNER_PASSWORD:-secret}"
PHASE1_USER_EMAIL="${PHASE1_USER_EMAIL:-trailbase-phase1-${RUN_ID}@example.com}"
PHASE1_USER_PASSWORD="${PHASE1_USER_PASSWORD:-secret}"
PHASE2_USER_EMAIL="${PHASE2_USER_EMAIL:-trailbase-phase2-${RUN_ID}@example.com}"
PHASE2_USER_PASSWORD="${PHASE2_USER_PASSWORD:-secret}"

need() {
  command -v "$1" >/dev/null 2>&1 || {
    echo "missing command: $1" >&2
    exit 1
  }
}

need kubectl
need curl
need jq
need sort

PHASE1_CODES=("${PLANT_PREFIX}-A" "${PLANT_PREFIX}-B")
PHASE2_CODES=("${PLANT_PREFIX}-C" "${PLANT_PREFIX}-D")

pf_pid=""
port_forward_log="$(mktemp)"

cleanup() {
  if [[ -n "${pf_pid}" ]]; then
    kill "${pf_pid}" >/dev/null 2>&1 || true
  fi
  rm -f "${port_forward_log}"
}
trap cleanup EXIT

stop_port_forward() {
  if [[ -n "${pf_pid}" ]]; then
    kill "${pf_pid}" >/dev/null 2>&1 || true
    pf_pid=""
  fi
}

wait_for_http() {
  local url="$1"
  local attempts="${2:-60}"
  local delay="${3:-2}"
  for _ in $(seq 1 "${attempts}"); do
    if curl -fsS "${url}" >/dev/null 2>&1; then
      return 0
    fi
    sleep "${delay}"
  done
  echo "timed out waiting for ${url}" >&2
  exit 1
}

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

start_port_forward() {
  stop_port_forward
  kubectl -n "${NAMESPACE}" port-forward svc/certd "${LOCAL_PORT}:80" >"${port_forward_log}" 2>&1 &
  pf_pid=$!
  wait_for_http "http://127.0.0.1:${LOCAL_PORT}/healthz"
}

ensure_trailbase_online() {
  local replicas
  replicas="$(kubectl -n "${NAMESPACE}" get deployment trailbase -o jsonpath='{.spec.replicas}' 2>/dev/null || echo "")"
  if [[ "${replicas}" != "1" ]]; then
    kubectl -n "${NAMESPACE}" scale deployment/trailbase --replicas=1 >/dev/null
  fi
  kubectl -n "${NAMESPACE}" rollout status deployment/trailbase --timeout=15m >/dev/null
}

seed_trailbase_user() {
  local email="$1"
  local password="$2"
  TRAILBASE_USER_EMAIL="${email}" \
  TRAILBASE_USER_PASSWORD="${password}" \
  NAMESPACE="${NAMESPACE}" \
    bash "${ROOT_DIR}/scripts/k3d-seed-trailbase-user.sh" >/dev/null
}

login_trailbase() {
  local email="$1"
  local password="$2"
  curl -fsS -X POST \
    "http://127.0.0.1:${LOCAL_PORT}/api/auth/v1/login" \
    -H 'content-type: application/json' \
    --data-binary "$(jq -n --arg email "${email}" --arg password "${password}" '{email: $email, password: $password}')" \
    | jq -r '.auth_token // .authToken // empty'
}

bootstrap_owner() {
  local auth_token="$1"
  curl -fsS -X POST \
    "http://127.0.0.1:${LOCAL_PORT}/api/v1/admin/bootstrap/trailbase-owner" \
    -H "authorization: Bearer ${auth_token}" \
    -H "x-certd-master-key: ${CERTD_MASTER_KEY}" >/dev/null
}

current_user() {
  local auth_token="$1"
  local active_org="$2"
  curl -fsS \
    "http://127.0.0.1:${LOCAL_PORT}/api/v1/me" \
    -H "authorization: Bearer ${auth_token}" \
    -H "x-organization-id: ${active_org}"
}

list_plants() {
  local auth_token="$1"
  local active_org="$2"
  curl -fsS \
    "http://127.0.0.1:${LOCAL_PORT}/api/v1/plants" \
    -H "authorization: Bearer ${auth_token}" \
    -H "x-organization-id: ${active_org}"
}

create_plant() {
  local auth_token="$1"
  local active_org="$2"
  local facility_code="$3"
  local ordinal="$4"
  curl -fsS -X POST \
    "http://127.0.0.1:${LOCAL_PORT}/api/v1/organizations/${active_org}/plants" \
    -H 'content-type: application/json' \
    -H "authorization: Bearer ${auth_token}" \
    -H "x-organization-id: ${active_org}" \
    --data-binary "$(jq -n \
      --arg facilityCode "${facility_code}" \
      --arg name "Production Chaos Plant ${ordinal}" \
      --arg contractNo "${facility_code}-CONTRACT" \
      --arg address "Seoul ${ordinal}" \
      --arg ownerOrganizationId "${active_org}" \
      '{
        facilityCode: $facilityCode,
        name: $name,
        contractNo: $contractNo,
        address: $address,
        installedKw: 50,
        recStartDate: "2026-01-01",
        confirmIssued: "Y",
        ownerOrganizationId: $ownerOrganizationId
      }')" >/dev/null
}

inspect_plant() {
  local auth_token="$1"
  local active_org="$2"
  local facility_code="$3"
  curl -fsS \
    "http://127.0.0.1:${LOCAL_PORT}/api/v1/plants/${facility_code}" \
    -H "authorization: Bearer ${auth_token}" \
    -H "x-organization-id: ${active_org}"
}

plant_status_code() {
  local auth_token="$1"
  local active_org="$2"
  local facility_code="$3"
  curl -sS -o /dev/null -w '%{http_code}' \
    "http://127.0.0.1:${LOCAL_PORT}/api/v1/plants/${facility_code}" \
    -H "authorization: Bearer ${auth_token}" \
    -H "x-organization-id: ${active_org}"
}

trigger_backup() {
  curl -fsS -X POST \
    "http://127.0.0.1:${LOCAL_PORT}/api/v1/admin/backups" \
    -H "x-certd-master-key: ${CERTD_MASTER_KEY}" >/dev/null
}

list_backups_s3() {
  curl -fsS \
    "http://127.0.0.1:${LOCAL_PORT}/api/v1/admin/backups/s3" \
    -H "x-certd-master-key: ${CERTD_MASTER_KEY}"
}

latest_s3_backup_name() {
  list_backups_s3 | jq -r '.items | sort_by(.lastModified // 0) | last | .name // empty'
}

wait_for_backup_cooldown() {
  local latest_last_modified now wait_seconds
  latest_last_modified="$(list_backups_s3 | jq -r '.items | sort_by(.lastModified // 0) | last | .lastModified // 0')"
  if [[ -z "${latest_last_modified}" || "${latest_last_modified}" == "0" ]]; then
    return 0
  fi

  now="$(date +%s)"
  wait_seconds="$(( latest_last_modified + 61 - now ))"
  if (( wait_seconds > 0 )); then
    echo "waiting ${wait_seconds}s for backup dedupe cooldown"
    sleep "${wait_seconds}"
  fi
}

wait_for_new_backup() {
  local previous_name="$1"
  local backup_name=""
  for _ in $(seq 1 60); do
    backup_name="$(latest_s3_backup_name)"
    if [[ -n "${backup_name}" && "${backup_name}" != "${previous_name}" ]]; then
      echo "${backup_name}"
      return 0
    fi
    sleep 2
  done
  echo "timed out waiting for new backup object" >&2
  exit 1
}

sorted_expected_codes() {
  printf '%s\n' "$@" | sort
}

sorted_actual_codes() {
  local auth_token="$1"
  local active_org="$2"
  list_plants "${auth_token}" "${active_org}" \
    | jq -r --arg prefix "${PLANT_PREFIX}" '.[] | (.facilityCode // .facility_code) | select(startswith($prefix))' \
    | sort
}

assert_visible_codes() {
  local auth_token="$1"
  local active_org="$2"
  shift 2
  local expected actual
  expected="$(sorted_expected_codes "$@")"
  actual="$(sorted_actual_codes "${auth_token}" "${active_org}")"
  if [[ "${expected}" != "${actual}" ]]; then
    echo "visible plant set mismatch" >&2
    echo "expected:" >&2
    printf '%s\n' "${expected}" >&2
    echo "actual:" >&2
    printf '%s\n' "${actual}" >&2
    exit 1
  fi
}

assert_present_and_readable() {
  local auth_token="$1"
  local active_org="$2"
  local facility_code="$3"
  inspect_plant "${auth_token}" "${active_org}" "${facility_code}" | jq -e --arg code "${facility_code}" '
    .plant.facilityCode == $code or .plant.facility_code == $code
  ' >/dev/null
}

assert_missing() {
  local auth_token="$1"
  local active_org="$2"
  local facility_code="$3"
  local status
  status="$(plant_status_code "${auth_token}" "${active_org}" "${facility_code}")"
  if [[ "${status}" == "200" ]]; then
    echo "expected ${facility_code} to be absent after restore, but it is still readable" >&2
    exit 1
  fi
}

restore_certd_from_backup() {
  local backup_name="$1"
  local voter_replicas
  voter_replicas="$(kubectl -n "${NAMESPACE}" get statefulset certd-voter -o jsonpath='{.spec.replicas}')"

  stop_port_forward
  kubectl -n "${NAMESPACE}" scale deployment/certd-learner-1 --replicas=0 >/dev/null
  kubectl -n "${NAMESPACE}" scale deployment/certd-learner-2 --replicas=0 >/dev/null
  kubectl -n "${NAMESPACE}" scale deployment/certd-learner-3 --replicas=0 >/dev/null
  kubectl -n "${NAMESPACE}" scale statefulset/certd-voter --replicas=0 >/dev/null
  kubectl -n "${NAMESPACE}" wait --for=delete pod -l app=certd --timeout=10m >/dev/null

  kubectl -n "${NAMESPACE}" set env statefulset/certd-voter HQL_BACKUP_RESTORE="s3:${backup_name}" >/dev/null
  kubectl -n "${NAMESPACE}" scale statefulset/certd-voter --replicas="${voter_replicas}" >/dev/null
  kubectl -n "${NAMESPACE}" rollout status statefulset/certd-voter --timeout=15m >/dev/null
  kubectl -n "${NAMESPACE}" scale deployment/certd-learner-1 --replicas=1 >/dev/null
  kubectl -n "${NAMESPACE}" scale deployment/certd-learner-2 --replicas=1 >/dev/null
  kubectl -n "${NAMESPACE}" scale deployment/certd-learner-3 --replicas=1 >/dev/null
  kubectl -n "${NAMESPACE}" rollout status deployment/certd-learner-1 --timeout=15m >/dev/null
  kubectl -n "${NAMESPACE}" rollout status deployment/certd-learner-2 --timeout=15m >/dev/null
  kubectl -n "${NAMESPACE}" rollout status deployment/certd-learner-3 --timeout=15m >/dev/null

  sleep "${CERTD_STABILIZE_SECONDS}"
  start_port_forward
}

login_and_bootstrap_owner() {
  local token me_json active_org user_id
  token="$(retry_capture 20 2 login_trailbase "${OWNER_EMAIL}" "${OWNER_PASSWORD}")"
  if [[ -z "${token}" || "${token}" == "null" ]]; then
    echo "failed to login owner" >&2
    exit 1
  fi
  retry_capture 20 2 bootstrap_owner "${token}" >/dev/null
  me_json="$(retry_capture 20 2 current_user "${token}" "${BOOTSTRAP_ORGANIZATION_ID}")"
  active_org="$(jq -r '.activeOrganizationId // .active_organization_id // empty' <<<"${me_json}")"
  user_id="$(jq -r '.user.id // empty' <<<"${me_json}")"
  if [[ -z "${active_org}" ]]; then
    active_org="${BOOTSTRAP_ORGANIZATION_ID}"
  fi
  echo "${token}"$'\n'"${active_org}"$'\n'"${user_id}"
}

verify_trailbase_latest_restore() {
  local pod_name phase1_token phase2_token
  phase1_token="$(retry_capture 20 2 login_trailbase "${PHASE1_USER_EMAIL}" "${PHASE1_USER_PASSWORD}")"
  phase2_token="$(retry_capture 20 2 login_trailbase "${PHASE2_USER_EMAIL}" "${PHASE2_USER_PASSWORD}")"
  if [[ -z "${phase1_token}" || -z "${phase2_token}" ]]; then
    echo "failed to login secondary TrailBase users before restore" >&2
    exit 1
  fi

  stop_port_forward
  pod_name="$(kubectl -n "${NAMESPACE}" get pod -l app=trailbase -o jsonpath='{.items[0].metadata.name}')"
  kubectl -n "${NAMESPACE}" delete pod "${pod_name}" >/dev/null
  kubectl -n "${NAMESPACE}" rollout status deployment/trailbase --timeout=15m >/dev/null
  start_port_forward

  phase1_token="$(retry_capture 20 2 login_trailbase "${PHASE1_USER_EMAIL}" "${PHASE1_USER_PASSWORD}")"
  phase2_token="$(retry_capture 20 2 login_trailbase "${PHASE2_USER_EMAIL}" "${PHASE2_USER_PASSWORD}")"
  if [[ -z "${phase1_token}" || -z "${phase2_token}" ]]; then
    echo "failed to login secondary TrailBase users after restore" >&2
    exit 1
  fi
}

read_owner_login() {
  local login_output="$1"
  local line1 line2 line3
  line1="${login_output%%$'\n'*}"
  login_output="${login_output#*$'\n'}"
  line2="${login_output%%$'\n'*}"
  line3="${login_output#*$'\n'}"
  OWNER_TOKEN="${line1}"
  ACTIVE_ORG_ID="${line2}"
  OWNER_USER_ID="${line3}"
}

echo "production-like restore chaos run_id=${RUN_ID} prefix=${PLANT_PREFIX}"
ensure_trailbase_online
start_port_forward

seed_trailbase_user "${OWNER_EMAIL}" "${OWNER_PASSWORD}"
seed_trailbase_user "${PHASE1_USER_EMAIL}" "${PHASE1_USER_PASSWORD}"

read_owner_login "$(login_and_bootstrap_owner)"
echo "owner user=${OWNER_USER_ID} org=${ACTIVE_ORG_ID}"

create_plant "${OWNER_TOKEN}" "${ACTIVE_ORG_ID}" "${PHASE1_CODES[0]}" "phase1-a"
create_plant "${OWNER_TOKEN}" "${ACTIVE_ORG_ID}" "${PHASE1_CODES[1]}" "phase1-b"
assert_visible_codes "${OWNER_TOKEN}" "${ACTIVE_ORG_ID}" "${PHASE1_CODES[@]}"
assert_present_and_readable "${OWNER_TOKEN}" "${ACTIVE_ORG_ID}" "${PHASE1_CODES[0]}"
assert_present_and_readable "${OWNER_TOKEN}" "${ACTIVE_ORG_ID}" "${PHASE1_CODES[1]}"

wait_for_backup_cooldown
PREVIOUS_BACKUP_NAME="$(latest_s3_backup_name)"
trigger_backup
BACKUP1_NAME="$(wait_for_new_backup "${PREVIOUS_BACKUP_NAME}")"
echo "backup1=${BACKUP1_NAME}"

seed_trailbase_user "${PHASE2_USER_EMAIL}" "${PHASE2_USER_PASSWORD}"
create_plant "${OWNER_TOKEN}" "${ACTIVE_ORG_ID}" "${PHASE2_CODES[0]}" "phase2-c"
create_plant "${OWNER_TOKEN}" "${ACTIVE_ORG_ID}" "${PHASE2_CODES[1]}" "phase2-d"
assert_visible_codes "${OWNER_TOKEN}" "${ACTIVE_ORG_ID}" "${PHASE1_CODES[@]}" "${PHASE2_CODES[@]}"

wait_for_backup_cooldown
PREVIOUS_BACKUP_NAME="${BACKUP1_NAME}"
trigger_backup
BACKUP2_NAME="$(wait_for_new_backup "${PREVIOUS_BACKUP_NAME}")"
echo "backup2=${BACKUP2_NAME}"

echo "restoring certd to backup1"
restore_certd_from_backup "${BACKUP1_NAME}"
read_owner_login "$(login_and_bootstrap_owner)"
assert_visible_codes "${OWNER_TOKEN}" "${ACTIVE_ORG_ID}" "${PHASE1_CODES[@]}"
assert_present_and_readable "${OWNER_TOKEN}" "${ACTIVE_ORG_ID}" "${PHASE1_CODES[0]}"
assert_present_and_readable "${OWNER_TOKEN}" "${ACTIVE_ORG_ID}" "${PHASE1_CODES[1]}"
assert_missing "${OWNER_TOKEN}" "${ACTIVE_ORG_ID}" "${PHASE2_CODES[0]}"
assert_missing "${OWNER_TOKEN}" "${ACTIVE_ORG_ID}" "${PHASE2_CODES[1]}"

echo "restoring certd to backup2"
restore_certd_from_backup "${BACKUP2_NAME}"
read_owner_login "$(login_and_bootstrap_owner)"
assert_visible_codes "${OWNER_TOKEN}" "${ACTIVE_ORG_ID}" "${PHASE1_CODES[@]}" "${PHASE2_CODES[@]}"
for code in "${PHASE1_CODES[@]}" "${PHASE2_CODES[@]}"; do
  assert_present_and_readable "${OWNER_TOKEN}" "${ACTIVE_ORG_ID}" "${code}"
done

echo "verifying TrailBase latest restore"
verify_trailbase_latest_restore

echo "production-like restore chaos completed"
echo "backup1=${BACKUP1_NAME}"
echo "backup2=${BACKUP2_NAME}"
echo "phase1_codes=${PHASE1_CODES[*]}"
echo "phase2_codes=${PHASE2_CODES[*]}"
