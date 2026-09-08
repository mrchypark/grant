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
printf '%s' '{"manifestVersion":"grant.patch-data-authority.v1","generation":"1","ledgerDigest":"x","users":[],"organizations":[],"memberships":[],"hierarchyEdges":[],"plantBindings":[],"applicationAssignments":[]}' \
 | "$task_dir/grant-patch-data-manifest-digest" >"$task_dir/digest.out" 2>"$task_dir/digest.err"
[ ! -s "$task_dir/digest.err" ]
[ "$(wc -l < "$task_dir/digest.out" | tr -d ' ')" = 1 ]
[ "$(wc -c < "$task_dir/digest.out" | tr -d ' ')" = 65 ]
case "$(cat "$task_dir/digest.out")" in *[!a-f0-9]*) echo 'digest stdout contamination' >&2; exit 1 ;; esac
printf '%s\n' 'Both cgo-free executable stdout contracts passed'
