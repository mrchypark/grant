#!/bin/sh
set -eu
image=${IMAGE_NAME:-grant:container-verify}
durability=${OBJECT_STORE_DURABILITY:-before-ack}
case "$durability" in
 async|before-ack) ;; *) echo 'durability must be async or before-ack' >&2; exit 1 ;;
esac
log_dir=${LOG_DIR:-$(mktemp -d "${TMPDIR:-/tmp}/grant-standalone-container.XXXXXX")}
mkdir -p "$log_dir"
printf 'Standalone container logs: %s\n' "$log_dir"
case "$(docker context inspect --format '{{.Endpoints.docker.Host}}')" in
 unix://*) ;; *) echo 'requires local Docker context' >&2; exit 1 ;;
esac
case "$(docker info --format '{{.Architecture}}')" in
 amd64|x86_64) ;; *) echo 'requires native AMD64 daemon; emulation is not deployment proof' >&2; exit 1 ;;
esac
[ "$(docker image inspect "$image" --format '{{.Architecture}}')" = amd64 ]
# Track only created container IDs and volumes whose owner label matches.
owner="grant-standalone-$(date +%s)-$$"
containers=''
volumes=''
cleanup() {
 code=$?
 trap - EXIT HUP INT TERM
 for id in $containers; do
  if [ "$(docker container inspect -f '{{index .Config.Labels "grant.verify.owner"}}' "$id" 2>/dev/null)" != "$owner" ]; then
   echo "refusing cleanup: container $id ownership unverified" >&2; code=1; continue
  fi
  docker logs "$id" > "$log_dir/$id.log" 2>&1 || true
  if ! docker rm -f "$id" >/dev/null 2>&1; then echo "cleanup failed: container $id" >&2; code=1; fi
  if docker container inspect "$id" >/dev/null 2>&1; then echo "container remains: $id" >&2; code=1; fi
 done
 for name in $volumes; do
  if [ "$(docker volume inspect -f '{{index .Labels "grant.verify.owner"}}' "$name" 2>/dev/null)" != "$owner" ]; then
   echo "refusing cleanup: volume $name ownership unverified" >&2; code=1; continue
  fi
  if ! docker volume rm "$name" >/dev/null 2>&1; then echo "cleanup failed: volume $name" >&2; code=1; fi
  if docker volume inspect "$name" >/dev/null 2>&1; then echo "volume remains: $name" >&2; code=1; fi
 done
 exit "$code"
}
trap cleanup EXIT
trap 'exit 130' HUP INT TERM
new_volume() {
 name=$1
 if docker volume inspect "$name" >/dev/null 2>&1; then echo "refusing existing volume: $name" >&2; exit 1; fi
 docker volume create --label "grant.verify.owner=$owner" "$name" >/dev/null
 if [ "$(docker volume inspect -f '{{index .Labels "grant.verify.owner"}}' "$name")" != "$owner" ]; then
  echo "refusing volume: $name ownership unverified" >&2; exit 1
 fi
 volumes="$volumes $name"
}
archive="$owner-archive"
new_volume "$archive"
# Root-only setup reproduces fsGroup ownership; init and main always run as 10001.
# Rhiza's owner anchor lives beside the archive root, inside the writable volume.
prepare() {
 data=$1
 new_volume "$data"
 name="$data-permissions"
 cid=$(docker create --name "$name" --label "grant.verify.owner=$owner" --network none --read-only --cap-drop ALL --cap-add CHOWN \
  --security-opt no-new-privileges --memory 64m --cpus 1 --pids-limit 32 --user 0:0 \
  -v "$data:/data" -v "$archive:/archive" --entrypoint /bin/sh "$image" \
  -c 'mkdir -p /archive/root && chown 10001:10001 /data /archive /archive/root')
 containers="$containers $cid"
 docker start -a "$cid"
}
run_grant() {
 name=$1; data=$2; shift 2
 cid=$(docker create --name "$name" --label "grant.verify.owner=$owner" --platform linux/amd64 \
  --user 10001:10001 --read-only --cap-drop ALL --security-opt no-new-privileges \
  --memory 512m --cpus 1 --pids-limit 256 \
  --tmpfs /tmp:rw,uid=10001,gid=10001,size=64m \
  -v "$data:/var/lib/grant" -v "$archive:/archive" -p 127.0.0.1::8080 \
  -e GRANT_MASTER_KEY=local-contract-master -e GRANT_APP_AUTH_KEY=local-contract-app \
  -e GRANT_AUTH_PROVIDER=rauthy -e GRANT_RAUTHY_CLIENT_ID=local-contract \
  -e GRANT_RAUTHY_CLIENT_SECRET=local-contract-secret -e GRANT_RAUTHY_CLIENT_READ_API_KEY=local-contract-read \
  -e GRANT_RUNTIME_MODE=embedded -e GRANT_LOCAL_NODE_ROLE=voter -e GRANT_READ_CONSISTENCY=strong \
  -e GRANT_DATA_DIR=/var/lib/grant/rhiza -e GRANT_BACKUP_DIR=/var/lib/grant/backups \
  -e GRANT_NODE_ID=standalone-contract -e GRANT_CLUSTER_ID="$owner" -e GRANT_PEER_ADDR= \
  -e "GRANT_RHIZA_CONFIG_JSON={\"Local\":true,\"NodeID\":\"standalone-contract\",\"ClusterID\":\"$owner\",\"Members\":[],\"PeerAddr\":\"\"}" \
  -e RHIZA_OBJSTORE_PROVIDER=filesystem -e RHIZA_OBJSTORE_DIR=/archive/root \
  -e "RHIZA_OBJSTORE_DURABILITY=$durability" "$image" "$@")
 containers="$containers $cid"
 docker start "$cid" >/dev/null
}
init_storage() {
 name=$1; data=$2
 run_grant "$name" "$data" restore-local-storage
 name=$cid
 i=0
 while [ "$(docker inspect -f '{{.State.Running}}' "$name")" = true ]; do
  i=$((i+1)); [ "$i" -lt 120 ] || { echo 'init timed out' >&2; return 1; }; sleep 1
 done
 [ "$(docker inspect -f '{{.State.ExitCode}}' "$name")" = 0 ]
}
start_main() {
 name=$1; data=$2
 run_grant "$name" "$data"
 name=$cid
 port=$(docker inspect "$name" | jq -r '.[0].NetworkSettings.Ports["8080/tcp"][0].HostPort')
 url="http://127.0.0.1:$port"
 i=0
 until curl --max-time 4 -fsS "$url/readyz" | jq -e '.writeReady==true' >/dev/null 2>&1; do
  [ "$(docker inspect -f '{{.State.Running}}' "$name")" = true ]
  i=$((i+1)); [ "$i" -lt 120 ] || return 1; sleep 1
 done
 curl --max-time 10 -fsS "$url/healthz" | jq -e '.status=="ok"' >/dev/null
 docker inspect "$name" | jq -e '.[0] | .Config.User=="10001:10001" and .HostConfig.ReadonlyRootfs and (.State.OOMKilled|not)' >/dev/null
 main_cid=$cid
}
stop_main() {
 docker stop --time 60 "$1" >/dev/null
 [ "$(docker inspect -f '{{.State.ExitCode}}' "$1")" = 0 ]
}
read_marker() {
 curl --max-time 10 -fsS "$url/api/v1/providers?consistency=strong" | jq -e 'any(.[];.id=="standalone-durable-marker")' >/dev/null
}
original="$owner-data-original"
prepare "$original"
init_storage "$owner-init-empty" "$original"
start_main "$owner-main-original" "$original"
curl --max-time 10 -fsS -H 'X-Grant-Master-Key: local-contract-master' -H 'Content-Type: application/json' \
 --data '{"id":"standalone-durable-marker","name":"Standalone durable marker","active":true}' "$url/api/v1/admin/providers" >/dev/null
