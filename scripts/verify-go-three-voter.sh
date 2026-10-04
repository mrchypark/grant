#!/bin/sh
# Isolated local native-Rhiza verification. Does not invoke kubectl or vcluster.
set -eu
root=$(CDPATH='' cd -- "$(dirname -- "$0")/.." && pwd)
for tool in docker go curl jq; do command -v "$tool" >/dev/null; done
endpoint=$(docker context inspect --format '{{.Endpoints.docker.Host}}')
case "$endpoint" in unix://*) ;; *) echo 'requires an explicitly selected local Unix-socket Docker context' >&2; exit 1 ;; esac
task_dir=$(mktemp -d "${TMPDIR:-/tmp}/grant-go-three-voter.XXXXXX")
container="grant-go-three-voter-$$"
minio_image="$container-minio"
pids=''
cleanup() {
  result=$?
  if [ "$result" -ne 0 ]; then
    printf 'Fixture failed (exit %s); logs: %s\n' "$result" "$task_dir" >&2
    docker logs --tail 30 "$container" >"$task_dir/object-store.log" 2>&1 || true
  fi
  for pid in $pids; do kill "$pid" 2>/dev/null || true; done
  for pid in $pids; do wait "$pid" 2>/dev/null || true; done
  docker rm -f "$container" >/dev/null 2>&1 || true
  docker image rm "$minio_image" >/dev/null 2>&1 || true
  if [ "${KEEP_GRANT_FIXTURE:-0}" = 1 ]; then printf 'Fixture logs: %s\n' "$task_dir"; else rm -rf "$task_dir"; fi
}
trap cleanup EXIT HUP INT TERM
(cd "$root" && CGO_ENABLED=0 go build -o "$task_dir/grant" ./cmd/grant)
# Upstream no longer publishes the community image. Build the same release
# from its checksum-verified source (07c3a429bfed433e49018cb0f78a52145d4bedeb).
minio_source=$(go mod download -json github.com/minio/minio@v0.0.0-20250907161309-07c3a429bfed \
  | jq -er 'select(.Sum == "h1:deu0m9BiyqnMbGWfJi1PpVyotSXlDnTmRIme1cUXnxo=").Dir')
minio_arch=$(docker version --format '{{.Server.Arch}}')
CGO_ENABLED=0 GOOS=linux GOARCH="$minio_arch" go build -C "$minio_source" -mod=readonly -trimpath \
  -ldflags '-s -w -X github.com/minio/minio/cmd.Version=2025-09-07T16:13:09Z -X github.com/minio/minio/cmd.ReleaseTag=RELEASE.2025-09-07T16-13-09Z -X github.com/minio/minio/cmd.CommitID=07c3a429bfed433e49018cb0f78a52145d4bedeb' \
  -o "$task_dir/minio" .
docker build --load --platform "linux/$minio_arch" -t "$minio_image" -f - "$task_dir" <<'DOCKERFILE'
FROM scratch
COPY minio /minio
ENTRYPOINT ["/minio"]
DOCKERFILE
docker run -d --name "$container" -p 127.0.0.1::9000 \
  -e MINIO_ROOT_USER=local-grant -e MINIO_ROOT_PASSWORD=local-grant-storage-secret \
  "$minio_image" server /data >/dev/null
s3_port=$(docker inspect "$container" | jq -r '.[0].NetworkSettings.Ports["9000/tcp"][0].HostPort // empty')
case "$s3_port" in ''|*[!0-9]*) echo 'local Docker did not report a numeric published S3 port' >&2; exit 1 ;; esac
# Dory reports wildcard binding despite the loopback publish request; its
# Unix-socket local runtime still forwards the published port to localhost.
printf 'Local object-store endpoint: http://127.0.0.1:%s\n' "$s3_port"
i=0
until curl -fsS "http://127.0.0.1:$s3_port/minio/health/ready" >/dev/null 2>&1; do
  i=$((i+1)); [ "$i" -lt 60 ] || exit 1; sleep 1
