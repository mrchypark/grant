#!/bin/sh
# Isolated local native-Rhiza verification. Does not invoke kubectl or vcluster.
set -eu
root=$(CDPATH='' cd -- "$(dirname -- "$0")/.." && pwd)
for tool in docker go curl jq xxd base64 awk; do command -v "$tool" >/dev/null; done
endpoint=$(docker context inspect --format '{{.Endpoints.docker.Host}}')
case "$endpoint" in unix://*) ;; *) echo 'requires an explicitly selected local Unix-socket Docker context' >&2; exit 1 ;; esac
task_dir=$(mktemp -d "${TMPDIR:-/tmp}/grant-go-three-voter.XXXXXX")
container="grant-go-three-voter-$$"
object_store_image=docker.io/versity/versitygw:v1.8.0@sha256:30292fc2eeacc67a36993b01f7a7a5e3361a19cced0e80c1d71cfa2a4b0a2499
pids=''
cleanup() {
  result=$?
  if [ "$result" -ne 0 ]; then
    printf 'Fixture failed (exit %s); logs: %s\n' "$result" "$task_dir" >&2
    docker logs --tail 30 "$container" >"$task_dir/object-store.log" 2>&1 || true
    for log in "$task_dir"/node-*.log; do
      [ -f "$log" ] || continue
      printf '%s\n' "Recent ${log##*/} lines (fixture credentials redacted):" >&2
      tail -n 20 "$log" | sed \
        -e 's/local-master-secret/[REDACTED]/g' \
        -e 's/local-app-secret/[REDACTED]/g' \
        -e 's/local-fixture-secret/[REDACTED]/g' \
        -e 's/local-fixture-read/[REDACTED]/g' \
        -e 's/local-runtime-secret-distinct-32-bytes/[REDACTED]/g' \
        -e 's/local-rhiza-admin-012345678901234567890123/[REDACTED]/g' \
        -e 's/local-voter-[0-3]-012345678901234567890123/[REDACTED]/g' \
        -e 's/local-grant-storage-secret/[REDACTED]/g' >&2
    done
  fi
  for pid in $pids; do kill "$pid" 2>/dev/null || true; done
  for pid in $pids; do wait "$pid" 2>/dev/null || true; done
  docker rm -fv "$container" >/dev/null 2>&1 || true
  if [ "${KEEP_GRANT_FIXTURE:-0}" = 1 ]; then printf 'Fixture logs: %s\n' "$task_dir"; else rm -rf "$task_dir"; fi
}
trap cleanup EXIT HUP INT TERM
(cd "$root" && CGO_ENABLED=0 go build -o "$task_dir/grant" ./cmd/grant)
object_store_arch=$(docker version --format '{{.Server.Arch}}')
docker run -d --platform "linux/$object_store_arch" --name "$container" -p 127.0.0.1::9000 \
  --mount type=volume,target=/data \
  -e ROOT_ACCESS_KEY=local-grant -e ROOT_SECRET_KEY=local-grant-storage-secret \
  "$object_store_image" --port :9000 --health /_/health posix /data >/dev/null
s3_port=$(docker inspect "$container" | jq -r '.[0].NetworkSettings.Ports["9000/tcp"][0].HostPort // empty')
case "$s3_port" in ''|*[!0-9]*) echo 'local Docker did not report a numeric published S3 port' >&2; exit 1 ;; esac
# Dory reports wildcard binding despite the loopback publish request; its
# Unix-socket local runtime still forwards the published port to localhost.
printf 'Local object-store endpoint: http://127.0.0.1:%s\n' "$s3_port"
i=0
until curl -fsS "http://127.0.0.1:$s3_port/_/health" >/dev/null 2>&1; do
  i=$((i+1)); [ "$i" -lt 60 ] || exit 1; sleep 1
done
curl -fsS --aws-sigv4 'aws:amz:us-east-1:s3' --user 'local-grant:local-grant-storage-secret' \
  -X PUT "http://127.0.0.1:$s3_port/grant-rhiza" >/dev/null
curl -fsS --aws-sigv4 'aws:amz:us-east-1:s3' --user 'local-grant:local-grant-storage-secret' \
  -X PUT --data 'grant-fixture-roundtrip' "http://127.0.0.1:$s3_port/grant-rhiza/fixture-probe" >/dev/null
