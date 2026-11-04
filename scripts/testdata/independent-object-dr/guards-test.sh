#!/bin/sh
set -eu
umask 077
here=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
. "$here/guards.sh"
fixture=$(mktemp -d "${TMPDIR:-/tmp}/grant-guard-test.XXXXXX")
pid=
cleanup() { [ -z "$pid" ] || { kill "$pid" 2>/dev/null || :; wait "$pid" 2>/dev/null || :; }; find "$fixture" -depth -delete; }
trap cleanup EXIT
header='Filesystem 1024-blocks Used Available Capacity Mounted on'
printf '%s\n/dev/test 9999999 1 4194304 1%% /\n' "$header" | independent_df_valid
for row in '/dev/test 9999999 1 4194303 1% /' '/dev/test 9999999 1 invalid 1% /' ''; do
 if printf '%s\n%s\n' "$header" "$row" | independent_df_valid; then echo 'FAIL invalid/low/empty df accepted'; exit 1; fi
done
if printf '' | independent_df_valid; then echo 'FAIL empty df accepted'; exit 1; fi
if (df() { printf '%s\n/dev/test 9999999 1 9999999 1%% /\n' "$header"; return 1; }; independent_space /); then echo 'FAIL failing df accepted'; exit 1; fi
independent_unix_endpoint unix:///var/run/docker.sock
for endpoint in tcp://example.test:2375 ssh://example.test npipe://pipe ''; do
 if independent_unix_endpoint "$endpoint"; then echo 'FAIL remote endpoint accepted'; exit 1; fi
done
cp /bin/sleep "$fixture/grant"
"$fixture/grant" 60 & pid=$!
start=$(ps -p "$pid" -o lstart= | sed 's/^[[:space:]]*//')
args=$(ps -p "$pid" -o args= | sed 's/^[[:space:]]*//')
printf '%s\n%s\n%s\n%s\n' "$pid" "$fixture/grant" stale-start "$args" > "$fixture/stale.receipt"
independent_reap_receipt "$fixture/stale.receipt" "$fixture"
kill -0 "$pid" # Stale identity must not signal the live process.
printf '%s\n%s\n%s\n%s\n' "$pid" "$fixture/not-grant" "$start" "$args" > "$fixture/foreign.receipt"
rc=0; independent_reap_receipt "$fixture/foreign.receipt" "$fixture" || rc=$?
[ "$rc" -eq 2 ]; kill -0 "$pid"
printf '%s\n%s\n%s\n%s\n' "$pid" "$fixture/grant" "$start" "$args" > "$fixture/owned.receipt"
rc=0; independent_reap_receipt "$fixture/owned.receipt" "$fixture" || rc=$?
[ "$rc" -eq 1 ] # Cleanup was required, so caller must fail the verification run.
wait "$pid" 2>/dev/null || :
if kill -0 "$pid" 2>/dev/null; then echo 'FAIL owned process remains'; exit 1; fi
pid=
independent_reap_receipt "$fixture/owned.receipt" "$fixture"
echo 'PASS disk, local endpoint, stale/foreign PID protection, owned native TERM/KILL cleanup and absence'
# Lost-CID discovery is read-only; foreign or ambiguous identities never reach rm.
mock_labels=; mock_names=; mock_error=0
mock_cid=$(printf '%064d' 0)
docker_local() {
 [ "$1" = ps ] || { echo 'FAIL reconciliation attempted mutation' >&2; return 9; }
 [ "$mock_error" -eq 0 ] || return 1
 for last do :; done
 case "$last" in label=*) printf '%s\n' "$mock_labels";; name=*) printf '%s\n' "$mock_names";; *) return 9;; esac
}
[ -z "$(independent_discover_container grant-owned)" ]
mock_labels=$mock_cid; mock_names=$mock_cid
[ "$(independent_discover_container grant-owned)" = "$mock_cid" ]
mock_labels=
if independent_discover_container grant-owned; then echo 'FAIL name collision accepted'; exit 1; fi
mock_labels=$mock_cid; mock_names=
if independent_discover_container grant-owned; then echo 'FAIL foreign label accepted'; exit 1; fi
mock_labels="$mock_cid
$mock_cid"; mock_names=$mock_cid
if independent_discover_container grant-owned; then echo 'FAIL ambiguous create accepted'; exit 1; fi
mock_error=1
if independent_discover_container grant-owned; then echo 'FAIL daemon error accepted'; exit 1; fi
echo 'PASS unknown-create reconciliation, collision/ambiguity protection, read-only discovery and daemon error guard'
independent_native_metadata default unix:///var/run/docker.sock
independent_native_metadata desktop-linux unix:///Users/test/.docker/run/docker.sock
for pair in 'dory unix:///Users/test/.dory/dory.sock' 'dory unix:///var/run/docker.sock' 'default tcp://127.0.0.1:2375' 'bridge unix:///var/run/docker.sock' 'default unix:///tmp/unknown.sock'; do
 set -- $pair
 if independent_native_metadata "$1" "$2"; then echo 'FAIL proxy metadata accepted'; exit 1; fi
done
independent_native_server 'Docker Engine - Community' 'Ubuntu 24.04'
independent_native_server 'Docker Desktop 4.0' 'Docker Desktop'
if independent_native_server 'Dory bridge' 'Ubuntu 24.04'; then echo 'FAIL proxy server identity accepted'; exit 1; fi
if independent_native_server 'Docker Engine - Community' unknown; then echo 'FAIL unknown server identity accepted'; exit 1; fi
echo 'PASS known default/Desktop native metadata and identity; reject Dory/bridge/remote/unknown before mutations'
# Exercise the real entrypoint against metadata-only mocks. Any daemon command
# is a regression: the stub fails it, and no real Docker binary is reachable.
mkdir "$fixture/bin"
cat > "$fixture/bin/docker" <<'MOCK'
#!/bin/sh
printf '%s\n' "$*" >> "$GRANT_MOCK_DOCKER_CALLS"
case "$*" in
 'context show') echo dory;;
 'context inspect --format {{.Endpoints.docker.Host}}') echo unix:///Users/mock/.dory/dory.sock;;
 *) echo 'unexpected daemon command' >&2; exit 99;;
esac
MOCK
chmod +x "$fixture/bin/docker"
runner="$here/../../verify-independent-object-dr.sh"
if env -u DOCKER_HOST -u DOCKER_CONTEXT PATH="$fixture/bin:$PATH" GRANT_MOCK_DOCKER_CALLS="$fixture/calls" /bin/sh "$runner" > "$fixture/reject.log" 2>&1; then echo 'FAIL real runner accepted Dory mock'; exit 1; fi
[ "$(wc -l < "$fixture/calls" | tr -d ' ')" -eq 2 ]
grep -q 'bridge/proxy or unknown Docker context' "$fixture/reject.log"
echo 'PASS real entrypoint rejects mocked Dory after metadata reads only; no daemon commands'
