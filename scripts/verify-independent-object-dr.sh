#!/bin/sh
# Disposable, local-only process verification; evidence survives cleanup.
set -eu
umask 077
root=$(CDPATH='' cd -- "$(dirname -- "$0")/.." && pwd)
# shellcheck source=/dev/null
. "$root/scripts/testdata/independent-object-dr/guards.sh"
if [ -n "${DOCKER_HOST:-}" ] && ! independent_unix_endpoint "$DOCKER_HOST"; then
 echo 'UNVERIFIED: ambient DOCKER_HOST must be a local Unix socket' >&2; exit 1
fi
for tool in go docker df awk mktemp; do command -v "$tool" >/dev/null || { echo "UNVERIFIED: missing $tool" >&2; exit 1; }; done
context_name=$(docker context show)
endpoint=$(docker context inspect --format '{{.Endpoints.docker.Host}}')
independent_unix_endpoint "$endpoint" || { echo 'UNVERIFIED: a local Unix Docker socket is required' >&2; exit 1; }
independent_native_metadata "$context_name" "$endpoint" || { echo 'UNVERIFIED: bridge/proxy or unknown Docker context; independent native daemon ownership requires owner review' >&2; exit 1; }
if [ -n "${DOCKER_HOST:-}" ] && [ "$DOCKER_HOST" != "$endpoint" ]; then
 echo 'UNVERIFIED: Docker override does not match the approved native context' >&2; exit 1
