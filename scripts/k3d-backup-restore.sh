#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
NAMESPACE="${NAMESPACE:-certd}"
LOCAL_PORT="${LOCAL_PORT:-18080}"
TRAILBASE_USER_EMAIL="${TRAILBASE_USER_EMAIL:-trailbase-user@example.com}"
TRAILBASE_USER_PASSWORD="${TRAILBASE_USER_PASSWORD:-secret}"
BOOTSTRAP_ORGANIZATION_ID="${BOOTSTRAP_ORGANIZATION_ID:-conalog-account}"
PLANT_FACILITY_CODE="${PLANT_FACILITY_CODE:-${FACILITY_CODE:-TB-CHAOS-001}}"
PLANT_NAME="${PLANT_NAME:-TrailBase Restore Baseline Plant}"
PLANT_CONTRACT_NO="${PLANT_CONTRACT_NO:-TB-RESTORE-CONTRACT-001}"
PLANT_ADDRESS="${PLANT_ADDRESS:-Seoul}"
PLANT_INSTALLED_KW="${PLANT_INSTALLED_KW:-50.0}"
PLANT_REC_START_DATE="${PLANT_REC_START_DATE:-2026-01-01}"
PLANT_CONFIRM_ISSUED="${PLANT_CONFIRM_ISSUED:-Y}"
CERT_DIR="${CERT_DIR:-$ROOT_DIR/secrets/cert}"
CERT_PASSWORD="${CERT_PASSWORD:-}"
CERT_BUSINESS_NO="${CERT_BUSINESS_NO:-}"
CERTD_STABILIZE_SECONDS="${CERTD_STABILIZE_SECONDS:-60}"
CERTD_MASTER_KEY="${CERTD_MASTER_KEY:-local-dev-master-key}"

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

ensure_trailbase_online() {
  local replicas
  replicas="$(kubectl -n "${NAMESPACE}" get deployment trailbase -o jsonpath='{.spec.replicas}' 2>/dev/null || echo "")"
  if [[ "${replicas}" != "1" ]]; then
    kubectl -n "${NAMESPACE}" scale deployment/trailbase --replicas=1 >/dev/null
  fi
  kubectl -n "${NAMESPACE}" rollout status deployment/trailbase --timeout=15m >/dev/null
}

start_port_forward() {
  stop_port_forward
  kubectl -n "${NAMESPACE}" port-forward svc/certd "${LOCAL_PORT}:80" >"${port_forward_log}" 2>&1 &
  pf_pid=$!
  wait_for_http "http://127.0.0.1:${LOCAL_PORT}/healthz"
}

seed_trailbase_user() {
  TRAILBASE_USER_EMAIL="${TRAILBASE_USER_EMAIL}" \
  TRAILBASE_USER_PASSWORD="${TRAILBASE_USER_PASSWORD}" \
  TRAILBASE_SELECTOR="${TRAILBASE_SELECTOR:-app=trailbase}" \
  TRAILBASE_CONTAINER="${TRAILBASE_CONTAINER:-trailbase}" \
  TRAILBASE_DATA_DIR="${TRAILBASE_DATA_DIR:-/app/traildepot}" \
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
  curl -fsS \
    "http://127.0.0.1:${LOCAL_PORT}/api/v1/plants/${facility_code}" \
    -H "authorization: Bearer ${auth_token}" \
    -H "x-organization-id: ${active_org}"
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

  echo "no baseline plant visible; creating ${PLANT_FACILITY_CODE}" >&2
  create_plant "${auth_token}" "${active_org}"
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

  echo "uploading cert bundle for organization ${active_org}"
  curl -fsS -X PUT \
    "http://127.0.0.1:${LOCAL_PORT}/api/v1/organizations/${active_org}/certificate" \
    -H 'content-type: application/json' \
    -H "authorization: Bearer ${auth_token}" \
    -H "x-organization-id: ${active_org}" \
    --data-binary "${payload}" | jq

  echo "running optional sync for ${facility_code}"
  curl -fsS -X POST \
    "http://127.0.0.1:${LOCAL_PORT}/api/v1/organizations/${active_org}/sync" \
    -H 'content-type: application/json' \
    -H "authorization: Bearer ${auth_token}" \
    -H "x-organization-id: ${active_org}" \
    --data-binary "$(jq -n \
      --arg facilityCode "${facility_code}" \
      '{facilityCode: $facilityCode, mode: "full"}')" | jq
}

