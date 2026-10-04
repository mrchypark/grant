#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
TMP_DIR="$(mktemp -d "${TMPDIR:-/tmp}/verify-vind-bootstrap.XXXXXX")"
MOCK_LOG="${TMP_DIR}/docker.log"
RUNTIME_REFS="${TMP_DIR}/runtime-refs"
IMPORT_COUNT="${TMP_DIR}/import-count"
TEST_TOKEN='fixture-token-must-not-leak'

cleanup() {
  rm -rf "${TMP_DIR}"
}
trap cleanup EXIT

fail() {
  echo "$*" >&2
  exit 1
}

# shellcheck disable=SC2034 # Referenced by the sourced production functions.
CLUSTER_NAME=grant-ci
# shellcheck source=scripts/vind-image-helpers.sh
. "${ROOT_DIR}/scripts/vind-image-helpers.sh"

: >"${MOCK_LOG}"
: >"${RUNTIME_REFS}"
printf '0\n' >"${IMPORT_COUNT}"

signal_import_subshell() {
  local signal_name="$1"

  if [[ -n "${BASHPID:-}" ]]; then
    [[ "${BASHPID}" != "$$" ]] \
      || fail 'signal fixture did not run in the import subshell'
    kill -s "${signal_name}" "${BASHPID}"
    return
  fi

  # Bash 3.2 has no BASHPID. A direct child sees the import subshell's real
  # process ID as PPID, which is the same process newer Bash exposes as BASHPID.
  sh -c '
    [ "$PPID" != "$1" ] || exit 98
    kill -s "$2" "$PPID"
  ' signal-import "$$" "${signal_name}"
}

docker() {
  {
    printf 'docker'
    printf ' <%s>' "$@"
    printf '\n'
  } >>"${MOCK_LOG}"

  case "${1:-} ${2:-} ${3:-} ${4:-} ${5:-} ${6:-}" in
    "inspect vcluster.cp.grant-ci "*)
      return 0
      ;;
    "save "*)
      while [[ "$#" -gt 0 ]]; do
        if [[ "$1" == "-o" ]]; then
          printf 'docker-archive-fixture\n' >"$2"
          return 0
        fi
        shift
      done
      return 90
      ;;
    "cp "*)
      if [[ "${MOCK_TRANSFER_FAIL:-0}" == "1" ]]; then
        echo 'container archive transfer failed' >&2
        return 43
      fi
      return 0
      ;;
    "exec vcluster.cp.grant-ci ctr -n k8s.io images"*)
      if [[ "${7:-}" == "ls" && "${8:-}" == "-q" ]]; then
        cat "${RUNTIME_REFS}"
        if [[ "${MOCK_FIRST_LIST_FAILURE:-0}" == "1" && ! -e "${TMP_DIR}/first-list-seen" ]]; then
          : >"${TMP_DIR}/first-list-seen"
          return 19
        fi
        return 0
      fi
      if [[ -n "${MOCK_IMPORT_SIGNAL:-}" ]]; then
        signal_import_subshell "${MOCK_IMPORT_SIGNAL}"
        return 99
      fi
      if [[ "${MOCK_IMPORT_FAIL:-0}" == "1" ]]; then
        echo "Authorization: Bearer ${TEST_TOKEN}" >&2
        echo "transport https://fixture:${TEST_TOKEN}@registry.example.test/v2/?token=${TEST_TOKEN}" >&2
        echo 'containerd fixture import exploded' >&2
        return 42
      fi
      printf 'docker.io/library/grant:local\n' >>"${RUNTIME_REFS}"
      printf '%s\n' "$(( $(cat "${IMPORT_COUNT}") + 1 ))" >"${IMPORT_COUNT}"
      return 0
      ;;
    "exec vcluster.cp.grant-ci rm -f "*)
      return 0
      ;;
  esac

  return 92
}

digest_ref='ghcr.io/conalog/grant-rauthy-launcher:rauthy-0.36.0@sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa'
# shellcheck disable=SC2034 # Referenced by the sourced production function.
RAUTHY_IMAGE="${digest_ref}"
ensure_rauthy_runtime_image
[ ! -s "${MOCK_LOG}" ] \
  || fail 'digest-qualified Rauthy image was preloaded instead of left for authenticated kubelet pull'

printf 'docker.io/library/grant:local\n' >"${RUNTIME_REFS}"
MOCK_FIRST_LIST_FAILURE=1 ensure_runtime_image 'grant:local'

grep -Eq '^docker <save> <grant:local> <-o> <.+>$' "${MOCK_LOG}" \
  || fail 'tag-qualified local Grant image was not archived for runtime preload'
grep -Eq '^docker <cp> <.+> <vcluster.cp.grant-ci:/home/grant-ci.image-import.[0-9]+.tar>$' "${MOCK_LOG}" \
  || fail 'archive was not copied to a container-visible path'
grep -Eq '^docker <exec> <vcluster.cp.grant-ci> <ctr> <-n> <k8s.io> <images> <import> </home/grant-ci.image-import.[0-9]+.tar>$' "${MOCK_LOG}" \
  || fail 'archive import did not use ctr in the Kubernetes containerd namespace'
grep -Eq '^docker <exec> <vcluster.cp.grant-ci> <rm> <-f> </home/grant-ci.image-import.[0-9]+.tar>$' "${MOCK_LOG}" \
  || fail 'remote archive was not removed after import'
[ "$(cat "${IMPORT_COUNT}")" -eq 1 ] \
  || fail 'a failed runtime-list producer was hidden by the grep result despite pipefail'