fi
space() { independent_space "$root" "${work:-$root}"; }
space || { echo 'UNVERIFIED: less than 4 GiB free' >&2; exit 1; }
evidence=$(mktemp -d "${TMPDIR:-/tmp}/grant-independent-evidence.XXXXXX")
work=$(mktemp -d "${TMPDIR:-/tmp}/grant-independent-runtime.XXXXXX")
mkdir "$work/docker-config" "$work/tmp"
echo "Evidence: $evidence"
child='' watch=''
stop_tree() {
 for p in $(ps -axo pid=,ppid= | awk -v parent="$1" '$2==parent {print $1}'); do stop_tree "$p"; done
 kill -TERM "$1" 2>/dev/null || :
}
cleanup() {
 status=$?
 trap - EXIT INT TERM
 [ -z "$watch" ] || { kill "$watch" 2>/dev/null || :; wait "$watch" 2>/dev/null || :; }
 if [ -n "$child" ]; then
  kill -INT "$child" 2>/dev/null || :
  n=0; while kill -0 "$child" 2>/dev/null && [ "$n" -lt 40 ]; do sleep 1; n=$((n+1)); done
  stop_tree "$child"; wait "$child" 2>/dev/null || :
 fi
 unsafe_process=0
 for receipt in "$evidence"/*/*-process.receipt; do
  [ -f "$receipt" ] || continue
  reap_result=0; independent_reap_receipt "$receipt" "$work" || reap_result=$?
  if [ "$reap_result" -ne 0 ]; then
   echo "FAIL: process fallback result=$reap_result receipt=$receipt" >> "$evidence/process-cleanup.log"
   status=1
   [ "$reap_result" -ne 2 ] || unsafe_process=1
  fi
 done
 # Fallback for a killed/panicking Go test: touch only recorded, label-matched CIDs.
 for receipt in "$evidence"/*/owner-id.txt; do
  [ -f "$receipt" ] || continue
  provider_logs=$(dirname "$receipt"); owner=$(cat "$receipt")
  [ -f "$provider_logs/create-admitted.txt" ] || continue
  [ "$(cat "$provider_logs/create-admitted.txt")" = "$owner" ] || { status=1; continue; }
  docker_local() { env -i PATH="$PATH" DOCKER_CONFIG="$work/docker-config" DOCKER_HOST="$endpoint" docker "$@"; }
  cid=$(independent_discover_container "$owner") || { echo "FAIL: unknown/ambiguous create outcome for $owner" >&2; status=1; continue; }
  if [ -n "$cid" ]; then
   status=1 # Falling back after failed Go cleanup is never an all-PASS run.
   actual=$(docker_local inspect --format '{{index .Config.Labels "grant.independent.owner"}}' "$cid")
   if [ "$actual" != "$owner" ]; then echo "FAIL: cleanup ownership mismatch $cid" >&2; status=1; continue; fi
   actual_name=$(docker_local inspect --format '{{.Name}}' "$cid")
   if [ "$actual_name" != "/$owner" ]; then echo "FAIL: cleanup name mismatch $cid" >&2; status=1; continue; fi
   printf '%s\n' "$cid" > "$provider_logs/container-id.txt"
   docker_local logs "$cid" > "$provider_logs/emulator.log" 2>&1 || :
   docker_local rm -f -v "$cid" >> "$evidence/runner-container-cleanup.log" 2>&1 || status=1
   containers=$(docker_local ps -aq --no-trunc) || { echo 'FAIL: cannot verify container absence' >&2; status=1; continue; }
   if printf '%s\n' "$containers" | awk -v cid="$cid" '$0==cid {found=1} END {exit !found}'; then echo "FAIL: container remains $cid" >&2; status=1; fi
  elif [ -f "$provider_logs/container-id.txt" ]; then
   known_cid=$(cat "$provider_logs/container-id.txt")
   containers=$(docker_local ps -aq --no-trunc) || { status=1; continue; }
   if printf '%s\n' "$containers" | awk -v cid="$known_cid" '$0==cid {found=1} END {exit !found}'; then echo "FAIL: known container no longer has owned identity $known_cid" >&2; status=1; fi
  fi
 done
 # Only these mktemp-owned artifacts are removed; logs remain outside work.
 if [ "$unsafe_process" -eq 1 ]; then echo "FAIL: unverifiable process; preserving $work" >&2; exit 1; fi
 find "$work" -depth -delete
 [ ! -e "$work" ] || { echo 'FAIL cleanup: runtime directory remains' >&2; exit 1; }
 echo 'owned runtime directory and build artifacts removed' > "$evidence/runner-cleanup.txt"
 for provider in s3 gcs azure gcs-scale; do
  if [ "$status" -eq 0 ] && [ -f "$evidence/$provider/PASS" ] && [ -f "$evidence/$provider/cleanup.txt" ]; then
   echo "$provider PASS"
  else echo "$provider FAIL/UNVERIFIED (see results.log)"; fi
 done > "$evidence/summary.txt"
 cat "$evidence/summary.txt"
 echo "Evidence: $evidence"
 exit "$status"
}
trap cleanup EXIT
trap 'exit 1' INT TERM
run() {
 "$@" & child=$!
 (while kill -0 "$child" 2>/dev/null; do
   df -Pk "$root" "$work" >> "$evidence/disk.log"
   if ! space; then echo 'FAIL: 4 GiB disk reserve reached' >> "$evidence/disk-failure.log"; kill -INT "$child"; exit; fi
   sleep 1
 done) & watch=$!
 set +e; wait "$child"; result=$?; set -e
 # wait reaped this PID; never signal it later after possible PID reuse.
 # Native main receipts remain available to the ownership-checked fallback.
 child=
 kill "$watch" 2>/dev/null || :; wait "$watch" 2>/dev/null || :; watch=
 [ ! -e "$evidence/disk-failure.log" ] && [ "$result" -eq 0 ]
}
native_docker() { env -i PATH="$PATH" DOCKER_CONFIG="$work/docker-config" DOCKER_HOST="$endpoint" docker "$@"; }
platform=$(native_docker version --format '{{.Server.Platform.Name}}') || { echo 'UNVERIFIED: cannot establish native Docker server identity' >&2; exit 1; }
server_os=$(native_docker info --format '{{.OperatingSystem}}') || { echo 'UNVERIFIED: cannot establish native Docker server OS' >&2; exit 1; }
independent_native_server "$platform" "$server_os" || { echo 'UNVERIFIED: unknown Docker server identity requires owner review' >&2; exit 1; }
printf '%s\n%s\n%s\n%s\n' "$context_name" "$endpoint" "$platform" "$server_os" > "$evidence/native-daemon.txt"
native_docker version --format '{{.Server.Os}}/{{.Server.Arch}}' > "$evidence/native-platform.txt"
cd "$root"
git rev-parse HEAD > "$evidence/source-sha.txt"
git diff HEAD --binary > "$work/source-diff"
if command -v sha256sum >/dev/null; then sha256sum "$work/source-diff"; else shasum -a 256 "$work/source-diff"; fi > "$evidence/source-diff.sha256"
find "$work/source-diff" -type f -delete
git status --short > "$evidence/source-status.txt"
mkdir "$evidence/harness-source"
cp scripts/verify-independent-object-dr.sh internal/grant/independent_object_dr_test.go internal/grant/independent_object_dr_authority_test.go "$evidence/harness-source/"
cp scripts/testdata/independent-object-dr/guards.sh "$evidence/harness-source/"
gopath=$(GOENV=off go env GOPATH); gocache=$(GOENV=off go env GOCACHE); goroot=$(GOENV=off go env GOROOT)
# Empty environment and owned Docker configuration prevent credential inheritance.
run env -i PATH="$PATH" DOCKER_CONFIG="$work/docker-config" TMPDIR="$work/tmp" GOPATH="$gopath" GOCACHE="$gocache" GOROOT="$goroot" GOENV=off GOTOOLCHAIN=local GOPROXY=https://proxy.golang.org GOSUMDB=sum.golang.org CGO_ENABLED=0 GOMAXPROCS=2 "$goroot/bin/go" build -mod=readonly -p=1 -o "$work/grant" ./cmd/grant > "$evidence/build.log" 2>&1
run env -i PATH="$PATH" DOCKER_CONFIG="$work/docker-config" TMPDIR="$work/tmp" GOPATH="$gopath" GOCACHE="$gocache" GOROOT="$goroot" GOENV=off GOTOOLCHAIN=local GOPROXY=https://proxy.golang.org GOSUMDB=sum.golang.org CGO_ENABLED=0 GOMAXPROCS=2 "$goroot/bin/go" test -mod=readonly -p=1 -c -o "$work/verify.test" ./internal/grant > "$evidence/test-build.log" 2>&1
"$goroot/bin/go" version -m "$work/grant" > "$evidence/buildinfo.txt"
"$goroot/bin/go" version -m "$work/verify.test" > "$evidence/helper-buildinfo.txt"
run env -i PATH="$PATH" DOCKER_CONFIG="$work/docker-config" TMPDIR="$work/tmp" DOCKER_HOST="$endpoint" GRANT_INDEPENDENT_OBJECT_DR=1 GRANT_INDEPENDENT_BINARY="$work/grant" GRANT_INDEPENDENT_EVIDENCE="$evidence" GOMAXPROCS=2 "$work/verify.test" -test.run='^TestIndependentObjectDR$' -test.v -test.timeout=15m > "$evidence/results.log" 2>&1
cat "$evidence/results.log"
for provider in s3 gcs azure gcs-scale; do
 test -f "$evidence/$provider/PASS" || { echo "FAIL: $provider missing completion receipt" >&2; exit 1; }
done
echo "PASS: all three basic provider DR cases and repeated GCS scale case; evidence $evidence"
