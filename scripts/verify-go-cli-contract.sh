#!/bin/sh
# Executable-level stdout and cgo-free build checks (including package init).
set -eu
root=$(CDPATH='' cd -- "$(dirname -- "$0")/.." && pwd)
command -v go >/dev/null
task_dir=$(mktemp -d "${TMPDIR:-/tmp}/grant-cli-contract.XXXXXX")
trap 'rm -rf "$task_dir"' EXIT HUP INT TERM
(cd "$root" && CGO_ENABLED=0 go build -o "$task_dir/" ./cmd/...)
if "$task_dir/grant" invalid-command >"$task_dir/server.out" 2>"$task_dir/server.err"; then
 echo 'server accepted unknown command' >&2; exit 1
fi
[ ! -s "$task_dir/server.out" ] && [ -s "$task_dir/server.err" ]
printf '%s\n' 'Both cgo-free executable stdout contracts passed'
