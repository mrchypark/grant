#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
NAMESPACE="${NAMESPACE:-certd}"
LOCAL_PORT="${LOCAL_PORT:-18080}"
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

need() {
  command -v "$1" >/dev/null 2>&1 || {
    echo "missing command: $1" >&2
    exit 1
  }
}

wait_for_http() {
  local url="$1"
  local attempts="${2:-60}"
  local delay="${3:-2}"
  for _ in $(seq 1 "$attempts"); do
    if curl -fsS "$url" >/dev/null 2>&1; then
      return 0
    fi
    sleep "$delay"
  done
  echo "timed out waiting for $url" >&2
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

seed_trailbase_user() {
  TRAILBASE_USER_EMAIL="$TRAILBASE_USER_EMAIL" \
  TRAILBASE_USER_PASSWORD="$TRAILBASE_USER_PASSWORD" \
  NAMESPACE="$NAMESPACE" \
    bash "$ROOT_DIR/scripts/k3d-seed-trailbase-user.sh"
}

login_trailbase() {
  curl -fsS -X POST \
    "http://127.0.0.1:${LOCAL_PORT}/api/auth/v1/login" \
    -H 'content-type: application/json' \
    --data-binary "$(jq -n \
      --arg email "$TRAILBASE_USER_EMAIL" \
      --arg password "$TRAILBASE_USER_PASSWORD" \
      '{email: $email, password: $password}')" | jq -r '.auth_token // .authToken // empty'
}

bootstrap_owner() {
  local auth_token="$1"
  curl -fsS -X POST \
    "http://127.0.0.1:${LOCAL_PORT}/api/v1/admin/bootstrap/trailbase-owner" \
    -H "authorization: Bearer ${auth_token}" \
    -H "x-certd-master-key: ${CERTD_MASTER_KEY}"
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
  curl -fsS -X POST \
    "http://127.0.0.1:${LOCAL_PORT}/api/v1/organizations/${active_org}/plants" \
    -H 'content-type: application/json' \
    -H "authorization: Bearer ${auth_token}" \
    -H "x-organization-id: ${active_org}" \
    --data-binary "$(jq -n \
      --arg facilityCode "$PLANT_FACILITY_CODE" \
      --arg name "$PLANT_NAME" \
      --arg contractNo "$PLANT_CONTRACT_NO" \
      --arg address "$PLANT_ADDRESS" \
      --arg recStartDate "$PLANT_REC_START_DATE" \
      --arg confirmIssued "$PLANT_CONFIRM_ISSUED" \
      --arg ownerOrganizationId "$active_org" \
      --argjson installedKw "$PLANT_INSTALLED_KW" \
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
  curl -fsS \
    "http://127.0.0.1:${LOCAL_PORT}/api/v1/plants/${facility_code}" \
    -H "authorization: Bearer ${auth_token}" \
    -H "x-organization-id: ${active_org}"
}

maybe_upload_certificate() {
  local auth_token="$1"
  local active_org="$2"
  local sign_cert="${CERT_DIR}/signCert.der"
  local sign_pri="${CERT_DIR}/signPri.key"
  local km_cert="${CERT_DIR}/kmCert.der"
  local km_pri="${CERT_DIR}/kmPri.key"

  if [[ ! -f "$sign_cert" || ! -f "$sign_pri" || ! -f "$km_cert" || ! -f "$km_pri" ]]; then
    echo "certificate bundle not present; skipping cert upload and sync"
    return 0
  fi

  if [[ -z "$CERT_PASSWORD" ]]; then
    echo "CERT_PASSWORD not set; skipping cert upload and sync"
    return 0
  fi

  local payload
  payload="$(jq -n \
    --arg businessNo "$CERT_BUSINESS_NO" \
    --arg password "$CERT_PASSWORD" \
    --arg signCertDerBase64 "$(base64 < "$sign_cert" | tr -d '\n')" \
    --arg signPriKeyBase64 "$(base64 < "$sign_pri" | tr -d '\n')" \
    --arg kmCertDerBase64 "$(base64 < "$km_cert" | tr -d '\n')" \
    --arg kmPriKeyBase64 "$(base64 < "$km_pri" | tr -d '\n')" \
    '{
      businessNo: ($businessNo | select(length > 0)),
      password: $password,
      signCertDerBase64: $signCertDerBase64,
      signPriKeyBase64: $signPriKeyBase64,
      kmCertDerBase64: $kmCertDerBase64,
      kmPriKeyBase64: $kmPriKeyBase64
    }')"

  echo "uploading cert bundle for organization ${active_org}"
  curl -fsS -X PUT \
    "http://127.0.0.1:${LOCAL_PORT}/api/v1/organizations/${active_org}/certificate" \
    -H 'content-type: application/json' \
    -H "authorization: Bearer ${auth_token}" \
    -H "x-organization-id: ${active_org}" \
    --data-binary "$payload" | jq

  echo "running optional sync for ${PLANT_FACILITY_CODE}"
  curl -fsS -X POST \
    "http://127.0.0.1:${LOCAL_PORT}/api/v1/organizations/${active_org}/sync" \
    -H 'content-type: application/json' \
    -H "authorization: Bearer ${auth_token}" \
    -H "x-organization-id: ${active_org}" \
    --data-binary "$(jq -n \
      --arg facilityCode "$PLANT_FACILITY_CODE" \
      '{facilityCode: $facilityCode, mode: "full"}')" | jq
}