probe=$(curl -fsS --aws-sigv4 'aws:amz:us-east-1:s3' --user 'local-grant:local-grant-storage-secret' \
  "http://127.0.0.1:$s3_port/grant-rhiza/fixture-probe")
[ "$probe" = grant-fixture-roundtrip ] || { echo 'S3 path-style roundtrip failed' >&2; exit 1; }
denied=$(curl -sS -o /dev/null -w '%{http_code}' --aws-sigv4 'aws:amz:us-east-1:s3' \
  --user 'local-grant:wrong-fixture-secret' "http://127.0.0.1:$s3_port/grant-rhiza/fixture-probe")
[ "$denied" = 403 ] || { echo "S3 accepted invalid fixture credentials: $denied" >&2; exit 1; }
curl -fsS --aws-sigv4 'aws:amz:us-east-1:s3' --user 'local-grant:local-grant-storage-secret' \
  -X DELETE "http://127.0.0.1:$s3_port/grant-rhiza/fixture-probe" >/dev/null
printf '%s\n' 'Versity S3 path-style bucket creation, signed roundtrip and invalid-credential rejection passed'
base=$((30000 + $$ % 10000))
http0=$base; http1=$((base+1)); http2=$((base+2))
peer0=$((base+3)); peer1=$((base+4)); peer2=$((base+5))
operator0=$((base+6)); operator1=$((base+7)); operator2=$((base+8))
cluster_id=$container
token0=local-voter-0-012345678901234567890123
token1=local-voter-1-012345678901234567890123
token2=local-voter-2-012345678901234567890123
token3=local-voter-3-012345678901234567890123
admin_token=local-rhiza-admin-012345678901234567890123
peer_tokens=$(printf '{"grant-0":"%s","grant-1":"%s","grant-2":"%s"}' "$token0" "$token1" "$token2")
legacy_members=$(printf '[{"node_id":"grant-0","peer_url":"quic://127.0.0.1:%s","token":"%s"},{"node_id":"grant-1","peer_url":"quic://127.0.0.1:%s","token":"%s"},{"node_id":"grant-2","peer_url":"quic://127.0.0.1:%s","token":"%s"}]' "$peer0" "$token0" "$peer1" "$token1" "$peer2" "$token2")
public_key() {
  raw=$(printf '{"ClusterID":"%s","Members":[{"node_id":"%s","peer_url":"quic://127.0.0.1:0","token":"%s"}]}' "$1" "$2" "$3" | "$root/scripts/replica-members")
  hex=$(printf '%s' "$raw" | jq -r '.[0].PublicKey[]' | awk '{printf "%02x", $1}')
  printf '%s' "$hex" | xxd -r -p | base64 | tr -d '\n'
}
key0=$(public_key "$cluster_id" grant-0 "$token0")
key1=$(public_key "$cluster_id" grant-1 "$token1")
key2=$(public_key "$cluster_id" grant-2 "$token2")
members=$(printf '[{"node_id":"grant-0","peer_url":"quic://127.0.0.1:%s","public_key":"%s"},{"node_id":"grant-1","peer_url":"quic://127.0.0.1:%s","public_key":"%s"},{"node_id":"grant-2","peer_url":"quic://127.0.0.1:%s","public_key":"%s"}]' "$peer0" "$key0" "$peer1" "$key1" "$peer2" "$key2")
restore_snapshot=''
node_root=$task_dir
legacy_mode=0
configure_legacy_env() {
  unset RHIZA_BIND_ADDR RHIZA_CLUSTER_ID RHIZA_NODE_ID RHIZA_DATA_DIR RHIZA_PEER_ADDR RHIZA_CLUSTER_MEMBERS RHIZA_PEER_TOKENS RHIZA_PEER_TOKEN RHIZA_ADMIN_TOKEN RHIZA_ENABLE_RECONFIGURATION RHIZA_LEARNER
  unset GRANT_RUNTIME_BIND_ADDR GRANT_REMOTE_API_SECRET GRANT_READ_FENCE_KEY
  export GRANT_BIND_ADDR="127.0.0.1:$http" GRANT_NODE_ID="grant-$node" GRANT_CLUSTER_ID="$cluster_id" GRANT_PEER_ADDR="127.0.0.1:$peer" \
    GRANT_CLUSTER_MEMBERS="$legacy_members" GRANT_DATA_DIR="$node_root/node-$node" \
    GRANT_BACKUP_DIR="$node_root/backups-$node" GRANT_CLUSTER_RESTORE_SNAPSHOT="$restore_snapshot" \
    RHIZA_OBJSTORE_PREFIX="$cluster_id"
}
configure_env() {
  if [ "$legacy_mode" = 1 ]; then configure_legacy_env; return; fi
  unset GRANT_DATA_DIR GRANT_NODE_ID GRANT_CLUSTER_ID GRANT_PEER_ADDR GRANT_CLUSTER_MEMBERS \
    GRANT_RHIZA_CONFIG_JSON GRANT_RHIZA_REPLICA_CONFIG_JSON GRANT_REMOTE_NODES \
    GRANT_REMOTE_API_SECRET GRANT_READ_FENCE_KEY GRANT_RUNTIME_BIND_ADDR GRANT_CLUSTER_RESTORE_SNAPSHOT RHIZA_PEER_TOKEN
  export GRANT_MASTER_KEY=local-master-secret GRANT_APP_AUTH_KEY=local-app-secret \
    GRANT_AUTH_PROVIDER=rauthy GRANT_BIND_ADDR="127.0.0.1:$http" \
    GRANT_RAUTHY_CLIENT_ID=local-fixture GRANT_RAUTHY_CLIENT_SECRET=local-fixture-secret GRANT_RAUTHY_CLIENT_READ_API_KEY=local-fixture-read \
    GRANT_BACKUP_DIR="$node_root/backups-$node" \
    GRANT_RUNTIME_MODE=embedded GRANT_LOCAL_NODE_ROLE=voter GRANT_READ_CONSISTENCY=strong \
    HQL_DATA_DIR='' HQL_NODES='' HQL_NODE_ID='' \
    RHIZA_OBJSTORE_PROVIDER=s3 RHIZA_OBJSTORE_ENDPOINT="127.0.0.1:$s3_port" RHIZA_OBJSTORE_INSECURE=true \
    RHIZA_OBJSTORE_BUCKET=grant-rhiza RHIZA_OBJSTORE_PREFIX="$cluster_id" RHIZA_OBJSTORE_REGION=us-east-1 \
    RHIZA_OBJSTORE_ACCESS_KEY=local-grant RHIZA_OBJSTORE_SECRET_KEY=local-grant-storage-secret \
    RHIZA_OBJSTORE_DURABILITY=before-ack RHIZA_CLUSTER_ID="$cluster_id" RHIZA_NODE_ID="grant-$node" \
    RHIZA_DATA_DIR="$node_root/node-$node" RHIZA_PEER_ADDR="127.0.0.1:$peer" \
    RHIZA_BIND_ADDR="127.0.0.1:$operator" RHIZA_CLUSTER_MEMBERS="$members" \
    RHIZA_PEER_TOKENS="$peer_tokens" RHIZA_ADMIN_TOKEN="$admin_token" \
    RHIZA_ENABLE_RECONFIGURATION=true RHIZA_LEARNER=''
}
start() {
  node=$1; http=$2; peer=$3; operator=$4
  configure_env
  "$task_dir/grant" >"$task_dir/node-$node.log" 2>&1 &
  pids="$pids $!"
  started_pid=$!
}
start 0 "$http0" "$peer0" "$operator0"; pid0=$started_pid
start 1 "$http1" "$peer1" "$operator1"; pid1=$started_pid
start 2 "$http2" "$peer2" "$operator2"; pid2=$started_pid
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
curl -fsS "http://127.0.0.1:$operator0/recovery/status" >"$task_dir/operator-status.json"
jq -e '.cluster_id == "'"$cluster_id"'" and .node_id == "grant-0"' "$task_dir/operator-status.json" >/dev/null
operator_public_code=$(curl --max-time 3 -sS -o /dev/null -w '%{http_code}' "http://127.0.0.1:$http0/recovery/status")
[ "$operator_public_code" = 404 ] || { echo "public Grant listener exposed OperatorHandler: $operator_public_code" >&2; exit 1; }
sql_public_code=$(curl --max-time 3 -sS -o /dev/null -w '%{http_code}' "http://127.0.0.1:$http0/sql/query")
[ "$sql_public_code" = 404 ] || { echo "public Grant listener exposed native SQL: $sql_public_code" >&2; exit 1; }
wrong_admin_code=$(curl --max-time 3 -sS -o /dev/null -w '%{http_code}' \
  -H 'Authorization: Bearer wrong-rhiza-admin' "http://127.0.0.1:$operator0/membership/status")
