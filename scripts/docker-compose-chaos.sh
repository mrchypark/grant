#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
COMPOSE_FILE="${COMPOSE_FILE:-${ROOT_DIR}/docker-compose.empty.yml}"
CERTD_URL="${CERTD_URL:-http://127.0.0.1:18080}"
TRAILBASE_URL="${TRAILBASE_URL:-http://127.0.0.1:4000}"
MINIO_URL="${MINIO_URL:-http://127.0.0.1:9000}"
EMAIL="${EMAIL:-owner@example.com}"
PASSWORD="${PASSWORD:-secret123}"
ORG_ID="${ORG_ID:-conalog-account}"
MASTER_KEY="${MASTER_KEY:-dev-only-master-key}"
FACILITY_CODE="${FACILITY_CODE:-LOCAL-CHAOS-001}"

need() {
  command -v "$1" >/dev/null 2>&1 || {
    echo "missing command: $1" >&2
    exit 1
  }
}

need curl
need jq
need docker-compose

wait_http_ok() {
  local url="$1"
  local retries="${2:-60}"
  for _ in $(seq 1 "${retries}"); do
    if curl -fsS "$url" >/dev/null 2>&1; then
      return 0
    fi
    sleep 2
  done
  echo "timeout waiting for $url" >&2
  return 1
}

seed_user() {
  docker-compose -f "${COMPOSE_FILE}" exec -T trailbase \
    trailbase-auth-bridge seed-user \
    --data-dir /data/trailbase \
    --email "${EMAIL}" \
    --password "${PASSWORD}" || true
}

login() {
  curl -fsS -X POST "${CERTD_URL}/api/auth/v1/login" \
    -H 'content-type: application/json' \
    -d "{\"email\":\"${EMAIL}\",\"password\":\"${PASSWORD}\"}" | jq -r '.auth_token'
}

bootstrap_owner() {
  local token="$1"
  curl -fsS -X POST "${CERTD_URL}/api/v1/admin/bootstrap/trailbase-owner" \
    -H "authorization: Bearer ${token}" \
    -H "x-certd-master-key: ${MASTER_KEY}" >/dev/null
}

ensure_baseline() {
  wait_http_ok "${TRAILBASE_URL}/healthz"
  wait_http_ok "${CERTD_URL}/healthz"
  seed_user
  local token
  token="$(login)"
  bootstrap_owner "${token}"
  curl -fsS "${CERTD_URL}/api/v1/me" \
    -H "authorization: Bearer ${token}" \
    -H "x-organization-id: ${ORG_ID}" >/dev/null
  curl -fsS "${CERTD_URL}/api/v1/plants" \
    -H "authorization: Bearer ${token}" \
    -H "x-organization-id: ${ORG_ID}" >/dev/null
  curl -fsS -X POST "${CERTD_URL}/api/v1/organizations/${ORG_ID}/plants" \
    -H "authorization: Bearer ${token}" \
    -H "x-organization-id: ${ORG_ID}" \
    -H 'content-type: application/json' \
    -d "{\"facilityCode\":\"${FACILITY_CODE}\",\"name\":\"Compose Chaos Plant\",\"contractNo\":\"CHAOS-001\",\"address\":\"Seoul\",\"installedKw\":1.5,\"recStartDate\":\"2026-01-01\",\"confirmIssued\":\"N\"}" >/dev/null || true
  echo "${token}"
}

scenario_certd_restart() {
  local token="$1"
  echo "[chaos] certd restart"
  docker-compose -f "${COMPOSE_FILE}" restart certd
  wait_http_ok "${CERTD_URL}/healthz"
  curl -fsS "${CERTD_URL}/api/v1/me" \
    -H "authorization: Bearer ${token}" \
    -H "x-organization-id: ${ORG_ID}" | jq
  curl -fsS "${CERTD_URL}/api/v1/plants" \
    -H "authorization: Bearer ${token}" \
    -H "x-organization-id: ${ORG_ID}" | jq
}

scenario_trailbase_restart() {
  local token="$1"
  echo "[chaos] trailbase restart"
  docker-compose -f "${COMPOSE_FILE}" restart trailbase
  sleep 2
  curl -sS -o /dev/null -w "auth_login_status=%{http_code}\n" "${CERTD_URL}/auth/login"
  curl -sS -o /dev/null -w "me_status=%{http_code}\n" \
    "${CERTD_URL}/api/v1/me" \
    -H "authorization: Bearer ${token}" \
    -H "x-organization-id: ${ORG_ID}" || true
  sleep 35
  curl -sS -o /dev/null -w "me_status_after_cache=%{http_code}\n" \
    "${CERTD_URL}/api/v1/me" \
    -H "authorization: Bearer ${token}" \
    -H "x-organization-id: ${ORG_ID}" || true
  wait_http_ok "${TRAILBASE_URL}/healthz"
  local new_token
  new_token="$(login)"
  curl -fsS "${CERTD_URL}/api/v1/me" \
    -H "authorization: Bearer ${new_token}" \
    -H "x-organization-id: ${ORG_ID}" | jq
}

scenario_minio_restart() {
  local token="$1"
  echo "[chaos] minio restart"
  docker-compose -f "${COMPOSE_FILE}" restart minio
  wait_http_ok "${MINIO_URL}/minio/health/live"
  curl -fsS "${CERTD_URL}/api/v1/plants" \
    -H "authorization: Bearer ${token}" \
    -H "x-organization-id: ${ORG_ID}" | jq
  curl -sS -o /dev/null -w "backup_create_status=%{http_code}\n" \
    -X POST "${CERTD_URL}/api/v1/admin/backups" \
    -H "authorization: Bearer ${token}" \
    -H "x-organization-id: ${ORG_ID}" \
    -H "x-certd-master-key: ${MASTER_KEY}" || true
}

main() {
  local token
  token="$(ensure_baseline)"
  scenario_certd_restart "${token}"
  scenario_trailbase_restart "${token}"
  scenario_minio_restart "${token}"
  echo "[chaos] complete"
}

main "$@"
