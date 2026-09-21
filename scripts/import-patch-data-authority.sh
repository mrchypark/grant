#!/bin/sh
# Deployment-only adapter for the legacy PatchData authority manifest.
set -eu
umask 077
if [ "$#" -ne 1 ] || [ ! -f "$1" ]; then
  echo 'Usage: GRANT_MASTER_KEY_FILE=/private/key GRANT_URL=http://127.0.0.1:8080 scripts/import-patch-data-authority.sh payload.json' >&2
  exit 2
fi
: "${GRANT_MASTER_KEY_FILE:?set GRANT_MASTER_KEY_FILE to a private credential file}"
root_url=${GRANT_URL:-http://127.0.0.1:8080}
headers=$(mktemp)
trap 'rm -f "$headers"' EXIT HUP INT TERM
printf 'Content-Type: application/json\nX-Grant-Master-Key: %s\n' "$(cat "$GRANT_MASTER_KEY_FILE")" > "$headers"
curl --fail-with-body --silent --show-error --request POST \
  --header "@$headers" --data-binary "@$1" \
  "${root_url%/}/ops/migrations/patch-data-authority"