[ "$wrong_admin_code" = 403 ] || { echo "Operator accepted wrong admin token: $wrong_admin_code" >&2; exit 1; }
curl -fsS -H "Authorization: Bearer $admin_token" \
  "http://127.0.0.1:$operator0/membership/status" >"$task_dir/membership-status.json"
printf '%s\n' 'Dedicated private OperatorHandler, public exposure denial and admin authentication passed'
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
start 2 "$http2" "$peer2" "$operator2"; pid2=$started_pid
i=0
until curl -fsS "http://127.0.0.1:$http2/readyz" >/dev/null; do
 check_children
 i=$((i+1)); [ "$i" -lt 90 ] || exit 1; sleep 1
done
check_children
curl -fsS "http://127.0.0.1:$http2/api/v1/providers?consistency=strong" \
  | jq -e 'any(.[]; .id == "quorum-recovery")' >/dev/null
printf '%s\n' 'Native three-voter shared-S3 write, quorum survival and same-directory recovery passed'

# An empty directory must never reclaim a registered voter identity.
old2_status=$(curl -fsS -H "Authorization: Bearer $admin_token" "http://127.0.0.1:$operator2/membership/status")
old2_wal=$(printf '%s' "$old2_status" | jq -r '.wal_identity')
kill -KILL "$pid2"; wait "$pid2" || true
pids="$pid0 $pid1"
source_root=$node_root
node_root="$task_dir/cold-single"
mkdir "$node_root"
node=2; http=$http2; peer=$peer2; operator=$operator2
configure_env
if "$task_dir/grant" >"$task_dir/same-id-empty-wal.log" 2>&1 & then
  bad_pid=$!