done
curl -fsS --aws-sigv4 'aws:amz:us-east-1:s3' --user 'local-grant:local-grant-storage-secret' \
  -X PUT "http://127.0.0.1:$s3_port/grant-rhiza" >/dev/null
base=$((30000 + $$ % 10000))
http0=$base; http1=$((base+1)); http2=$((base+2))
peer0=$((base+3)); peer1=$((base+4)); peer2=$((base+5))
members=$(printf '[{"node_id":"grant-0","peer_url":"quic://127.0.0.1:%s","token":"local-voter-0"},{"node_id":"grant-1","peer_url":"quic://127.0.0.1:%s","token":"local-voter-1"},{"node_id":"grant-2","peer_url":"quic://127.0.0.1:%s","token":"local-voter-2"}]' "$peer0" "$peer1" "$peer2")
cluster_id=$container
restore_snapshot=''
node_root=$task_dir
configure_env() {
  export GRANT_MASTER_KEY=local-master-secret GRANT_APP_AUTH_KEY=local-app-secret \
    GRANT_AUTH_PROVIDER=rauthy GRANT_BIND_ADDR="127.0.0.1:$http" \
    GRANT_RAUTHY_CLIENT_ID=local-fixture GRANT_RAUTHY_CLIENT_SECRET=local-fixture-secret GRANT_RAUTHY_CLIENT_READ_API_KEY=local-fixture-read \
    GRANT_NODE_ID="grant-$node" GRANT_CLUSTER_ID="$cluster_id" \
    GRANT_PEER_ADDR="127.0.0.1:$peer" GRANT_CLUSTER_MEMBERS="$members" \
    GRANT_DATA_DIR="$node_root/node-$node" GRANT_BACKUP_DIR="$node_root/backups-$node" \
    GRANT_CLUSTER_RESTORE_SNAPSHOT="$restore_snapshot" \
    GRANT_RUNTIME_MODE=embedded GRANT_LOCAL_NODE_ROLE=voter GRANT_READ_CONSISTENCY=strong \
    HQL_DATA_DIR='' HQL_NODES='' HQL_NODE_ID='' \
    RHIZA_OBJSTORE_PROVIDER=s3 RHIZA_OBJSTORE_ENDPOINT="http://127.0.0.1:$s3_port" \
    RHIZA_OBJSTORE_BUCKET=grant-rhiza RHIZA_OBJSTORE_PREFIX="$cluster_id" RHIZA_OBJSTORE_REGION=us-east-1 \
    RHIZA_OBJSTORE_ACCESS_KEY=local-grant RHIZA_OBJSTORE_SECRET_KEY=local-grant-storage-secret \
    RHIZA_OBJSTORE_DURABILITY=before-ack
}
start() {
  node=$1; http=$2; peer=$3
  configure_env
  "$task_dir/grant" >"$task_dir/node-$node.log" 2>&1 &
  pids="$pids $!"
  started_pid=$!
}
start 0 "$http0" "$peer0"; pid0=$started_pid
start 1 "$http1" "$peer1"; pid1=$started_pid
start 2 "$http2" "$peer2"; pid2=$started_pid
check_children() {
 for pid in "$pid0" "$pid1" "$pid2"; do
  if ! kill -0 "$pid" 2>/dev/null; then cat "$task_dir"/node-*.log >&2; echo 'owned voter exited; refusing to contact potentially unrelated listeners' >&2; exit 1; fi
 done
}
for port in "$http0" "$http1" "$http2"; do
 i=0
 until curl -fsS "http://127.0.0.1:$port/readyz" >"$task_dir/health-$port.json"; do
  check_children
  i=$((i+1)); if [ "$i" -ge 90 ]; then cat "$task_dir"/node-*.log >&2; exit 1; fi; sleep 1
 done
done
check_children
curl -fsS -H 'X-Grant-Master-Key: local-master-secret' -H 'Content-Type: application/json' \
  --data '{"id":"quorum-fixture","name":"Quorum fixture","active":true}' \
  "http://127.0.0.1:$http0/api/v1/admin/providers" >"$task_dir/created.json"
