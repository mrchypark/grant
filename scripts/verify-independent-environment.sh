#!/bin/sh
# One disposable local entry point; no cluster access or deployment.
set -eu
umask 077

usage() {
  echo 'Usage: sh scripts/verify-independent-environment.sh [all|unit|dr]'
  echo 'all (default): go test ./..., then all-three-provider process DR'
  echo 'unit: Go tests only; integration helpers remain SKIP/UNVERIFIED'
  echo 'dr: scripts/verify-independent-object-dr.sh only; Docker required'
}
[ "$#" -le 1 ] || { usage >&2; exit 2; }
phase=${1:-all}
case "$phase" in
  -h|--help) usage; exit 0 ;;
  all|unit|dr) ;;
  *) usage >&2; exit 2 ;;
esac
root=$(CDPATH='' cd -- "$(dirname -- "$0")/.." && pwd -P)
cd "$root"
for tool in git go mktemp uname; do
  command -v "$tool" >/dev/null || { echo "UNVERIFIED: missing $tool" >&2; exit 1; }
done
case "$(uname -s):$(uname -m)" in
  Linux:x86_64|Linux:aarch64|Linux:arm64|Darwin:x86_64|Darwin:arm64) ;;
  *) echo 'SKIP/UNVERIFIED: supported hosts are Linux/macOS AMD64 or ARM64' >&2; exit 77 ;;
esac
if [ "$phase" != unit ]; then
  case "${DOCKER_HOST:-}" in
    *[Dd][Oo][Rr][Yy]*) echo 'UNVERIFIED: Dory bridge is not an independent native daemon' >&2; exit 1 ;;
    ''|unix:///*) ;;
    *) echo 'UNVERIFIED: ambient DOCKER_HOST must be a local Unix socket' >&2; exit 1 ;;
  esac
  command -v docker >/dev/null || { echo 'UNVERIFIED: missing docker' >&2; exit 1; }
  if [ -n "${DOCKER_HOST:-}" ]; then
    endpoint=$DOCKER_HOST
  else
    context=$(docker context show)
    case "$context" in
      *[Dd][Oo][Rr][Yy]*) echo 'UNVERIFIED: Dory context is not an independent native daemon' >&2; exit 1 ;;
    esac
    endpoint=$(docker context inspect "$context" --format '{{.Endpoints.docker.Host}}')
  fi
  case "$endpoint" in
    *[Dd][Oo][Rr][Yy]*) echo 'UNVERIFIED: Dory bridge is not an independent native daemon' >&2; exit 1 ;;
    unix:///*) ;;
    *) echo 'UNVERIFIED: Docker context must use a local Unix socket' >&2; exit 1 ;;
  esac
fi
evidence=$(mktemp -d "${TMPDIR:-/tmp}/grant-independent-environment.XXXXXX")
echo "Environment evidence: $evidence"
git rev-parse HEAD > "$evidence/source-sha.txt"
git status --short > "$evidence/source-status.txt"
# Hash Git's tracked diff stream, not arbitrary worktree/untracked targets.
tracked_diff=$(git diff --no-ext-diff --no-textconv --binary HEAD)
printf '%s' "$tracked_diff" | git hash-object --stdin > "$evidence/tracked-diff-object-id.txt"
unset tracked_diff
for directory in scripts scripts/testdata scripts/testdata/independent-object-dr internal internal/grant; do
  [ ! -L "$directory" ] || { echo "UNVERIFIED: harness ancestor is a symlink: $directory" >&2; exit 1; }
done
for source in scripts/verify-independent-environment.sh scripts/verify-independent-object-dr.sh \
  scripts/testdata/independent-object-dr/guards.sh \
  internal/grant/independent_object_dr_test.go internal/grant/independent_object_dr_authority_test.go; do
  [ -f "$source" ] && [ ! -L "$source" ] || { echo "UNVERIFIED: missing/nonregular harness file $source" >&2; exit 1; }
  printf '%s %s\n' "$(git hash-object --no-filters -- "$source")" "$source"
done > "$evidence/harness-object-ids.txt"
uname -sm > "$evidence/host.txt"
printf 'phase=%s\nunit=NOT_RUN\ndr=NOT_RUN\n' "$phase" > "$evidence/result.txt"
# Explicit user cache paths work without setting HOME in a child environment.
gopath=${GOPATH:-$HOME/go}
case "$(uname -s)" in
  Darwin) gocache=${GOCACHE:-$HOME/Library/Caches/go-build} ;;
  *) gocache=${GOCACHE:-${XDG_CACHE_HOME:-$HOME/.cache}/go-build} ;;