need kubectl
need curl
need jq
need base64

port_forward_log="$(mktemp)"
cleanup() {
  if [[ -n "${pf_pid:-}" ]]; then
    kill "${pf_pid}" >/dev/null 2>&1 || true
  fi
  rm -f "$port_forward_log"
}
trap cleanup EXIT

kubectl -n "${NAMESPACE}" port-forward svc/certd "${LOCAL_PORT}:80" >"${port_forward_log}" 2>&1 &
pf_pid=$!
wait_for_http "http://127.0.0.1:${LOCAL_PORT}/healthz"

seed_trailbase_user

AUTH_TOKEN="$(retry_capture 20 2 login_trailbase)"
if [[ -z "${AUTH_TOKEN}" || "${AUTH_TOKEN}" == "null" ]]; then
  echo "failed to obtain TrailBase auth token" >&2
  exit 1
fi

BOOTSTRAP_JSON="$(retry_capture 20 2 bootstrap_owner "${AUTH_TOKEN}")"
BOOTSTRAP_ACTIVE_ORG_ID="$(jq -r '.organizationId // .organization_id // empty' <<<"${BOOTSTRAP_JSON}")"
if [[ -z "${BOOTSTRAP_ACTIVE_ORG_ID}" ]]; then
  BOOTSTRAP_ACTIVE_ORG_ID="${BOOTSTRAP_ORGANIZATION_ID}"
fi

ME_JSON="$(retry_capture 20 2 current_user "${AUTH_TOKEN}" "${BOOTSTRAP_ACTIVE_ORG_ID}")"
ACTIVE_ORG_ID="$(jq -r '.activeOrganizationId // .active_organization_id // empty' <<<"${ME_JSON}")"
if [[ -z "${ACTIVE_ORG_ID}" ]]; then
  ACTIVE_ORG_ID="${BOOTSTRAP_ACTIVE_ORG_ID}"
fi
USER_ID="$(jq -r '.user.id // empty' <<<"${ME_JSON}")"
echo "logged in user=${USER_ID:-unknown} active_organization=${ACTIVE_ORG_ID}"
echo "${ME_JSON}" | jq

PLANTS_JSON="$(retry_capture 20 2 list_plants "${AUTH_TOKEN}" "${ACTIVE_ORG_ID}")"
if [[ "$(jq 'length' <<<"${PLANTS_JSON}")" -eq 0 ]]; then
  echo "no plants visible; creating baseline plant"
  CREATED_PLANT_JSON="$(retry_capture 20 2 create_plant "${AUTH_TOKEN}" "${ACTIVE_ORG_ID}")"
  FACILITY_CODE="$(jq -r '.facilityCode // .facility_code // empty' <<<"${CREATED_PLANT_JSON}")"
  if [[ -z "${FACILITY_CODE}" ]]; then
    echo "failed to resolve created plant facility code" >&2
    exit 1
  fi
  echo "${CREATED_PLANT_JSON}" | jq
else
  FACILITY_CODE="$(jq -r '.[0].facilityCode // .[0].facility_code // empty' <<<"${PLANTS_JSON}")"
  if [[ -z "${FACILITY_CODE}" ]]; then
    echo "failed to resolve visible plant facility code" >&2
    exit 1
  fi
  echo "reusing existing visible plant ${FACILITY_CODE}"
fi

INSPECTED_PLANT_JSON="$(retry_capture 20 2 inspect_plant "${AUTH_TOKEN}" "${ACTIVE_ORG_ID}" "${FACILITY_CODE}")"
echo "${INSPECTED_PLANT_JSON}" | jq

maybe_upload_certificate "${AUTH_TOKEN}" "${ACTIVE_ORG_ID}"

echo "E2E completed"
