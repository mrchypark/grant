#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
NAMESPACE="${NAMESPACE:-certd}"
LOCAL_PORT="${LOCAL_PORT:-18080}"
BUSINESS_NO="${BUSINESS_NO:-2768603291}"
FACILITY_CODE="${FACILITY_CODE:-PV15-R-090795}"
CERT_DIR="${CERT_DIR:-$ROOT_DIR/secrets/cert}"
CERT_PASSWORD="${CERT_PASSWORD:?set CERT_PASSWORD}"

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
cleanup() {
  if [[ -n "${pf_pid}" ]]; then
    kill "${pf_pid}" >/dev/null 2>&1 || true
  fi
}
trap cleanup EXIT

kubectl -n "${NAMESPACE}" port-forward svc/certd "${LOCAL_PORT}:80" >/tmp/certd-port-forward.log 2>&1 &
pf_pid=$!
sleep 3

SIGN_CERT="$(base64 < "${CERT_DIR}/signCert.der" | tr -d '\n')"
SIGN_PRI="$(base64 < "${CERT_DIR}/signPri.key" | tr -d '\n')"
KM_CERT="$(base64 < "${CERT_DIR}/kmCert.der" | tr -d '\n')"
KM_PRI="$(base64 < "${CERT_DIR}/kmPri.key" | tr -d '\n')"

cat > /tmp/certd-cert-upload.json <<EOF
{
  "password": "${CERT_PASSWORD}",
  "signCertDerBase64": "${SIGN_CERT}",
  "signPriKeyBase64": "${SIGN_PRI}",
  "kmCertDerBase64": "${KM_CERT}",
  "kmPriKeyBase64": "${KM_PRI}"
}
EOF

curl -sS -X PUT \
  "http://127.0.0.1:${LOCAL_PORT}/api/v1/businesses/${BUSINESS_NO}/certificate" \
  -H 'content-type: application/json' \
  --data-binary @/tmp/certd-cert-upload.json | jq

echo "running full sync..."
time curl -sS -X POST \
  "http://127.0.0.1:${LOCAL_PORT}/api/v1/businesses/${BUSINESS_NO}/sync" \
  -H 'content-type: application/json' \
  --data-binary "{\"facilityCode\":\"${FACILITY_CODE}\",\"mode\":\"full\"}" | jq

echo "running delta sync..."
time curl -sS -X POST \
  "http://127.0.0.1:${LOCAL_PORT}/api/v1/businesses/${BUSINESS_NO}/sync" \
  -H 'content-type: application/json' \
  --data-binary "{\"facilityCode\":\"${FACILITY_CODE}\",\"mode\":\"delta\"}" | jq

curl -sS "http://127.0.0.1:${LOCAL_PORT}/api/v1/plants/${FACILITY_CODE}" | jq
curl -sS "http://127.0.0.1:${LOCAL_PORT}/api/v1/plants/${FACILITY_CODE}/generation" | jq 'length'
curl -sS "http://127.0.0.1:${LOCAL_PORT}/api/v1/plants/${FACILITY_CODE}/rates" | jq 'length'
curl -sS "http://127.0.0.1:${LOCAL_PORT}/api/v1/plants/${FACILITY_CODE}/rec" | jq 'length'

DOC_ID="$(curl -sS "http://127.0.0.1:${LOCAL_PORT}/api/v1/plants/${FACILITY_CODE}/documents" | jq -r '.[0].id')"
curl -sSI "http://127.0.0.1:${LOCAL_PORT}/api/v1/documents/${DOC_ID}/download"

echo "E2E completed"
