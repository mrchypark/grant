#!/bin/sh
set -eu
endpoint=$(docker context inspect --format '{{.Endpoints.docker.Host}}')
case "$endpoint" in unix://*) ;; *) echo 'container smoke requires a local Docker context' >&2; exit 1 ;; esac
case "$(docker info --format '{{.Architecture}}')" in
  amd64|x86_64) ;;
  *) echo 'container smoke requires a native AMD64 Docker daemon; emulation is not deployment proof' >&2; exit 1 ;;
esac
[ "$(docker image inspect "${IMAGE_NAME:-grant:container-verify}" --format '{{.Architecture}}')" = amd64 ] \
 || { echo 'container smoke requires an AMD64 candidate image' >&2; exit 1; }
container="grant-container-smoke-$$"
cleanup() { code=$?; if [ "$code" -ne 0 ]; then docker logs "$container" >&2 || true; fi; docker rm -f "$container" >/dev/null 2>&1 || true; }
trap cleanup EXIT HUP INT TERM
docker run -d --name "$container" --platform linux/amd64 \
 --user 10001:10001 --read-only --cap-drop ALL --security-opt no-new-privileges \
 --memory 512m --cpus 1 --pids-limit 256 \
 --tmpfs /data:rw,uid=10001,gid=10001,size=512m --tmpfs /tmp:rw,uid=10001,gid=10001,size=64m \
 -p 127.0.0.1::8080 \
 -e GRANT_MASTER_KEY=local-smoke-master-secret -e GRANT_APP_AUTH_KEY=local-smoke-app-secret \
 -e GRANT_AUTH_PROVIDER=rauthy -e GRANT_RAUTHY_CLIENT_ID=local-smoke \
 -e GRANT_RAUTHY_CLIENT_SECRET=local-smoke-rauthy-secret -e GRANT_RAUTHY_CLIENT_READ_API_KEY=local-smoke-read-key \
 -e GRANT_NODE_ID=smoke -e GRANT_CLUSTER_ID="$container" -e GRANT_PEER_ADDR=127.0.0.1:8100 \
 -e GRANT_RUNTIME_MODE=embedded -e GRANT_LOCAL_NODE_ROLE=voter -e GRANT_READ_CONSISTENCY=strong \
 -e RHIZA_OBJSTORE_PROVIDER=filesystem -e RHIZA_OBJSTORE_DIR=/data/objects \
 "${IMAGE_NAME:-grant:container-verify}" >/dev/null
port=$(docker inspect "$container" | jq -r '.[0].NetworkSettings.Ports["8080/tcp"][0].HostPort')
i=0
until curl --max-time 4 -fsS "http://127.0.0.1:$port/readyz" >/dev/null 2>&1; do
 [ "$(docker inspect -f '{{.State.Running}}' "$container")" = true ] || exit 1
 i=$((i+1)); [ "$i" -lt 120 ] || exit 1; sleep 1
done
curl --max-time 10 -fsS "http://127.0.0.1:$port/healthz" | jq -e '.status=="ok"' >/dev/null
curl --max-time 4 -fsS "http://127.0.0.1:$port/readyz" | jq -e '.writeReady==true' >/dev/null
curl --max-time 10 -fsS -H 'X-Grant-Master-Key: local-smoke-master-secret' -H 'Content-Type: application/json' \
 --data '{"id":"image-smoke","name":"Image smoke","active":true}' "http://127.0.0.1:$port/api/v1/admin/providers" >/dev/null
curl --max-time 10 -fsS "http://127.0.0.1:$port/api/v1/providers?consistency=strong" | jq -e 'any(.[];.id=="image-smoke")' >/dev/null
code=$(curl --max-time 10 -sS -o /dev/null -w '%{http_code}' -H 'Content-Type: application/json' --data '{"id":"reject","name":"Reject","active":true}' "http://127.0.0.1:$port/api/v1/admin/providers")
[ "$code" = 401 ]
docker inspect "$container" | jq -e '.[0] | .Config.User=="10001:10001" and .HostConfig.ReadonlyRootfs==true and .State.OOMKilled==false' >/dev/null
printf '%s\n' 'Container production security/resource image startup, strong readiness, write/read and auth-negative smoke passed'