else
  bad_pid=$!
fi
sleep 5
if kill -0 "$bad_pid" 2>/dev/null; then
  kill "$bad_pid" 2>/dev/null || true
  wait "$bad_pid" 2>/dev/null || true
  echo 'same-ID empty-WAL replacement unexpectedly stayed alive' >&2
  exit 1
fi
if wait "$bad_pid"; then
  echo 'same-ID empty-WAL replacement unexpectedly succeeded' >&2
  exit 1
else
  bad_status=$?
fi
[ "$bad_status" -ne 0 ] || exit 1
printf '%s\n' 'Same-ID empty-WAL replacement was rejected before serving'
node_root=$source_root
expected_config=$(curl -fsS -H "Authorization: Bearer $admin_token" "http://127.0.0.1:$operator0/membership/status" | jq -r '.config_id')
key3=$(public_key "$cluster_id" grant-3 "$token3")
learner_member=$(printf '{"node_id":"grant-3","peer_url":"quic://127.0.0.1:%s","public_key":"%s"}' "$peer2" "$key3")
node=3; http=$http2; peer=$peer2; operator=$operator2
configure_env
export RHIZA_NODE_ID=grant-3 RHIZA_PEER_TOKEN="$token3" \
  RHIZA_PEER_TOKENS='' RHIZA_LEARNER="$learner_member" \
  GRANT_RUNTIME_BIND_ADDR="127.0.0.1:$((base+9))" \
  GRANT_REMOTE_API_SECRET=local-runtime-secret-distinct-32-bytes
"$task_dir/grant" >"$task_dir/node-3.log" 2>&1 &
pid3=$!; pids="$pids $pid3"; pid2=$pid3
i=0
until curl --max-time 3 -fsS "http://127.0.0.1:$http2/healthz" >/dev/null 2>&1; do
 check_children
 i=$((i+1)); [ "$i" -lt 90 ] || { cat "$task_dir"/node-3.log >&2; exit 1; }; sleep 1
done
learner_ready_code=$(curl --max-time 3 -sS -o /dev/null -w '%{http_code}' "http://127.0.0.1:$http2/readyz")
[ "$learner_ready_code" = 503 ] || { echo "fresh learner readyz was $learner_ready_code, want 503" >&2; exit 1; }
learner_business_code=$(curl --max-time 3 -sS -o /dev/null -w '%{http_code}' "http://127.0.0.1:$http2/api/v1/providers?consistency=strong")
[ "$learner_business_code" = 503 ] || { echo "fresh learner business API was $learner_business_code, want 503" >&2; exit 1; }
learner_admin_code=$(curl --max-time 3 -sS -o /dev/null -w '%{http_code}' \
  -H 'X-Grant-Master-Key: local-master-secret' -H 'Content-Type: application/json' \
  --data '{"id":"learner-must-not-write","name":"learner-must-not-write","active":true}' \
  "http://127.0.0.1:$http2/api/v1/admin/providers")
