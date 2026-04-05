#!/usr/bin/env bash
set -euo pipefail

ROOT="${1:?usage: ./scripts/dev-empty-bootstrap-owner.sh <root> <email> <password>}"
EMAIL="${2:?missing email}"
PASSWORD="${3:?missing password}"

if ! command -v jq >/dev/null 2>&1; then
  echo "jq is required" >&2
  exit 1
fi

# shellcheck disable=SC1090
source "$ROOT/env.sh"

AUTH_TOKEN="$(curl -fsS \
  -X POST \
  "$CERTD_TRAILBASE_BASE_URL/api/auth/v1/login" \
  -H 'content-type: application/json' \
  -d "{\"email\":\"$EMAIL\",\"password\":\"$PASSWORD\"}" | jq -r '.auth_token')"

if [[ -z "$AUTH_TOKEN" || "$AUTH_TOKEN" == "null" ]]; then
  echo "failed to obtain TrailBase auth token" >&2
  exit 1
fi

curl -fsS \
  -X POST \
  "$CERTD_URL/api/v1/admin/bootstrap/trailbase-owner" \
  -H "authorization: Bearer $AUTH_TOKEN" \
  -H "x-certd-master-key: $CERTD_MASTER_KEY"

echo
echo "bootstrapped TrailBase user $EMAIL as certd owner"