for port in "$http0" "$http1" "$http2"; do
 curl -fsS "http://127.0.0.1:$port/api/v1/providers?consistency=strong" \
   | jq -e 'any(.[]; .id == "quorum-fixture" and .active == true)' >/dev/null
done
kill "$pid2"
wait "$pid2" || true
pids="$pid0 $pid1"
for pid in "$pid0" "$pid1"; do kill -0 "$pid"; done
curl --retry 12 --retry-all-errors --retry-delay 1 --max-time 30 -fsS \
  -H 'X-Grant-Master-Key: local-master-secret' -H 'Content-Type: application/json' \
  --data '{"id":"quorum-recovery","name":"Quorum recovery","active":true}' \
  "http://127.0.0.1:$http0/api/v1/admin/providers" >"$task_dir/recovery-created.json"
curl -fsS "http://127.0.0.1:$http1/api/v1/providers?consistency=strong" \
  | jq -e 'any(.[]; .id == "quorum-recovery")' >/dev/null
start 2 "$http2" "$peer2"; pid2=$started_pid
i=0
until curl -fsS "http://127.0.0.1:$http2/readyz" >/dev/null; do
 check_children
 i=$((i+1)); [ "$i" -lt 90 ] || exit 1; sleep 1
done
check_children
curl -fsS "http://127.0.0.1:$http2/api/v1/providers?consistency=strong" \
  | jq -e 'any(.[]; .id == "quorum-recovery")' >/dev/null
printf '%s\n' 'Native three-voter shared-S3 write, quorum survival and same-directory recovery passed'

# Production uses emptyDir: replacement must recover from shared S3 without any
# original node-local state. Preserve old fixture directories for diagnosis.
kill -KILL "$pid2"; wait "$pid2" || true
pids="$pid0 $pid1"
source_root=$node_root
node_root="$task_dir/cold-single"
mkdir "$node_root"
start 2 "$http2" "$peer2"; pid2=$started_pid
node_root=$source_root
i=0
until curl --max-time 3 -fsS "http://127.0.0.1:$http2/readyz" >/dev/null 2>&1; do
 check_children
 i=$((i+1)); [ "$i" -lt 90 ] || { cat "$task_dir"/node-*.log >&2; exit 1; }; sleep 1
done
curl -fsS "http://127.0.0.1:$http2/api/v1/providers?consistency=strong" \
 | jq -e 'any(.[]; .id=="quorum-recovery")' >/dev/null
curl -fsS -H 'X-Grant-Master-Key: local-master-secret' -H 'Content-Type: application/json' \
 --data '{"id":"cold-recovery-latest","name":"Latest before cold loss","active":true}' \
 "http://127.0.0.1:$http0/api/v1/admin/providers" >/dev/null

# Loss of quorum must close strong readiness without failing process liveness.
kill -KILL "$pid1" "$pid2"
wait "$pid1" || true; wait "$pid2" || true
pids=$pid0
kill -0 "$pid0"
curl --max-time 2 -fsS "http://127.0.0.1:$http0/healthz" | jq -e '.status=="ok"' >/dev/null
ready_code=$(curl --max-time 3 -sS -o "$task_dir/no-quorum-ready.json" -w '%{http_code}' "http://127.0.0.1:$http0/readyz")
[ "$ready_code" = 503 ] || { echo "strong readiness remained open without quorum: $ready_code" >&2; exit 1; }
kill -KILL "$pid0"; wait "$pid0" || true
pids=''
node_root="$task_dir/cold-all"
mkdir "$node_root"
start 0 "$http0" "$peer0"; pid0=$started_pid
start 1 "$http1" "$peer1"; pid1=$started_pid
start 2 "$http2" "$peer2"; pid2=$started_pid
for port in "$http0" "$http1" "$http2"; do
 i=0
 until curl --max-time 3 -fsS "http://127.0.0.1:$port/readyz" >/dev/null 2>&1; do
  check_children
  i=$((i+1)); [ "$i" -lt 90 ] || { cat "$task_dir"/node-*.log >&2; exit 1; }; sleep 1
 done
 curl -fsS "http://127.0.0.1:$port/api/v1/providers?consistency=strong" \
  | jq -e 'any(.[]; .id=="quorum-fixture") and any(.[]; .id=="quorum-recovery") and any(.[]; .id=="cold-recovery-latest")' >/dev/null