[ "$learner_admin_code" = 503 ] || { echo "fresh learner admin API was $learner_admin_code, want 503" >&2; exit 1; }
for path in healthz read-fence; do
  private_code=$(curl --max-time 3 -sS -o /dev/null -w '%{http_code}' "http://127.0.0.1:$((base+9))/$path")
  [ "$private_code" = 503 ] || { echo "pending learner private /$path was $private_code, want 503" >&2; exit 1; }
done
# Cancellation during admission must exit cleanly and preserve the learner WAL.
kill "$pid3"
wait "$pid3" || { echo 'pending learner failed graceful shutdown' >&2; exit 1; }
pids="$pid0 $pid1"
"$task_dir/grant" >"$task_dir/node-3.log" 2>&1 &
pid3=$!; pid2=$pid3; pids="$pids $pid3"
i=0
until curl --max-time 3 -fsS "http://127.0.0.1:$http2/healthz" >/dev/null 2>&1; do
  check_children
  i=$((i+1)); [ "$i" -lt 90 ] || { cat "$task_dir/node-3.log" >&2; exit 1; }; sleep 1
done
printf '%s\n' 'Pending learner private gates and graceful restart passed'
learner_status_code=$(curl --max-time 3 -sS -o "$task_dir/learner-membership-status.json" -w '%{http_code}' \
  -H "Authorization: Bearer $admin_token" "http://127.0.0.1:$operator2/membership/status") || {
  echo 'learner membership status request failed before admission' >&2
  cat "$task_dir/learner-membership-status.json" >&2
  exit 1
}
i=0
while [ "$learner_status_code" = 503 ]; do
  check_children
  i=$((i+1)); [ "$i" -lt 90 ] || {
    echo 'learner membership status remained unavailable after restart (HTTP 503)' >&2
    cat "$task_dir/learner-membership-status.json" >&2
    exit 1
  }
  sleep 1
  learner_status_code=$(curl --max-time 3 -sS -o "$task_dir/learner-membership-status.json" -w '%{http_code}' \
    -H "Authorization: Bearer $admin_token" "http://127.0.0.1:$operator2/membership/status") || {
    echo 'learner membership status request failed during startup' >&2
    cat "$task_dir/learner-membership-status.json" >&2
    exit 1
  }
done
[ "$learner_status_code" = 200 ] || {
  echo "learner membership status returned HTTP $learner_status_code before admission" >&2
  cat "$task_dir/learner-membership-status.json" >&2
  exit 1
}
learner_status=$(cat "$task_dir/learner-membership-status.json")
printf '%s' "$learner_status" | jq -e --arg cluster "$cluster_id" \
  '.node_id == "grant-3" and .cluster_id == $cluster and .voting == false' >/dev/null || {
  echo 'fresh learner membership identity or non-voting state was invalid' >&2
  cat "$task_dir/learner-membership-status.json" >&2
  exit 1
}
learner_wal=$(printf '%s' "$learner_status" | jq -r '.wal_identity')
[ -n "$learner_wal" ] && [ "$learner_wal" != null ] || { echo 'fresh learner did not expose a WAL identity' >&2; exit 1; }
add_body=$(jq -cn --arg op add-grant-3 --arg cluster "$cluster_id" --argjson config "$expected_config" \
  --arg id grant-3 --arg peer "quic://127.0.0.1:$peer2" --arg key "$key3" --arg wal "$learner_wal" \
  '{operation_id:$op,cluster_id:$cluster,expected_config_id:$config,add:{node_id:$id,peer_url:$peer,public_key:$key,wal_identity:$wal}}')
curl --fail-with-body -sS -H "Authorization: Bearer $admin_token" -H 'Content-Type: application/json' \
  --data "$add_body" "http://127.0.0.1:$operator0/membership/change" >"$task_dir/learner-added.json"
