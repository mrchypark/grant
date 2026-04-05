#!/usr/bin/env bash
set -euo pipefail

ROOT="${1:?usage: ./scripts/dev-empty-seed-user.sh <root> <email> <password>}"
EMAIL="${2:?missing email}"
PASSWORD="${3:?missing password}"

# shellcheck disable=SC1090
source "$ROOT/env.sh"

./crates/trailbase-auth-bridge/target/debug/trailbase-auth-bridge \
  seed-user \
  --data-dir "$TRAILBASE_DATA_DIR" \
  --email "$EMAIL" \
  --password "$PASSWORD"

echo "seeded TrailBase user $EMAIL"
