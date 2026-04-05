#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
KEPLOY_DIR="${KEPLOY_DIR:-$ROOT_DIR/.keploy}"
RUNTIME_DIR="$KEPLOY_DIR/runtime"
ENV_FILE="$RUNTIME_DIR/certd.env"
IMAGE_NAME="${IMAGE_NAME:-certd:local}"
KEPLOY_BIN="${KEPLOY_BIN:-$HOME/.keploy/bin/keploy}"
CONTAINER_NAME="${CONTAINER_NAME:-certd-keploy}"
NETWORK_NAME="${NETWORK_NAME:-keploy-network}"
PORT="${PORT:-18080}"

need() {
  command -v "$1" >/dev/null 2>&1 || {
    echo "missing command: $1" >&2
    exit 1
  }
}

need docker
need "$KEPLOY_BIN"

if [ -z "${DOCKER_HOST:-}" ]; then
  DOCKER_HOST="$(docker context inspect "$(docker context show)" --format '{{.Endpoints.docker.Host}}')"
  export DOCKER_HOST
fi

mkdir -p "$RUNTIME_DIR/data" "$RUNTIME_DIR/artifacts" "$KEPLOY_DIR"
mkdir -p "$KEPLOY_DIR/keploy"
rm -rf "$KEPLOY_DIR"/test-set-* "$KEPLOY_DIR"/reports "$KEPLOY_DIR"/keploy/test-set-* "$KEPLOY_DIR"/keploy/reports

docker image inspect "$IMAGE_NAME" >/dev/null 2>&1 || {
  echo "missing image $IMAGE_NAME" >&2
  echo "run: docker build -t $IMAGE_NAME ." >&2
  exit 1
}

cat > "$ENV_FILE" <<EOF
CERTD_BIND_ADDR=0.0.0.0:${PORT}
CERTD_MASTER_KEY=${CERTD_MASTER_KEY:-local-dev-master-key}
CERTD_BOOTSTRAP_USER_ID=${CERTD_BOOTSTRAP_USER_ID:-conalog-account}
CERTD_BOOTSTRAP_USER_NAME=${CERTD_BOOTSTRAP_USER_NAME:-Conalog Account}
ARTIFACTS_BACKEND=fs
ARTIFACTS_ROOT_DIR=/var/lib/certd/artifacts
HQL_NODE_ID=1
HQL_NODES=1 127.0.0.1:18100 127.0.0.1:18200
HQL_LISTEN_ADDR_API=0.0.0.0
HQL_LISTEN_ADDR_RAFT=0.0.0.0
HQL_DATA_DIR=/var/lib/certd/data
HQL_SECRET_RAFT=${HQL_SECRET_RAFT:-SuperSecureSecret1337}
HQL_SECRET_API=${HQL_SECRET_API:-AnotherSecureSecret42}
HQL_ENC_KEYS_FROM=env
ENC_KEYS=${ENC_KEYS:-bVCyTsGaggVy5yqQ/UzluN29DZW41M3hTSkx6Y3NtZmRuQkR2TnJxUTYzcjQ=}
ENC_KEY_ACTIVE=${ENC_KEY_ACTIVE:-bVCyTsGaggVy5yqQ}
EOF

docker rm -f "$CONTAINER_NAME" keploy-v3 >/dev/null 2>&1 || true
docker ps -aq --filter 'name=^keploy-v3' | xargs -r docker rm -f >/dev/null 2>&1 || true
docker network create "$NETWORK_NAME" >/dev/null 2>&1 || true

APP_CMD="docker run --name $CONTAINER_NAME --network $NETWORK_NAME -p ${PORT}:${PORT} --env-file $ENV_FILE -v $RUNTIME_DIR:/var/lib/certd $IMAGE_NAME"

"$KEPLOY_BIN" \
  record \
  --config-path "$KEPLOY_DIR" \
  -p "$KEPLOY_DIR" \
  --cmd-type docker \
  -c "$APP_CMD" \
  --container-name "$CONTAINER_NAME" \
  -n "$NETWORK_NAME" \
  --pass-through-ports 443 \
  --build-delay 30 \
  --record-timer 120s \
  --disable-ansi &

KEPLOY_PID=$!
trap 'docker rm -f "$CONTAINER_NAME" keploy-v3 >/dev/null 2>&1 || true; docker ps -aq --filter '"'"'name=^keploy-v3'"'"' | xargs -r docker rm -f >/dev/null 2>&1 || true' EXIT

"$ROOT_DIR/scripts/keploy-scenario.sh"

kill -INT "$KEPLOY_PID" >/dev/null 2>&1 || true
wait "$KEPLOY_PID"
echo "keploy record completed"