esac
go_driver=$(command -v go)
go_toolchain=auto
go_clean() {
  env -i PATH="$PATH" GOPATH="$gopath" GOCACHE="$gocache" \
    GOENV=off GOWORK=off GOTOOLCHAIN="$go_toolchain" \
    GOPROXY=https://proxy.golang.org GOSUMDB=sum.golang.org "$go_driver" "$@"
}
goroot=$(go_clean env GOROOT)
go_driver="$goroot/bin/go"
go_toolchain=local
go_clean version > "$evidence/go-version.txt"
go_clean list -m github.com/mrchypark/rhiza > "$evidence/rhiza-version.txt"

if [ "$phase" != dr ]; then
  # No cloud/kube credentials or HOME/CODEX_HOME override enters the child.
  work=$(mktemp -d "${TMPDIR:-/tmp}/grant-independent-unit.XXXXXX")
  mkdir "$work/tmp" "$work/docker"
  mkdir "$evidence/unit-processes"
  child='' observer=''
  # Record descendants while their parentage is known. Reused PIDs are never signaled.
  snapshot_unit() {
    seeds=''
    for receipt in "$evidence/unit-processes/"*.receipt; do
      [ -f "$receipt" ] || continue
      pid=${receipt##*/}; pid=${pid%.receipt}
      start=$(LC_ALL=C ps -p "$pid" -o lstart= 2>/dev/null) || continue
      [ "$start" = "$(sed -n '1p' "$receipt")" ] || continue
      seeds="$seeds $pid"
    done
    if [ -n "$child" ] && [ ! -f "$evidence/unit-processes/$child.receipt" ]; then
      parent=$(ps -p "$child" -o ppid= 2>/dev/null | tr -d ' ') || return 1
      [ "$parent" != "$$" ] || seeds="$seeds $child"
    fi
    process_tree=$(ps -axo pid=,ppid=) || return 1
    printf '%s\n' "$process_tree" | awk -v roots="$seeds" '
      BEGIN {n=split(roots,r," "); for(i=1;i<=n;i++) owned[r[i]]=1}
      {parent[$1]=$2}
      END {for(pass=0;pass<100;pass++) for(p in parent) if(owned[parent[p]]) owned[p]=1;
           for(p in parent) if(owned[p]) print p}' > "$work/descendant-pids"
    while IFS= read -r pid; do
      start=$(LC_ALL=C ps -p "$pid" -o lstart= 2>/dev/null) || continue
      args=$(LC_ALL=C ps -p "$pid" -o args= 2>/dev/null) || continue
      if [ -d /proc ]; then
        binary=$(readlink "/proc/$pid/exe" 2>/dev/null) || continue
      else
        binary=$(LC_ALL=C ps -p "$pid" -o comm= 2>/dev/null) || continue
      fi
      case "$binary" in "$work/"*|"$goroot/"*) ;;
        *) printf 'Unverified descendant PID %s; preserve on cleanup if still active\n' "$pid" >> "$evidence/unit-cleanup.log"; binary=UNVERIFIED ;;
      esac
      printf '%s\n%s\n%s\n' "$start" "$args" "$binary" > "$evidence/unit-processes/$pid.receipt.new"
      mv "$evidence/unit-processes/$pid.receipt.new" "$evidence/unit-processes/$pid.receipt"
    done < "$work/descendant-pids"
  }
  cleanup_unit() {
    result=${1:-$?}
    trap - EXIT INT TERM
    [ -z "$observer" ] || { kill -TERM "$observer" 2>/dev/null || :; wait "$observer" 2>/dev/null || :; observer=''; }
    if ! snapshot_unit; then
      echo "FAIL/UNVERIFIED: cannot snapshot descendants; preserving $work" >&2; exit 1
    fi
    # First TERM, then KILL after five seconds; every signal revalidates identity.
    round=0
    while [ "$round" -le 10 ]; do
      alive=0
      for receipt in "$evidence/unit-processes/"*.receipt; do
        [ -f "$receipt" ] || continue
        pid=${receipt##*/}; pid=${pid%.receipt}
        start=$(LC_ALL=C ps -p "$pid" -o lstart= 2>/dev/null) || continue
        [ "$start" = "$(sed -n '1p' "$receipt")" ] || continue
        args=$(LC_ALL=C ps -p "$pid" -o args= 2>/dev/null) || { alive=1; continue; }
        if [ -d /proc ]; then binary=$(readlink "/proc/$pid/exe" 2>/dev/null) || { alive=1; continue; }
        else binary=$(LC_ALL=C ps -p "$pid" -o comm= 2>/dev/null) || { alive=1; continue; }; fi
        state=$(LC_ALL=C ps -p "$pid" -o stat= 2>/dev/null) || { alive=1; continue; }
        case "$state" in *Z*) continue ;; esac
        alive=1
        [ "$args" = "$(sed -n '2p' "$receipt")" ] && [ "$binary" = "$(sed -n '3p' "$receipt")" ] || { result=1; continue; }
        case "$binary" in "$work/"*|"$goroot/"*) ;; *) result=1; continue ;; esac
        [ -f "$binary" ] && [ ! -L "$binary" ] || { result=1; continue; }
        result=1 # Needing fallback cleanup must not turn a failed test into PASS.
        if [ "$round" -eq 0 ]; then
          printf 'TERM owned PID %s with matched start/args/executable\n' "$pid" >> "$evidence/unit-cleanup.log"
          kill -TERM "$pid" 2>/dev/null || :
        fi
        if [ "$round" -eq 5 ]; then
          printf 'KILL owned PID %s with matched start/args/executable\n' "$pid" >> "$evidence/unit-cleanup.log"
          kill -KILL "$pid" 2>/dev/null || :
        fi
      done
      [ "$alive" -ne 0 ] || break
      round=$((round+1)); sleep 1
    done
    if [ "$alive" -ne 0 ]; then
      echo "FAIL/UNVERIFIED: owned descendants unresolved; preserving $work" >&2
      echo "preserved=$work" >> "$evidence/unit-cleanup.log"
      exit 1
    fi
    [ -z "$child" ] || { wait "$child" 2>/dev/null || :; child=''; }
    # A missed orphan with an executable in our temporary tree also blocks deletion.
    if command -v lsof >/dev/null; then
      lsof +D "$work" > "$evidence/unit-open-files.log" 2> "$evidence/unit-open-files.err" && open_status=0 || open_status=$?
      if [ "$open_status" -ne 1 ] || [ -s "$evidence/unit-open-files.log" ] || [ -s "$evidence/unit-open-files.err" ]; then
        echo "FAIL/UNVERIFIED: runtime tree still open or inspection failed; preserving $work" >&2; exit 1
      fi
    else
      echo "FAIL/UNVERIFIED: lsof required before removing $work" >&2; exit 1
    fi
    rm -rf "$work"
    test ! -e "$work" || exit 1
    echo 'owned unit runtime tree removed after process verification' >> "$evidence/unit-cleanup.log"
    printf 'cleanup_exit=%s\n' "$result" >> "$evidence/unit-cleanup.log"
    [ "$result" -eq 0 ] || exit "$result"
  }
  trap 'cleanup_unit "$?"' EXIT
  trap 'exit 1' INT TERM
  env -i PATH="$PATH" TMPDIR="$work/tmp" DOCKER_CONFIG="$work/docker" \
    GOPATH="$gopath" GOCACHE="$gocache" GOROOT="$goroot" \
    GOENV=off GOWORK=off GOTOOLCHAIN=local \
    GOPROXY=https://proxy.golang.org GOSUMDB=sum.golang.org \
    CGO_ENABLED=0 GOMAXPROCS=2 "$goroot/bin/go" test -v -mod=readonly -p=1 -timeout=10m ./... \
    > "$evidence/unit.log" 2>&1 &
  child=$!
  snapshot_unit
  (while kill -0 "$child" 2>/dev/null; do snapshot_unit; sleep 1; done) & observer=$!
  status=0
  wait "$child" || status=$?
  # Keep descendant receipts after reaping the Go parent; a test child may outlive it.
  child=''
  cat "$evidence/unit.log"
  printf 'unit_exit=%s\n' "$status" >> "$evidence/result.txt"
  grep '^[[:space:]]*--- SKIP:' "$evidence/unit.log" > "$evidence/unit-skips.txt" || :
  cleanup_unit "$status"
  trap - EXIT INT TERM
  [ "$status" -eq 0 ] || exit "$status"
  echo 'unit=PASS (integration helpers may be SKIP)' >> "$evidence/result.txt"