pull_secret_line="$(grep -n '^apply_rauthy_registry_pull_secret$' "${ROOT_DIR}/scripts/vind-bootstrap.sh" | cut -d: -f1)"
workload_line="$(grep -n '^kubectl apply -k ' "${ROOT_DIR}/scripts/vind-bootstrap.sh" | cut -d: -f1)"
[[ -n "${pull_secret_line}" && -n "${workload_line}" && "${pull_secret_line}" -lt "${workload_line}" ]] \
  || fail 'Rauthy imagePullSecret is not applied before the workload'

grant_image_override_line="$(grep -n 'set image statefulset/grant-voter' "${ROOT_DIR}/scripts/vind-bootstrap.sh" | cut -d: -f1)"
[[ -n "${grant_image_override_line}" && "${workload_line}" -lt "${grant_image_override_line}" ]] \
  || fail 'selected IMAGE_NAME is not applied to the Grant StatefulSet'
grep -q 'applied_grant_image=' "${ROOT_DIR}/scripts/vind-bootstrap.sh" \
  || fail 'Grant image override is not verified after application'

# Render the actual overlay, including bucket initialization on the same volume.
object_store_image='docker.io/versity/versitygw:v1.8.0@sha256:30292fc2eeacc67a36993b01f7a7a5e3361a19cced0e80c1d71cfa2a4b0a2499'
kubectl kustomize "${ROOT_DIR}/deploy/overlays/ephemeral" >"${TMP_DIR}/local-overlay.yaml"
[[ "$(grep -Fc "image: ${object_store_image}" "${TMP_DIR}/local-overlay.yaml")" -eq 2 ]] \
  || fail 'gateway and bucket initializer must use the same immutable Versity image'
grep -Fq "object_store_image=${object_store_image}" "${ROOT_DIR}/scripts/verify-go-three-voter.sh" \
  || fail 'native and Kubernetes fixtures must use the same immutable Versity image'
if grep -Eq 'quay.io/minio/|grant-object-store-init' "${TMP_DIR}/local-overlay.yaml"; then
  fail 'rendered local overlay still contacts the unavailable MinIO registry'
fi
grep -q 'mkdir -p /data/grant-rhiza' "${TMP_DIR}/local-overlay.yaml" \
  || fail 'the local POSIX bucket is not created before the gateway starts'
object_store_pull_line="$(grep -n 'docker pull --platform' "${ROOT_DIR}/scripts/vind-bootstrap.sh" | cut -d: -f1)"
[[ -n "${object_store_pull_line}" && "${object_store_pull_line}" -lt "${workload_line}" ]] \
  || fail 'object-store public image availability is not checked before workload creation'
grep -q 'rollout status deployment/grant-object-store --timeout=120s' "${ROOT_DIR}/scripts/vind-bootstrap.sh" \
  || fail 'bootstrap must fail promptly when the object store cannot start'

if grep -Eq 'export GRANT_(MASTER_KEY|APP_AUTH_KEY)=\$\{GRANT_(MASTER_KEY|APP_AUTH_KEY)\}' \
  "${ROOT_DIR}/scripts/vind-bootstrap.sh"; then
  fail 'vind bootstrap completion guidance exposes a Grant secret value'
fi

: >"${MOCK_LOG}"
if MOCK_TRANSFER_FAIL=1 import_image_with_ctr_vcluster 'grant:local' >"${TMP_DIR}/transfer-failure.log" 2>&1; then
  fail 'archive transfer failure unexpectedly succeeded'
fi
grep -q 'during containerd archive transfer' "${TMP_DIR}/transfer-failure.log" \
  || fail 'transfer failure did not identify the failed runtime step'
if grep -q '<images> <import>' "${MOCK_LOG}"; then
  fail 'failed archive transfer was followed by an import'
fi
grep -q '<rm> <-f> </home/' "${MOCK_LOG}" \
  || fail 'failed transfer did not attempt remote archive cleanup'

: >"${MOCK_LOG}"
: >"${RUNTIME_REFS}"
MOCK_IMPORT_FAIL=1
failure_log="${TMP_DIR}/failure.log"
if import_image_with_ctr_vcluster 'grant:local' >"${failure_log}" 2>&1; then
  fail 'containerd import failure unexpectedly succeeded'
fi
grep -q 'during containerd archive import' "${failure_log}" \
  || fail 'import failure did not identify the failed runtime step'
grep -q 'containerd fixture import exploded' "${failure_log}" \
  || fail 'import failure hid the actionable containerd diagnostic'
grep -q '\[redacted\]' "${failure_log}" \
  || fail 'import diagnostics did not mark redacted credentials'
if grep -Fq "${TEST_TOKEN}" "${failure_log}"; then
  fail 'import diagnostics leaked a credential'
fi

unset MOCK_IMPORT_FAIL
for signal_case in HUP:129 INT:130 TERM:143; do
  signal_name="${signal_case%%:*}"
  expected_status="${signal_case##*:}"
  signal_tmp="${TMP_DIR}/signal-${signal_name}"
  signal_log="${TMP_DIR}/signal-${signal_name}.log"
  mkdir -p "${signal_tmp}"

  set +e
  TMPDIR="${signal_tmp}" MOCK_IMPORT_SIGNAL="${signal_name}" \
    import_image_with_ctr_vcluster 'grant:local' >"${signal_log}" 2>&1
  signal_status=$?
  set -e

  [[ "${signal_status}" -eq "${expected_status}" ]] \
    || fail "${signal_name} import cleanup exited ${signal_status}, expected ${expected_status}"
  if find "${signal_tmp}" -type f -print | grep -q .; then
    fail "${signal_name} import cleanup left a temporary archive or diagnostic log"
  fi
done
unset MOCK_IMPORT_SIGNAL

echo 'vind bootstrap image import verified'