i=0
while :; do
  learner_status_code=$(curl --max-time 3 -sS -o "$task_dir/learner-membership-status.json" -w '%{http_code}' \
    -H "Authorization: Bearer $admin_token" "http://127.0.0.1:$operator2/membership/status") || {
    echo 'learner membership status request failed during admission' >&2
    cat "$task_dir/learner-membership-status.json" >&2
    exit 1
  }
  if [ "$learner_status_code" = 503 ]; then
    check_children
    i=$((i+1)); [ "$i" -lt 90 ] || {
      echo 'learner membership status remained unavailable during admission (HTTP 503)' >&2
      cat "$task_dir/learner-membership-status.json" >&2
      exit 1
    }
    sleep 1
    continue
  fi
  [ "$learner_status_code" = 200 ] || {
    echo "learner membership status returned HTTP $learner_status_code during admission" >&2
    cat "$task_dir/learner-membership-status.json" >&2
    exit 1
  }
  learner_status=$(cat "$task_dir/learner-membership-status.json")
  printf '%s' "$learner_status" | jq -e --arg cluster "$cluster_id" --arg wal "$learner_wal" \
    '.node_id == "grant-3" and .cluster_id == $cluster and .wal_identity == $wal' >/dev/null || {
    echo 'learner membership identity or WAL changed during admission' >&2
    cat "$task_dir/learner-membership-status.json" >&2
    exit 1
  }
  if printf '%s' "$learner_status" | jq -e '.voting == true' >/dev/null 2>&1; then break; fi
  i=$((i+1)); [ "$i" -lt 90 ] || { echo 'learner admission did not promote the fresh voter' >&2; exit 1; }; sleep 1
done
# Finish application initialization before beginning the next membership change.
i=0
until curl --max-time 3 -fsS "http://127.0.0.1:$http2/readyz" >/dev/null 2>&1; do
  check_children
  i=$((i+1)); [ "$i" -lt 90 ] || { cat "$task_dir/node-3.log" >&2; exit 1; }; sleep 1
done
remove_body=$(jq -cn --arg op remove-grant-2 --arg cluster "$cluster_id" --argjson config "$((expected_config+1))" \
  --arg wal "$old2_wal" '{operation_id:$op,cluster_id:$cluster,expected_config_id:$config,remove:"grant-2",fence:{node_id:"grant-2",wal_identity:$wal,workload_uid:"grant-go-three-voter",confirmed:true,evidence:"owned local process terminated and empty replacement rejected"}}')
curl --fail-with-body -sS -H "Authorization: Bearer $admin_token" -H 'Content-Type: application/json' \
  --data "$remove_body" "http://127.0.0.1:$operator0/membership/change" >"$task_dir/learner-promoted.json"
curl -fsS -H "Authorization: Bearer $admin_token" "http://127.0.0.1:$operator0/membership/status" \
  | jq -e '.voters | index("grant-3") and (index("grant-2") | not)' >/dev/null
i=0
until curl --max-time 3 -fsS "http://127.0.0.1:$http2/readyz" >/dev/null 2>&1; do
  check_children
  i=$((i+1)); [ "$i" -lt 90 ] || { cat "$task_dir"/node-3.log >&2; exit 1; }; sleep 1
done
peer_tokens=$(printf '{"grant-0":"%s","grant-1":"%s","grant-3":"%s"}' "$token0" "$token1" "$token3")
legacy_members=$(printf '[{"node_id":"grant-0","peer_url":"quic://127.0.0.1:%s","token":"%s"},{"node_id":"grant-1","peer_url":"quic://127.0.0.1:%s","token":"%s"},{"node_id":"grant-3","peer_url":"quic://127.0.0.1:%s","token":"%s"}]' "$peer0" "$token0" "$peer1" "$token1" "$peer2" "$token3")
members=$(printf '[{"node_id":"grant-0","peer_url":"quic://127.0.0.1:%s","public_key":"%s"},{"node_id":"grant-1","peer_url":"quic://127.0.0.1:%s","public_key":"%s"},{"node_id":"grant-3","peer_url":"quic://127.0.0.1:%s","public_key":"%s"}]' "$peer0" "$key0" "$peer1" "$key1" "$peer2" "$key3")
curl -fsS -H 'X-Grant-Master-Key: local-master-secret' -H 'Content-Type: application/json' \
 --data '{"id":"cold-recovery-latest","name":"Latest before cold loss","active":true}' \
 "http://127.0.0.1:$http0/api/v1/admin/providers" >/dev/null
