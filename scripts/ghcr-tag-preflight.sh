#!/bin/sh
set -eu

: "${SOURCE_SHA:?SOURCE_SHA is required}"
: "${GHCR_USERNAME:?GHCR_USERNAME is required}"
: "${GHCR_TOKEN:?GHCR_TOKEN is required}"
: "${RUNNER_TEMP:?RUNNER_TEMP is required}"
: "${GITHUB_RUN_ID:?GITHUB_RUN_ID is required}"
: "${GITHUB_RUN_ATTEMPT:?GITHUB_RUN_ATTEMPT is required}"
: "${GITHUB_OUTPUT:?GITHUB_OUTPUT is required}"

printf '%s\n' "${SOURCE_SHA}" | grep -Eq '^[0-9a-f]{40}$'

SPAWN_HOOK=${GHCR_PREFLIGHT_SPAWN_HOOK:-}
auth_config="${RUNNER_TEMP}/ghcr-auth-${GITHUB_RUN_ID}-${GITHUB_RUN_ATTEMPT}.curlrc"
token_response="${RUNNER_TEMP}/ghcr-token-${GITHUB_RUN_ID}-${GITHUB_RUN_ATTEMPT}.json"
registry_config="${RUNNER_TEMP}/ghcr-registry-${GITHUB_RUN_ID}-${GITHUB_RUN_ATTEMPT}.curlrc"
status_response="${RUNNER_TEMP}/ghcr-status-${GITHUB_RUN_ID}-${GITHUB_RUN_ATTEMPT}.txt"
CURL_PID=
SPAWNING_CURL=0
PENDING_SIGNAL_STATUS=
cleanup() {
  rm -f "${auth_config}" "${token_response}" "${registry_config}" "${status_response}"
  unset registry_token GHCR_TOKEN
}
terminate_curl() {
  if [ -n "${CURL_PID}" ]; then
    kill -TERM "${CURL_PID}" 2>/dev/null || :
    wait "${CURL_PID}" 2>/dev/null || :
    CURL_PID=
  fi
}
handle_signal() {
  signal_status=$1
  if [ "${SPAWNING_CURL}" -eq 1 ]; then
    PENDING_SIGNAL_STATUS=${signal_status}
    return 0
  fi
  exit_for_signal "${signal_status}"
}
exit_for_signal() {
  signal_status=$1
  trap - HUP INT TERM
  terminate_curl
  exit "${signal_status}"
}
run_curl() {
  curl_config=$1
  curl_output=$2
  use_spawn_hook=$3
  PENDING_SIGNAL_STATUS=
  SPAWNING_CURL=1
  curl --config "${curl_config}" > "${curl_output}" &
  hook_status=0
  if [ "${use_spawn_hook}" -eq 1 ] && [ -n "${SPAWN_HOOK}" ]; then
    if env -i PATH="${PATH}" GHCR_PREFLIGHT_PARENT_PID="$$" "${SPAWN_HOOK}"; then
      :
    else
      hook_status=$?
    fi
  fi
  CURL_PID=$!
  SPAWNING_CURL=0
  if [ -n "${PENDING_SIGNAL_STATUS}" ]; then
    pending_status=${PENDING_SIGNAL_STATUS}
    PENDING_SIGNAL_STATUS=
    exit_for_signal "${pending_status}"
  fi
  if [ "${hook_status}" -ne 0 ]; then
    terminate_curl
    return "${hook_status}"
  fi
  if wait "${CURL_PID}"; then
    CURL_PID=
    return 0
  else
    curl_status=$?
    CURL_PID=
    return "${curl_status}"
  fi
}
trap cleanup EXIT
trap 'handle_signal 129' HUP
trap 'handle_signal 130' INT
trap 'handle_signal 143' TERM
umask 077

{
  printf 'silent\n'
  printf 'show-error\n'
  printf 'fail\n'
  printf 'user = "%s:%s"\n' "${GHCR_USERNAME}" "${GHCR_TOKEN}"
  printf 'url = "https://ghcr.io/token?service=ghcr.io&scope=repository:conalog/grant-rauthy-launcher:pull"\n'
} > "${auth_config}"
run_curl "${auth_config}" "${token_response}" 0
registry_token="$(jq -er '.token // .access_token' "${token_response}")"

{
  printf 'silent\n'
  printf 'show-error\n'
  printf 'no-body\n'
  printf 'output = "/dev/null"\n'
  printf 'write-out = "%%{http_code}"\n'
  printf 'header = "Authorization: Bearer %s"\n' "${registry_token}"
  printf 'header = "Accept: application/vnd.oci.image.index.v1+json, application/vnd.docker.distribution.manifest.list.v2+json, application/vnd.oci.image.manifest.v1+json, application/vnd.docker.distribution.manifest.v2+json"\n'
  printf 'url = "https://ghcr.io/v2/conalog/grant-rauthy-launcher/manifests/rauthy-0.36.0-%s"\n' "${SOURCE_SHA}"
} > "${registry_config}"

if ! run_curl "${registry_config}" "${status_response}" 1; then
  echo "cannot verify whether the production commit tag exists" >&2
  exit 1
fi
status=$(cat "${status_response}")
case "${status}" in
  200)
    printf 'tag_exists=true\n' >> "${GITHUB_OUTPUT}"
    echo "production commit tag exists; build and push will be skipped"
    ;;
  404)
    printf 'tag_exists=false\n' >> "${GITHUB_OUTPUT}"
    ;;
  *)
    echo "unexpected GHCR manifest response: HTTP ${status}" >&2
    exit 1
    ;;
esac
