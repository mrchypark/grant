#!/usr/bin/env bash
set -euo pipefail

ROOT="${1:-$(mktemp -d "${TMPDIR:-/tmp}/certd-empty.XXXXXX")}"
mkdir -p "$ROOT/trailbase-data" "$ROOT/certd-data" "$ROOT/artifacts" "$ROOT/logs"

if command -v openssl >/dev/null 2>&1; then
  INTERNAL_SECRET="${TRAILBASE_CERTD_INTERNAL_SECRET:-$(openssl rand -hex 16)}"
else
  INTERNAL_SECRET="${TRAILBASE_CERTD_INTERNAL_SECRET:-dev-trailbase-internal-secret}"
fi

MASTER_KEY="${CERTD_MASTER_KEY:-dev-only-master-key}"

free_port() {
  python3 - <<'PY'
import socket
s = socket.socket()
s.bind(("127.0.0.1", 0))
print(s.getsockname()[1])
s.close()
PY
}

TRAILBASE_PORT="${TRAILBASE_PORT:-$(free_port)}"
CERTD_PORT="${CERTD_PORT:-$(free_port)}"
HQL_API_PORT="${HQL_API_PORT:-$(free_port)}"
HQL_RAFT_PORT="${HQL_RAFT_PORT:-$(free_port)}"

TRAILBASE_URL="${TRAILBASE_URL:-http://127.0.0.1:${TRAILBASE_PORT}}"
CERTD_URL="${CERTD_URL:-http://127.0.0.1:${CERTD_PORT}}"

cat >"$ROOT/env.sh" <<EOF
export CERTD_DEV_ROOT="$ROOT"
export TRAILBASE_CERTD_INTERNAL_SECRET="$INTERNAL_SECRET"
export TRAILBASE_DATA_DIR="$ROOT/trailbase-data"
export TRAILBASE_ADDRESS="127.0.0.1:${TRAILBASE_PORT}"
export TRAILBASE_URL="$TRAILBASE_URL"

export CERTD_BIND_ADDR="127.0.0.1:${CERTD_PORT}"
export CERTD_URL="$CERTD_URL"
export CERTD_MASTER_KEY="$MASTER_KEY"
export CERTD_BOOTSTRAP_USER_ID="conalog-account"
export CERTD_BOOTSTRAP_USER_NAME="Conalog Account"
export CERTD_TRAILBASE_BASE_URL="$TRAILBASE_URL"
export CERTD_TRAILBASE_INTROSPECT_PATH="/api/certd/introspect"
export CERTD_TRAILBASE_INTERNAL_SECRET="$INTERNAL_SECRET"
export CERTD_TRAILBASE_CACHE_TTL_SECONDS="30"

export ARTIFACTS_BACKEND="fs"
export ARTIFACTS_ROOT_DIR="$ROOT/artifacts"

export CERTD_RUNTIME_MODE="embedded"
export CERTD_LOCAL_NODE_ROLE="voter"
export CERTD_READ_CONSISTENCY="local"

export HQL_NODE_ID="1"
export HQL_NODES=$'1 127.0.0.1:${HQL_RAFT_PORT} 127.0.0.1:${HQL_API_PORT}\n'
export HQL_LISTEN_ADDR_API="127.0.0.1"
export HQL_LISTEN_ADDR_RAFT="127.0.0.1"
export HQL_DATA_DIR="$ROOT/certd-data"
export HQL_SECRET_RAFT="SuperSecureSecret1337"
export HQL_SECRET_API="AnotherSecureSecret42"
export HQL_ENC_KEYS_FROM="env"
export ENC_KEYS=$'bVCyTsGaggVy5yqQ/UzluN29DZW41M3hTSkx6Y3NtZmRuQkR2TnJxUTYzcjQ=\n'
export ENC_KEY_ACTIVE="bVCyTsGaggVy5yqQ"
EOF

wait_for() {
  local url="$1"
  for _ in $(seq 1 60); do
    if curl -fsS "$url" >/dev/null 2>&1; then
      return 0
    fi
    sleep 1
  done
  echo "timed out waiting for $url" >&2
  return 1
}

start_bg() {
  local pid_file="$1"
  local log_file="$2"
  shift 2

  python3 - "$pid_file" "$log_file" "$@" <<'PY'
import subprocess
import sys

pid_file = sys.argv[1]
log_file = sys.argv[2]
cmd = sys.argv[3:]

with open(log_file, "ab", buffering=0) as log:
    proc = subprocess.Popen(
        cmd,
        stdin=subprocess.DEVNULL,
        stdout=log,
        stderr=log,
        start_new_session=True,
        close_fds=True,
    )

with open(pid_file, "w", encoding="utf-8") as fh:
    fh.write(f"{proc.pid}\n")
PY
}

# shellcheck disable=SC1090
source "$ROOT/env.sh"

echo "building trailbase-auth-bridge..."
cargo build --manifest-path crates/trailbase-auth-bridge/Cargo.toml >/dev/null
echo "building certd app..."
cargo build -p app >/dev/null

start_bg \
  "$ROOT/trailbase.pid" \
  "$ROOT/logs/trailbase.log" \
  ./crates/trailbase-auth-bridge/target/debug/trailbase-auth-bridge

start_bg \
  "$ROOT/certd.pid" \
  "$ROOT/logs/certd.log" \
  ./target/debug/app

wait_for "$TRAILBASE_URL/healthz"
wait_for "$CERTD_URL/healthz"

cat <<EOF
started fresh local stack

root: $ROOT
trailbase: $TRAILBASE_URL
certd: $CERTD_URL

next steps:
1. create a TrailBase user:
  ./scripts/dev-empty-seed-user.sh "$ROOT" <email> <password>
2. bind that user as the bootstrap owner:
  ./scripts/dev-empty-bootstrap-owner.sh "$ROOT" <email> <password>
3. login UI:
   $TRAILBASE_URL/_/auth/login
EOF