trigger_backup() {
  curl -fsS -X POST \
    "http://127.0.0.1:${LOCAL_PORT}/api/v1/admin/backups" \
    -H "x-certd-master-key: ${CERTD_MASTER_KEY}" \
    >/dev/null
}

list_backups_s3() {
  curl -fsS \
    "http://127.0.0.1:${LOCAL_PORT}/api/v1/admin/backups/s3" \
    -H "x-certd-master-key: ${CERTD_MASTER_KEY}"
}

latest_s3_backup_name() {
  list_backups_s3 \
    | jq -r '
        .items
        | sort_by(.lastModified // 0)
        | last
        | .name // empty
      '
}

wait_for_backup_cooldown() {
  local latest_last_modified now wait_seconds
  latest_last_modified="$(
    list_backups_s3 \
      | jq -r '
          .items
          | sort_by(.lastModified // 0)
          | last
          | .lastModified // 0
        '
  )"
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

wait_for_new_s3_backup_name() {
  local previous_name="$1"
  local backup_name=""
  for _ in $(seq 1 60); do
    backup_name="$(
      latest_s3_backup_name \
        | jq -Rr --arg previous_name "${previous_name}" '
          if . == $previous_name then
            empty
          else
            .
          end
        '
    )"
    if [[ -n "${backup_name}" ]]; then
      echo "${backup_name}"
      return 0
    fi
    sleep 2
  done
  return 1
}

wait_for_s3_backup_named() {
  local backup_name="$1"
  for _ in $(seq 1 60); do
    if list_backups_s3 \
      | jq -e --arg backup_name "${backup_name}" '
          .items
          | any(.name == $backup_name)
        ' >/dev/null; then
      echo "${backup_name}"
      return 0
    fi
    sleep 2
  done
  return 1
}

ensure_backup_bucket() {
  kubectl -n "${NAMESPACE}" delete job minio-bootstrap-backups --ignore-not-found >/dev/null 2>&1 || true
  cat <<EOF | kubectl apply -f - >/dev/null
apiVersion: batch/v1
kind: Job
metadata:
  name: minio-bootstrap-backups
  namespace: ${NAMESPACE}
spec:
  backoffLimit: 6
  template:
    spec:
      restartPolicy: OnFailure
      containers:
        - name: mc
          image: minio/mc:latest
          env:
            - name: MINIO_ROOT_USER
              valueFrom:
                secretKeyRef:
                  name: minio-secrets
                  key: MINIO_ROOT_USER
            - name: MINIO_ROOT_PASSWORD
              valueFrom:
                secretKeyRef:
                  name: minio-secrets
                  key: MINIO_ROOT_PASSWORD
          command: ["/bin/sh", "-lc"]
          args:
            - |
              set -e
              until mc alias set local http://minio.${NAMESPACE}.svc.cluster.local:9000 "\$MINIO_ROOT_USER" "\$MINIO_ROOT_PASSWORD"; do
                sleep 2
              done
              mc mb -p local/cert-backups || true
              mc mb -p local/trailbase-backups || true
EOF
  kubectl -n "${NAMESPACE}" wait --for=condition=complete job/minio-bootstrap-backups --timeout=10m >/dev/null
}

ensure_trailbase_online
ensure_backup_bucket
start_port_forward

seed_trailbase_user

AUTH_TOKEN="$(login_trailbase)"
if [[ -z "${AUTH_TOKEN}" || "${AUTH_TOKEN}" == "null" ]]; then
  echo "failed to obtain TrailBase auth token" >&2
  exit 1
fi

BOOTSTRAP_JSON="$(bootstrap_owner "${AUTH_TOKEN}")"
BOOTSTRAP_ACTIVE_ORG_ID="$(jq -r '.organizationId // .organization_id // empty' <<<"${BOOTSTRAP_JSON}")"
if [[ -z "${BOOTSTRAP_ACTIVE_ORG_ID}" ]]; then
  BOOTSTRAP_ACTIVE_ORG_ID="${BOOTSTRAP_ORGANIZATION_ID}"
fi

ME_BEFORE_JSON="$(current_user "${AUTH_TOKEN}" "${BOOTSTRAP_ACTIVE_ORG_ID}")"
ACTIVE_ORGANIZATION_ID="$(jq -r '.activeOrganizationId // .active_organization_id // empty' <<<"${ME_BEFORE_JSON}")"
if [[ -z "${ACTIVE_ORGANIZATION_ID}" ]]; then
  ACTIVE_ORGANIZATION_ID="${BOOTSTRAP_ACTIVE_ORG_ID}"
fi
USER_ID_BEFORE="$(jq -r '.user.id // empty' <<<"${ME_BEFORE_JSON}")"
echo "bootstrapped TrailBase user=${USER_ID_BEFORE:-unknown} active_organization=${ACTIVE_ORGANIZATION_ID}"
echo "${ME_BEFORE_JSON}" | jq

ORIGINAL_REPLICAS="$(
  kubectl -n "${NAMESPACE}" get statefulset certd-voter -o jsonpath='{.spec.replicas}'
)"
if [[ -z "${ORIGINAL_REPLICAS}" ]]; then
  echo "failed to determine current certd-voter replica count" >&2
  exit 1
