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

docker image inspect "$IMAGE_NAME" >/dev/null 2>&1 || {
  echo "missing image $IMAGE_NAME" >&2
  echo "run: docker build -t $IMAGE_NAME ." >&2
  exit 1
}

test -f "$ENV_FILE" || {
  echo "missing $ENV_FILE. run scripts/keploy-record.sh first" >&2
  exit 1
}

docker rm -f "$CONTAINER_NAME" keploy-v3 >/dev/null 2>&1 || true
docker ps -aq --filter 'name=^keploy-v3' | xargs -r docker rm -f >/dev/null 2>&1 || true
docker network create "$NETWORK_NAME" >/dev/null 2>&1 || true

APP_CMD="docker run --name $CONTAINER_NAME --network $NETWORK_NAME -p ${PORT}:${PORT} --env-file $ENV_FILE -v $RUNTIME_DIR:/var/lib/certd $IMAGE_NAME"

"$KEPLOY_BIN" \
  test \
  --config-path "$KEPLOY_DIR" \
  -p "$KEPLOY_DIR" \
  --cmd-type docker \
  -c "$APP_CMD" \
  --container-name "$CONTAINER_NAME" \
  -n "$NETWORK_NAME" \
  --pass-through-ports 443 \
  --delay 20 \
  --disable-ansi