read_marker
code=$(curl --max-time 10 -sS -o /dev/null -w '%{http_code}' -H 'Content-Type: application/json' \
 --data '{"id":"reject","name":"Reject","active":true}' "$url/api/v1/admin/providers")
[ "$code" = 401 ]
stop_main "$main_cid"
# Reopen exactly the same DataDir, then model clean disk loss with a NEW volume.
init_storage "$owner-init-reuse" "$original"
start_main "$owner-main-reuse" "$original"
read_marker
stop_main "$main_cid"
fresh="$owner-data-fresh"
prepare "$fresh"
init_storage "$owner-init-restore" "$fresh"
start_main "$owner-main-restored" "$fresh"
read_marker
stop_main "$main_cid"
printf '%s\n' "Standalone $durability init/main shared DataDir, reuse and clean data-loss archive recovery passed (not crash recovery or an async losslessness guarantee)"

# Run the packaged helper without Go, networking, writable storage, or voter access.
replica_members() {
 name=$1
 cid=$(docker create --name "$name" --label "grant.verify.owner=$owner" \
  --platform linux/amd64 --network none --user 10001:10001 --read-only \
  --cap-drop ALL --security-opt no-new-privileges --memory 512m --cpus 1 --pids-limit 256 \
  -i --entrypoint /usr/local/bin/replica-members "$image")
 containers="$containers $cid"
 docker start -ai "$cid" > "$log_dir/$name.json"
 [ "$(docker inspect -f '{{.State.ExitCode}}' "$cid")" = 0 ]
 jq -e 'type == "array" and length == 1 and
  (.[0].PublicKey | type == "array" and length == 32) and
  ([.. | objects | keys[] | ascii_downcase | select(. == "token" or . == "peertoken")] | length == 0)' \
  "$log_dir/$name.json" >/dev/null
 ! grep -F 'fixture-private-token' "$log_dir/$name.json"
}
replica_members "$owner-members-token" <<'JSON'
{"ClusterID":"image-contract","Members":[{"node_id":"n1","peer_url":"quic://n1:8100","log_url":"http://n1:8081/log","wal_identity":"wal-1","token":"fixture-private-token"}]}
JSON
key_hex=$(jq -r '.[0].PublicKey[]' "$log_dir/$owner-members-token.json" | awk '{printf "%02x", $1}')
key_base64=$(printf '%s' "$key_hex" | xxd -r -p | base64 | tr -d '\n')
jq --arg key "$key_base64" '{ClusterID:"image-contract",Members:[{node_id:"n1",peer_url:"quic://n1:8100",log_url:"http://n1:8081/log",wal_identity:"wal-1",public_key:$key}]}' \
 "$log_dir/$owner-members-token.json" > "$log_dir/members-public-input.json"
replica_members "$owner-members-public" < "$log_dir/members-public-input.json"
cmp "$log_dir/$owner-members-token.json" "$log_dir/$owner-members-public.json"
printf '%s\n' 'Packaged replica-members stdin/stdout and public-only output contract passed'
