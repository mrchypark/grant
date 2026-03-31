#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
BASE_URL="${BASE_URL:-http://127.0.0.1:18080}"
BUSINESS_NO="${BUSINESS_NO:-2768603291}"
FACILITY_CODE="${FACILITY_CODE:-PV15-R-090795}"
CERT_PASSWORD="${CERT_PASSWORD:?CERT_PASSWORD is required}"
CERT_DIR="${CERT_DIR:-$ROOT_DIR/secrets/cert}"

need() {
  command -v "$1" >/dev/null 2>&1 || {
    echo "missing command: $1" >&2
    exit 1
  }
}

need curl
need jq
need base64

wait_for_ready() {
  for _ in $(seq 1 120); do
    if curl -fsS "$BASE_URL/healthz" >/dev/null 2>&1; then
      return 0
    fi
    sleep 1
  done
  echo "certd did not become ready" >&2
  exit 1
}

encode() {
  base64 < "$1" | tr -d '\n'
}

wait_for_ready

SIGN_CERT="$(encode "$CERT_DIR/signCert.der")"
SIGN_PRI="$(encode "$CERT_DIR/signPri.key")"
KM_CERT="$(encode "$CERT_DIR/kmCert.der")"
KM_PRI="$(encode "$CERT_DIR/kmPri.key")"

CERT_PAYLOAD="$(mktemp)"
cat > "$CERT_PAYLOAD" <<JSON
{
  "password": "$CERT_PASSWORD",
  "signCertDerBase64": "$SIGN_CERT",
  "signPriKeyBase64": "$SIGN_PRI",
  "kmCertDerBase64": "$KM_CERT",
  "kmPriKeyBase64": "$KM_PRI"
}
JSON

curl -fsS -X PUT \
  "$BASE_URL/api/v1/businesses/$BUSINESS_NO/certificate" \
  -H 'content-type: application/json' \
  --data-binary "@$CERT_PAYLOAD" | jq -e '.businessNo == "'"$BUSINESS_NO"'"' >/dev/null

curl -fsS -X POST \
  "$BASE_URL/api/v1/businesses/$BUSINESS_NO/sync" \
  -H 'content-type: application/json' \
  --data-binary "{\"facilityCode\":\"$FACILITY_CODE\",\"mode\":\"full\"}" | jq -e '.status == "completed"' >/dev/null

curl -fsS "$BASE_URL/api/v1/plants/$FACILITY_CODE" | jq -e '.generation.count >= 1 and .documents.count >= 1' >/dev/null
curl -fsS "$BASE_URL/api/v1/plants/$FACILITY_CODE/generation" | jq -e 'length >= 1' >/dev/null
curl -fsS "$BASE_URL/api/v1/plants/$FACILITY_CODE/rates" | jq -e 'length >= 1' >/dev/null
curl -fsS "$BASE_URL/api/v1/plants/$FACILITY_CODE/rec" | jq -e 'length >= 1' >/dev/null
curl -fsS "$BASE_URL/api/v1/plants/$FACILITY_CODE/documents" | jq -e 'length >= 1' >/dev/null

rm -f "$CERT_PAYLOAD"
echo "keploy scenario completed"