done
printf '%s\n' 'Cold single-voter and entire-quorum S3 recovery, process liveness and strong readiness passed'

# Restore a populated logical snapshot into a distinct, wholly fresh cluster.
# The original prefix and directories remain intact throughout this fixture.
curl -fsS -X POST -H 'X-Grant-Master-Key: local-master-secret' \
  "http://127.0.0.1:$http0/ops/backups" >/dev/null
set -- "$node_root"/backups-0/grant-*.json
[ "$#" -eq 1 ] && [ -f "$1" ] || { echo 'expected one verified source snapshot' >&2; exit 1; }
restore_snapshot=$1
for pid in $pids; do kill "$pid"; done
for pid in $pids; do wait "$pid" || true; done
pids=''
cluster_id="$container-restored"
node_root="$task_dir/restored"
mkdir "$node_root"
node=0; http=$http0; peer=$peer0
configure_env
"$task_dir/grant" migrate-prepare-cluster --snapshot "$restore_snapshot"
start 0 "$http0" "$peer0"; pid0=$started_pid
start 1 "$http1" "$peer1"; pid1=$started_pid
# Without the third explicit admission, neither voter may serve readiness.
i=0
while [ "$i" -lt 3 ]; do
 for pid in "$pid0" "$pid1"; do kill -0 "$pid"; done
 for port in "$http0" "$http1"; do
  if curl --max-time 1 -fsS "http://127.0.0.1:$port/readyz" >/dev/null 2>&1; then
   echo 'restore readiness opened before all voters opted in' >&2; exit 1
  fi
 done
 i=$((i+1)); sleep 1
done
start 2 "$http2" "$peer2"; pid2=$started_pid
for port in "$http0" "$http1" "$http2"; do
 i=0
 until curl --max-time 2 -fsS "http://127.0.0.1:$port/readyz" >/dev/null 2>&1; do
  check_children
  i=$((i+1)); [ "$i" -lt 90 ] || { cat "$task_dir"/node-*.log >&2; exit 1; }; sleep 1
 done
 check_children
 curl -fsS "http://127.0.0.1:$port/api/v1/providers?consistency=strong" \
  | jq -e 'any(.[]; .id=="quorum-fixture") and any(.[]; .id=="quorum-recovery")' >/dev/null
done
# Admission retries are idempotent, but changed membership must not overwrite
# the existing admission or active target prefix.
node=0; http=$http0; peer=$peer0
original_members=$members
members=$(printf '%s' "$members" | jq -c '.[2].node_id="different-admission-voter"')
configure_env
if "$task_dir/grant" migrate-prepare-cluster --snapshot "$restore_snapshot" >"$task_dir/reprepare.log" 2>&1; then
 echo 'restore replaced existing target admission with different membership' >&2; exit 1
fi
members=$original_members
kill "$pid2"; wait "$pid2" || true
pids="$pid0 $pid1"
for pid in "$pid0" "$pid1"; do kill -0 "$pid"; done
curl --retry 12 --retry-all-errors --retry-delay 1 --max-time 30 -fsS \
 -H 'X-Grant-Master-Key: local-master-secret' -H 'Content-Type: application/json' \
 --data '{"id":"restore-recovery","name":"Restored quorum recovery","active":true}' \
 "http://127.0.0.1:$http0/api/v1/admin/providers" >/dev/null
start 2 "$http2" "$peer2"; pid2=$started_pid
i=0
until curl --max-time 2 -fsS "http://127.0.0.1:$http2/readyz" >/dev/null 2>&1; do
 check_children
 i=$((i+1)); [ "$i" -lt 90 ] || exit 1; sleep 1
done
check_children
curl -fsS "http://127.0.0.1:$http2/api/v1/providers?consistency=strong" \
 | jq -e 'any(.[]; .id=="restore-recovery")' >/dev/null
printf '%s\n' 'Fresh three-voter snapshot restore, closed admission readiness, occupied-target rejection and recovery passed'