fi

BASELINE_PLANT_JSON="$(resolve_baseline_plant "${AUTH_TOKEN}" "${ACTIVE_ORGANIZATION_ID}")"
BASELINE_FACILITY_CODE="$(jq -r '.facilityCode // .facility_code // empty' <<<"${BASELINE_PLANT_JSON}")"
if [[ -z "${BASELINE_FACILITY_CODE}" ]]; then
  echo "failed to resolve baseline plant facility code" >&2
  exit 1
fi
echo "${BASELINE_PLANT_JSON}" | jq
inspect_plant "${AUTH_TOKEN}" "${ACTIVE_ORGANIZATION_ID}" "${BASELINE_FACILITY_CODE}" | jq
maybe_upload_certificate_and_sync "${AUTH_TOKEN}" "${ACTIVE_ORGANIZATION_ID}" "${BASELINE_FACILITY_CODE}"

wait_for_backup_cooldown
echo "triggering backup"
PREVIOUS_BACKUP_NAME="$(latest_s3_backup_name)"
trigger_backup

BACKUP_NAME="$(wait_for_new_s3_backup_name "${PREVIOUS_BACKUP_NAME}" || true)"
if [[ -z "${BACKUP_NAME}" ]]; then
  echo "new S3 backup object was not created" >&2
  exit 1
fi

stop_port_forward

kubectl -n "${NAMESPACE}" scale deployment/certd-learner-1 --replicas=0
kubectl -n "${NAMESPACE}" scale deployment/certd-learner-2 --replicas=0
kubectl -n "${NAMESPACE}" scale deployment/certd-learner-3 --replicas=0
kubectl -n "${NAMESPACE}" scale statefulset/certd-voter --replicas=0
kubectl -n "${NAMESPACE}" wait --for=delete pod -l app=certd --timeout=10m

kubectl -n "${NAMESPACE}" set env statefulset/certd-voter HQL_BACKUP_RESTORE="s3:${BACKUP_NAME}"
# Hiqlite restore expects the cluster nodes to come back together.
# Booting only voter-0 leaves the restored node stuck before it can
# re-establish raft membership and open the HTTP listener.
kubectl -n "${NAMESPACE}" scale statefulset/certd-voter --replicas="${ORIGINAL_REPLICAS}"
kubectl -n "${NAMESPACE}" rollout status statefulset/certd-voter --timeout=15m
kubectl -n "${NAMESPACE}" scale deployment/certd-learner-1 --replicas=1
kubectl -n "${NAMESPACE}" scale deployment/certd-learner-2 --replicas=1
kubectl -n "${NAMESPACE}" scale deployment/certd-learner-3 --replicas=1
kubectl -n "${NAMESPACE}" rollout status deployment/certd-learner-1 --timeout=15m
kubectl -n "${NAMESPACE}" rollout status deployment/certd-learner-2 --timeout=15m
kubectl -n "${NAMESPACE}" rollout status deployment/certd-learner-3 --timeout=15m

sleep "${CERTD_STABILIZE_SECONDS}"
start_port_forward

AUTH_TOKEN_AFTER="$(login_trailbase)"
if [[ -z "${AUTH_TOKEN_AFTER}" || "${AUTH_TOKEN_AFTER}" == "null" ]]; then
  echo "failed to re-login after restore" >&2
  exit 1
fi

# Re-bind the TrailBase owner after certd DR restore. The restored certd
# state can come back before the external identity mapping has been
# reconstructed for the current TrailBase subject.
bootstrap_owner "${AUTH_TOKEN_AFTER}" >/dev/null

