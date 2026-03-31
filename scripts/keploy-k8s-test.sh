#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
KEPLOY_DIR="${KEPLOY_DIR:-$ROOT_DIR/.keploy}"
KEPLOY_BIN="${KEPLOY_BIN:-$HOME/.keploy/bin/keploy}"
NAMESPACE="${NAMESPACE:-certd}"
LOCAL_PORT="${LOCAL_PORT:-18080}"
TEST_SETS="${TEST_SETS:-test-set-0}"
NETWORK_NAME="${NETWORK_NAME:-keploy-network}"
NOOP_CONTAINER_NAME="${NOOP_CONTAINER_NAME:-keploy-noop}"

need() {
  command -v "$1" >/dev/null 2>&1 || {
    echo "missing command: $1" >&2
    exit 1
  }
}

need kubectl
need curl
need "$KEPLOY_BIN"
need docker

pf_pid=""
cleanup() {
  if [[ -n "${pf_pid}" ]]; then
    kill "${pf_pid}" >/dev/null 2>&1 || true
  fi
  docker rm -f keploy-v3 "${NOOP_CONTAINER_NAME}" >/dev/null 2>&1 || true
}
trap cleanup EXIT

docker network create "${NETWORK_NAME}" >/dev/null 2>&1 || true
kubectl -n "${NAMESPACE}" port-forward svc/certd "${LOCAL_PORT}:80" >/tmp/certd-port-forward.log 2>&1 &
pf_pid=$!

for _ in $(seq 1 60); do
  if curl -fsS "http://127.0.0.1:${LOCAL_PORT}/healthz" >/dev/null 2>&1; then
    break
  fi
  sleep 2
done

curl -fsS "http://127.0.0.1:${LOCAL_PORT}/healthz" >/dev/null

"$KEPLOY_BIN" \
  test \
  --config-path "$KEPLOY_DIR" \
  -p "$KEPLOY_DIR" \
  --cmd-type docker \
  -c "docker run --name ${NOOP_CONTAINER_NAME} --network ${NETWORK_NAME} alpine:3.20 sleep 60" \
  --container-name "${NOOP_CONTAINER_NAME}" \
  -n "${NETWORK_NAME}" \
  --host 127.0.0.1 \
  --port "${LOCAL_PORT}" \
  --api-timeout 120 \
  --test-sets "${TEST_SETS}" \
  --disable-ansi
