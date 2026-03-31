#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
NAMESPACE="${NAMESPACE:-certd}"
LOCAL_PORT="${LOCAL_PORT:-18080}"
BUSINESS_NO="${BUSINESS_NO:-2768603291}"
FACILITY_CODE="${FACILITY_CODE:-PV15-R-090795}"
CERTD_STABILIZE_SECONDS="${CERTD_STABILIZE_SECONDS:-45}"
MINIO_STABILIZE_SECONDS="${MINIO_STABILIZE_SECONDS:-10}"

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
cleanup() {
  if [[ -n "${pf_pid}" ]]; then
    kill "${pf_pid}" >/dev/null 2>&1 || true
  fi
}
trap cleanup EXIT

stop_port_forward() {
  if [[ -n "${pf_pid}" ]]; then
    kill "${pf_pid}" >/dev/null 2>&1 || true
    pf_pid=""
  fi
}

start_port_forward() {
  stop_port_forward
  kubectl -n "${NAMESPACE}" port-forward svc/certd "${LOCAL_PORT}:80" >/tmp/certd-port-forward.log 2>&1 &
  pf_pid=$!
  for _ in $(seq 1 90); do
    if curl -fsS "http://127.0.0.1:${LOCAL_PORT}/healthz" >/dev/null 2>&1; then
      return 0
    fi
    sleep 2
  done
  echo "certd service did not become healthy on localhost:${LOCAL_PORT}" >&2
  return 1
}

start_port_forward

echo "baseline keploy replay..."
stop_port_forward
time "${ROOT_DIR}/scripts/keploy-k8s-test.sh"
start_port_forward

CERTD_POD="$(kubectl -n "${NAMESPACE}" get pod -l app=certd -o jsonpath='{.items[0].metadata.name}')"
MINIO_POD="$(kubectl -n "${NAMESPACE}" get pod -l app=minio -o jsonpath='{.items[0].metadata.name}')"

kubectl -n "${NAMESPACE}" delete pod "${CERTD_POD}" --wait=false
kubectl -n "${NAMESPACE}" wait --for=condition=ready pod -l app=certd --timeout=10m
sleep "${CERTD_STABILIZE_SECONDS}"
start_port_forward

echo "delta sync after certd pod restart..."
time curl -sS -X POST \
  "http://127.0.0.1:${LOCAL_PORT}/api/v1/businesses/${BUSINESS_NO}/sync" \
  -H 'content-type: application/json' \
  --data-binary "{\"facilityCode\":\"${FACILITY_CODE}\",\"mode\":\"delta\"}" | jq

echo "keploy replay after certd pod restart..."
stop_port_forward
time "${ROOT_DIR}/scripts/keploy-k8s-test.sh"
start_port_forward

kubectl -n "${NAMESPACE}" delete pod "${MINIO_POD}" --wait=false
kubectl -n "${NAMESPACE}" wait --for=condition=ready pod -l app=minio --timeout=10m
sleep "${MINIO_STABILIZE_SECONDS}"
start_port_forward

DOC_ID="$(curl -sS "http://127.0.0.1:${LOCAL_PORT}/api/v1/plants/${FACILITY_CODE}/documents" | jq -r '.[0].id')"
curl -sSI "http://127.0.0.1:${LOCAL_PORT}/api/v1/documents/${DOC_ID}/download"

echo "keploy replay after minio pod restart..."
stop_port_forward
time "${ROOT_DIR}/scripts/keploy-k8s-test.sh"

echo "chaos test completed"
