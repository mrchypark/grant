#!/bin/sh
# Exercise the real contract's early failure/cleanup paths without a Docker daemon.
set -eu
root=$(CDPATH='' cd -- "$(dirname -- "$0")/.." && pwd)
contract=${1:-$root/scripts/verify-standalone-container.sh}
task_dir=$(mktemp -d "${TMPDIR:-/tmp}/grant-container-ownership.XXXXXX")
trap 'rm -r "$task_dir"' EXIT HUP INT TERM
mkdir "$task_dir/bin"
cat > "$task_dir/bin/docker" <<'DOCKER'
#!/bin/sh
set -eu
last=''
for arg do last=$arg; done
case "$1 $2" in
 'context inspect') echo unix:///fake/docker.sock ;;
 'info --format'|'image inspect') echo amd64 ;;
 'volume inspect')
  test -f "$FIXTURE/volume-$last" || exit 1
  cat "$FIXTURE/volume-$last" ;;
 'volume create')
  label=$4
  case "$SCENARIO:$last" in
   volume-collision:*-archive) label=grant.verify.owner=foreign ;;
  esac
  printf '%s\n' "${label#grant.verify.owner=}" > "$FIXTURE/volume-$last"
  echo "$last" ;;
 'volume rm')
  printf 'volume-rm %s\n' "$last" >> "$FIXTURE/actions"
  rm "$FIXTURE/volume-$last" ;;
 'container inspect')
  test -f "$FIXTURE/container-$last" || exit 1
  cat "$FIXTURE/container-$last" ;;
 'run --name')
  # Replay the previous prepare path's name collision without creating a CID.
  echo foreign > "$FIXTURE/container-$3"
  exit 17 ;;
 'create --name')
  name=$3; label=$5
  case "$label" in grant.verify.owner=grant-standalone-*) ;; *) exit 95 ;; esac
  printf 'create %s\n' "$name" >> "$FIXTURE/actions"
  case "$SCENARIO:$name" in
   prepare-collision:*-permissions|run-collision:*-init-empty)
    echo foreign > "$FIXTURE/container-$name"
    exit 17 ;;
  esac
  # The filesystem owner anchor must share the writable archive volume.
  case "$name" in
   *-permissions)
    [ "$last" = 'mkdir -p /archive/root && chown 10001:10001 /data /archive /archive/root' ] || {
     echo 'archive root and parent must be prepared inside the writable volume' >&2; exit 97;
    } ;;
   *)
    archive_dir=''; archive_mount=''; readonly=false
    for arg do
     case "$arg" in
      RHIZA_OBJSTORE_DIR=*) archive_dir=${arg#RHIZA_OBJSTORE_DIR=} ;;
      *:/archive) archive_mount=$arg ;;
      --read-only) readonly=true ;;
     esac
    done
    [ "$archive_dir" = /archive/root ] && [ -n "$archive_mount" ] && [ "$readonly" = true ] || {
     echo 'archive root must be below its mount with read-only container filesystem' >&2; exit 97;
    } ;;
  esac
  case "$name" in *-permissions) id=created-prepare ;; *) id=created-init ;; esac
  printf '%s\n' "${label#grant.verify.owner=}" > "$FIXTURE/container-$id"
  echo "$id" ;;
 'start -a') test "$last" = created-prepare ;;
 'start created-init')
  if [ "$SCENARIO" = ownership-changed ]; then
   echo foreign > "$FIXTURE/container-created-prepare"
   for volume in "$FIXTURE"/volume-*-data-original; do echo foreign > "$volume"; done
  fi
  # A deterministic failure after successful creation must clean owned IDs.
  exit 23 ;;
 'logs created-prepare'|'logs created-init')
  printf 'logs %s\n' "$last" >> "$FIXTURE/actions" ;;
 'rm -f')
  printf 'container-rm %s\n' "$last" >> "$FIXTURE/actions"
  rm "$FIXTURE/container-$last" ;;
 *) printf 'unexpected Docker command: %s\n' "$*" >&2; exit 96 ;;
esac
DOCKER
chmod +x "$task_dir/bin/docker"
for scenario in prepare-collision run-collision volume-collision valid-cleanup ownership-changed; do
 fixture="$task_dir/$scenario"
 mkdir "$fixture"
 : > "$fixture/actions"
 if PATH="$task_dir/bin:$PATH" FIXTURE="$fixture" SCENARIO="$scenario" LOG_DIR="$fixture/logs" sh "$contract" > "$fixture/output" 2>&1; then
  cat "$fixture/output" >&2; echo "expected fixture failure: $scenario" >&2; exit 1
 else
  fixture_code=$?
 fi
 if [ "$fixture_code" -eq 97 ]; then cat "$fixture/output" >&2; exit 1; fi
 # Names involved in collisions must never be logged or removed.
 if grep -E '^(container-rm|logs) grant-standalone-' "$fixture/actions"; then
  cat "$fixture/output" >&2; exit 1
 fi
 case "$scenario" in
  prepare-collision)
   test "$fixture_code" -eq 17
   test -f "$fixture"/container-*-permissions
   ! grep -q '^container-rm ' "$fixture/actions" ;;
  run-collision)
   test "$fixture_code" -eq 17
   test -f "$fixture"/container-*-init-empty
   grep -qx 'container-rm created-prepare' "$fixture/actions"
   ! grep -q 'container-rm created-init' "$fixture/actions" ;;
  volume-collision)
   test "$fixture_code" -eq 1
   test -f "$fixture"/volume-*-archive
   ! grep -q '^volume-rm ' "$fixture/actions" ;;
  valid-cleanup)
   test "$fixture_code" -eq 23
   grep -qx 'container-rm created-prepare' "$fixture/actions"
   grep -qx 'container-rm created-init' "$fixture/actions"
   test "$(grep -c '^volume-rm ' "$fixture/actions")" -eq 2
   test ! -f "$fixture/container-created-prepare"
   test ! -f "$fixture/container-created-init" ;;
  ownership-changed)
   test "$fixture_code" -eq 1
   test -f "$fixture/container-created-prepare"
   test -f "$fixture"/volume-*-data-original
   ! grep -q 'container-rm created-prepare' "$fixture/actions"
   ! grep -q 'logs created-prepare' "$fixture/actions"
   ! grep -q 'volume-rm .*data-original' "$fixture/actions"
   grep -qx 'container-rm created-init' "$fixture/actions" ;;
 esac
 printf 'PASS: %s\n' "$scenario"
done