curl -fsS "http://127.0.0.1:$http2/api/v1/providers?consistency=strong" \
 | jq -e 'any(.[]; .id=="cold-recovery-latest")' >/dev/null
printf '%s\n' 'Fresh learner catch-up, authenticated membership replacement and promoted-voter write passed'

# Capture the logical restore source while the promoted three-voter generation is live.
curl -fsS -X POST -H 'X-Grant-Master-Key: local-master-secret' \
  "http://127.0.0.1:$http0/ops/backups" >/dev/null
set -- "$node_root"/backups-0/grant-*.json
[ "$#" -eq 1 ] && [ -f "$1" ] || { echo 'expected one verified source snapshot' >&2; exit 1; }
restore_snapshot=$1

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
printf '%s\n' 'Quorum loss closed strong readiness while preserving process safety'

# Restore a populated logical snapshot into a distinct, wholly fresh cluster.
# The original prefix and directories remain intact throughout this fixture.
for pid in $pids; do kill "$pid"; done
for pid in $pids; do wait "$pid" || true; done
pids=''
i=0
while :; do
  busy=0
  for port in "$http0" "$http1" "$http2" "$operator0" "$operator1" "$operator2"; do
    if curl --max-time 1 -sS "http://127.0.0.1:$port/healthz" >/dev/null 2>&1; then busy=1; fi
  done
  [ "$busy" -eq 0 ] && break
  i=$((i+1)); [ "$i" -lt 30 ] || { echo 'owned fixture listeners did not close before logical restore' >&2; exit 1; }; sleep 1
done
cluster_id="$container-restored"
node_root="$task_dir/restored"
mkdir "$node_root"
legacy_mode=1
legacy_members=$(printf '[{"node_id":"grant-0","peer_url":"quic://127.0.0.1:%s","token":"%s"},{"node_id":"grant-1","peer_url":"quic://127.0.0.1:%s","token":"%s"},{"node_id":"grant-3","peer_url":"quic://127.0.0.1:%s","token":"%s"}]' "$peer0" "$token0" "$peer1" "$token1" "$peer2" "$token3")
node=0; http=$http0; peer=$peer0; operator=$operator0
configure_env
"$task_dir/grant" migrate-prepare-cluster --snapshot "$restore_snapshot"
start 0 "$http0" "$peer0" "$operator0"; pid0=$started_pid
start 1 "$http1" "$peer1" "$operator1"; pid1=$started_pid
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
start 3 "$http2" "$peer2" "$operator2"; pid2=$started_pid
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
node=0; http=$http0; peer=$peer0; operator=$operator0
original_members=$members
members=$(printf '%s' "$members" | jq -c '.[2].node_id="different-admission-voter"')
original_legacy_members=$legacy_members
legacy_members=$(printf '%s' "$legacy_members" | jq -c '.[2].node_id="different-admission-voter"')
configure_env
if "$task_dir/grant" migrate-prepare-cluster --snapshot "$restore_snapshot" >"$task_dir/reprepare.log" 2>&1; then
 echo 'restore replaced existing target admission with different membership' >&2; exit 1
fi
members=$original_members
legacy_members=$original_legacy_members
kill "$pid2"; wait "$pid2" || true
pids="$pid0 $pid1"
for pid in "$pid0" "$pid1"; do kill -0 "$pid"; done
curl --retry 12 --retry-all-errors --retry-delay 1 --max-time 30 -fsS \
 -H 'X-Grant-Master-Key: local-master-secret' -H 'Content-Type: application/json' \
 --data '{"id":"restore-recovery","name":"Restored quorum recovery","active":true}' \
 "http://127.0.0.1:$http0/api/v1/admin/providers" >/dev/null
start 3 "$http2" "$peer2" "$operator2"; pid2=$started_pid
i=0
until curl --max-time 2 -fsS "http://127.0.0.1:$http2/readyz" >/dev/null 2>&1; do
 check_children
 i=$((i+1)); [ "$i" -lt 90 ] || exit 1; sleep 1
done
check_children
curl -fsS "http://127.0.0.1:$http2/api/v1/providers?consistency=strong" \
 | jq -e 'any(.[]; .id=="restore-recovery")' >/dev/null
printf '%s\n' 'Fresh three-voter snapshot restore, closed admission readiness, occupied-target rejection and recovery passed'
