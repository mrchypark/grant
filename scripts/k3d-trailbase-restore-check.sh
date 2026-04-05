#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
NAMESPACE="${NAMESPACE:-certd}"
LOCAL_PORT="${LOCAL_PORT:-18080}"
TRAILBASE_USER_EMAIL="${TRAILBASE_USER_EMAIL:-trailbase-user@example.com}"
TRAILBASE_USER_PASSWORD="${TRAILBASE_USER_PASSWORD:-secret}"
BOOTSTRAP_ORGANIZATION_ID="${BOOTSTRAP_ORGANIZATION_ID:-conalog-account}"
TRAILBASE_SELECTOR="${TRAILBASE_SELECTOR:-app=trailbase}"
TRAILBASE_CONTAINER="${TRAILBASE_CONTAINER:-trailbase}"
TRAILBASE_DATA_DIR="${TRAILBASE_DATA_DIR:-/app/traildepot}"
CERTD_MASTER_KEY="${CERTD_MASTER_KEY:?set CERTD_MASTER_KEY}"
TRAILBASE_REPLICATION_SETTLE_SECONDS="${TRAILBASE_REPLICATION_SETTLE_SECONDS:-15}"

need() {
  command -v "$1" >/dev/null 2>&1 || {
    echo "missing command: $1" >&2
    exit 1
  }
}

need kubectl
need curl
need jq

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

start_port_forward() {
  if [[ -n "${pf_pid}" ]]; then
    kill "${pf_pid}" >/dev/null 2>&1 || true
    pf_pid=""
  fi
  kubectl -n "${NAMESPACE}" port-forward svc/certd "${LOCAL_PORT}:80" >"${port_forward_log}" 2>&1 &
  pf_pid=$!
  wait_for_http "http://127.0.0.1:${LOCAL_PORT}/healthz"
}

seed_trailbase_user() {
  TRAILBASE_USER_EMAIL="${TRAILBASE_USER_EMAIL}" \
  TRAILBASE_USER_PASSWORD="${TRAILBASE_USER_PASSWORD}" \
  TRAILBASE_SELECTOR="${TRAILBASE_SELECTOR}" \
  TRAILBASE_CONTAINER="${TRAILBASE_CONTAINER}" \
  TRAILBASE_DATA_DIR="${TRAILBASE_DATA_DIR}" \
  NAMESPACE="${NAMESPACE}" \
    bash "${ROOT_DIR}/scripts/k3d-seed-trailbase-user.sh"
}

login_trailbase() {
  curl -fsS -X POST \
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

trailbase_pod() {
  kubectl -n "${NAMESPACE}" get pods -l "${TRAILBASE_SELECTOR}" -o json \
    | jq -r '
        .items
        | map(select(.status.phase == "Running" and (.metadata.deletionTimestamp | not)))
        | sort_by(.metadata.creationTimestamp)
        | last
        | .metadata.name // empty
      '
}

wait_for_trailbase_ready() {
  kubectl -n "${NAMESPACE}" wait --for=condition=Ready pod -l "${TRAILBASE_SELECTOR}" --timeout=180s >/dev/null
}

main() {
  local auth_token me_before_json active_org_before user_id_before trailbase_before trailbase_after auth_token_after me_after_json active_org_after user_id_after

  wait_for_trailbase_ready
  seed_trailbase_user
  start_port_forward

  auth_token="$(login_trailbase)"
  if [[ -z "${auth_token}" || "${auth_token}" == "null" ]]; then
    echo "failed to obtain TrailBase auth token" >&2
    exit 1
  fi

  local bootstrap_json
  bootstrap_json="$(bootstrap_owner "${auth_token}")"
  local bootstrap_active_org
  bootstrap_active_org="$(jq -r '.organizationId // .organization_id // empty' <<<"${bootstrap_json}")"
  if [[ -z "${bootstrap_active_org}" ]]; then
    bootstrap_active_org="${BOOTSTRAP_ORGANIZATION_ID}"
  fi

  me_before_json="$(current_user "${auth_token}" "${bootstrap_active_org}")"
  active_org_before="$(jq -r '.activeOrganizationId // .active_organization_id // empty' <<<"${me_before_json}")"
  if [[ -z "${active_org_before}" ]]; then
    active_org_before="${bootstrap_active_org}"
  fi
  user_id_before="$(jq -r '.user.id // empty' <<<"${me_before_json}")"
  echo "before restore: user=${user_id_before:-unknown} active_organization=${active_org_before}"
  echo "${me_before_json}" | jq

  if (( TRAILBASE_REPLICATION_SETTLE_SECONDS > 0 )); then
    echo "waiting ${TRAILBASE_REPLICATION_SETTLE_SECONDS}s for TrailBase replication to settle"
    sleep "${TRAILBASE_REPLICATION_SETTLE_SECONDS}"
  fi

  trailbase_before="$(trailbase_pod)"
  if [[ -z "${trailbase_before}" ]]; then
    echo "no running TrailBase pod found" >&2
    exit 1
  fi

  echo "deleting TrailBase pod ${trailbase_before} to force Litestream restore"
  kubectl -n "${NAMESPACE}" delete pod "${trailbase_before}" --wait=false
  kubectl -n "${NAMESPACE}" wait --for=delete pod "${trailbase_before}" --timeout=10m >/dev/null
  kubectl -n "${NAMESPACE}" rollout status deployment/trailbase --timeout=15m
  wait_for_trailbase_ready

  trailbase_after="$(trailbase_pod)"
  if [[ -z "${trailbase_after}" ]]; then
    echo "TrailBase pod did not recover after deletion" >&2
    exit 1
  fi

  kubectl -n "${NAMESPACE}" exec "${trailbase_after}" -c "${TRAILBASE_CONTAINER}" -- \
    test -s "${TRAILBASE_DATA_DIR}/data/main.db"

  auth_token_after="$(login_trailbase)"
  if [[ -z "${auth_token_after}" || "${auth_token_after}" == "null" ]]; then
    echo "failed to login after TrailBase restore" >&2
    exit 1
  fi

  me_after_json="$(current_user "${auth_token_after}" "${active_org_before}")"
  active_org_after="$(jq -r '.activeOrganizationId // .active_organization_id // empty' <<<"${me_after_json}")"
  if [[ -z "${active_org_after}" ]]; then
    active_org_after="${active_org_before}"
  fi
  user_id_after="$(jq -r '.user.id // empty' <<<"${me_after_json}")"

  if [[ "${user_id_before}" != "${user_id_after}" ]]; then
    echo "TrailBase user id changed across restore: before=${user_id_before}, after=${user_id_after}" >&2
    exit 1
  fi
  if [[ "${active_org_before}" != "${active_org_after}" ]]; then
    echo "active organization changed across restore: before=${active_org_before}, after=${active_org_after}" >&2
    exit 1
  fi

  echo "after restore: user=${user_id_after:-unknown} active_organization=${active_org_after}"
  echo "${me_after_json}" | jq

  stop_port_forward
  echo "TrailBase restore check completed"
}

main "$@"
