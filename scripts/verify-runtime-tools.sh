#!/bin/sh
set -eu

root=$(CDPATH='' cd -- "$(dirname -- "$0")/.." && pwd)
dockerfile=${1:-"$root/Dockerfile"}

verify() {
  file=$1
  grep -F -- './cmd/grant' "$file" >/dev/null &&
  grep -F -- '/workspace/grant /usr/local/bin/grant' "$file" >/dev/null &&
  grep -F -- 'ln -s /usr/local/bin/grant /usr/local/bin/grant-admin' "$file" >/dev/null &&
  grep -F -- '/usr/local/bin/replica-members' "$file" >/dev/null &&
  grep -F -- 'ENTRYPOINT ["/usr/local/bin/grant"]' "$file" >/dev/null
}

verify "$dockerfile"

tmp=$(mktemp -d "${TMPDIR:-/tmp}/grant-runtime-tools.XXXXXX")
trap 'rm -rf "$tmp"' EXIT HUP INT TERM

sed '/replica-members/d' "$dockerfile" >"$tmp/missing-replica-members.Dockerfile"
if verify "$tmp/missing-replica-members.Dockerfile" 2>/dev/null; then
  echo 'runtime tool verifier accepted a missing replica members tool' >&2
  exit 1
fi

sed 's|ENTRYPOINT \["/usr/local/bin/grant"\]|ENTRYPOINT ["/usr/local/bin/grant-admin"]|' \
  "$dockerfile" >"$tmp/changed-entrypoint.Dockerfile"
if verify "$tmp/changed-entrypoint.Dockerfile" 2>/dev/null; then
  echo 'runtime tool verifier accepted a changed server entrypoint' >&2
  exit 1
fi

printf '%s\n' 'Grant runtime tool packaging contract verified'