fi
if [ "$phase" != unit ]; then
  # Resolve the socket once; never pass consumer Docker config or kube credentials.
  work=$(mktemp -d "${TMPDIR:-/tmp}/grant-independent-docker.XXXXXX")
  mkdir "$work/config"
  child=
  cleanup_dr() {
    if [ -n "$child" ]; then
      kill -TERM "$child" 2>/dev/null || :
      wait "$child" 2>/dev/null || :
    fi
    rm -rf "$work"
    test ! -e "$work"
  }
  trap cleanup_dr EXIT
  trap 'exit 1' INT TERM
  status=0
  env -i PATH="$PATH" TMPDIR="${TMPDIR:-/tmp}" \
    DOCKER_CONFIG="$work/config" DOCKER_HOST="$endpoint" \
    GOPATH="$gopath" GOCACHE="$gocache" GOROOT="$goroot" \
    GOENV=off GOWORK=off GOTOOLCHAIN=local \
    sh scripts/verify-independent-object-dr.sh > "$evidence/dr.log" 2>&1 &
  child=$!
  wait "$child" || status=$?
  child=
  cat "$evidence/dr.log"
  printf 'dr_exit=%s\n' "$status" >> "$evidence/result.txt"
  cleanup_dr
  trap - EXIT INT TERM
  [ "$status" -eq 0 ] || exit "$status"
  echo 'dr=PASS (S3, GCS, Azure emulator process DR only)' >> "$evidence/result.txt"
fi
echo "PASS: selected phase $phase; evidence $evidence"