ME_AFTER_JSON="$(current_user "${AUTH_TOKEN_AFTER}" "${ACTIVE_ORGANIZATION_ID}")"
USER_ID_AFTER="$(jq -r '.user.id // empty' <<<"${ME_AFTER_JSON}")"
ACTIVE_ORGANIZATION_ID_AFTER="$(jq -r '.activeOrganizationId // .active_organization_id // empty' <<<"${ME_AFTER_JSON}")"
if [[ -z "${ACTIVE_ORGANIZATION_ID_AFTER}" ]]; then
  ACTIVE_ORGANIZATION_ID_AFTER="${ACTIVE_ORGANIZATION_ID}"
fi
if [[ "${USER_ID_AFTER}" != "${USER_ID_BEFORE}" ]]; then
  echo "TrailBase user changed across restore: before=${USER_ID_BEFORE}, after=${USER_ID_AFTER}" >&2
  exit 1
fi
if [[ "${ACTIVE_ORGANIZATION_ID_AFTER}" != "${ACTIVE_ORGANIZATION_ID}" ]]; then
  echo "active organization changed across restore: before=${ACTIVE_ORGANIZATION_ID}, after=${ACTIVE_ORGANIZATION_ID_AFTER}" >&2
  exit 1
fi
echo "${ME_AFTER_JSON}" | jq

PLANTS_AFTER_JSON="$(list_plants "${AUTH_TOKEN_AFTER}" "${ACTIVE_ORGANIZATION_ID_AFTER}")"
if [[ "$(jq 'length' <<<"${PLANTS_AFTER_JSON}")" -eq 0 ]]; then
  echo "no plants visible after restore" >&2
  exit 1
fi
echo "${PLANTS_AFTER_JSON}" | jq

RESTORED_PLANT_JSON="$(inspect_plant "${AUTH_TOKEN_AFTER}" "${ACTIVE_ORGANIZATION_ID_AFTER}" "${BASELINE_FACILITY_CODE}")"
echo "${RESTORED_PLANT_JSON}" | jq

documents_response_file="$(mktemp)"
documents_status_code="$(
  curl -sS -o "${documents_response_file}" -w '%{http_code}' \
    "http://127.0.0.1:${LOCAL_PORT}/api/v1/plants/${BASELINE_FACILITY_CODE}/documents" \
    -H "authorization: Bearer ${AUTH_TOKEN_AFTER}" \
    -H "x-organization-id: ${ACTIVE_ORGANIZATION_ID_AFTER}"
)"

if [[ "${documents_status_code}" == "200" ]]; then
  DOC_ID="$(jq -r '.[0].id // empty' < "${documents_response_file}")"
  if [[ -n "${DOC_ID}" ]]; then
    curl -fsSI \
      "http://127.0.0.1:${LOCAL_PORT}/api/v1/documents/${DOC_ID}/download" \
      -H "authorization: Bearer ${AUTH_TOKEN_AFTER}" \
      -H "x-organization-id: ${ACTIVE_ORGANIZATION_ID_AFTER}" >/dev/null
  else
    echo "no documents visible after restore; skipping download check"
  fi
else
  echo "documents endpoint returned HTTP ${documents_status_code}; skipping document download check"
  cat "${documents_response_file}" >&2 || true
fi
rm -f "${documents_response_file}"

stop_port_forward
echo "running TrailBase restore verification..."
TRAILBASE_USER_EMAIL="${TRAILBASE_USER_EMAIL}" \
TRAILBASE_USER_PASSWORD="${TRAILBASE_USER_PASSWORD}" \
BOOTSTRAP_ORGANIZATION_ID="${BOOTSTRAP_ORGANIZATION_ID}" \
TRAILBASE_SELECTOR="${TRAILBASE_SELECTOR:-app=trailbase}" \
TRAILBASE_CONTAINER="${TRAILBASE_CONTAINER:-trailbase}" \
TRAILBASE_DATA_DIR="${TRAILBASE_DATA_DIR:-/app/traildepot}" \
NAMESPACE="${NAMESPACE}" \
LOCAL_PORT="${LOCAL_PORT}" \
CERTD_MASTER_KEY="${CERTD_MASTER_KEY}" \
  bash "${ROOT_DIR}/scripts/k3d-trailbase-restore-check.sh"

echo "auth-aware restore checks completed"
echo "NOTE: HQL_BACKUP_RESTORE remains set on certd-voter in this disposable harness."
echo "NOTE: certd-voter uses emptyDir, so clearing the env on the StatefulSet would trigger pod recreation and wipe the restored local state."
echo "NOTE: before running chaos on the same disposable environment, rebuild the baseline with scripts/k3d-bootstrap.sh or use scripts/k3d-backup-restore-chaos.sh."
echo "backup/restore completed with ${BACKUP_NAME}"
