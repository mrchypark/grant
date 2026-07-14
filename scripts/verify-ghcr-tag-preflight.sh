#!/bin/sh
set -eu

ROOT_DIR=$(CDPATH='' cd -P "$(dirname "$0")/.." && pwd)
TMP_DIR=$(mktemp -d "${TMPDIR:-/tmp}/grant-ghcr-preflight.XXXXXX")
cleanup() {
  rm -rf "${TMP_DIR}"
}
trap cleanup EXIT HUP INT TERM

mkdir -p "${TMP_DIR}/bin"
cat > "${TMP_DIR}/bin/curl" <<'EOF'
#!/bin/sh
set -eu

[ "$#" -eq 2 ] && [ "$1" = --config ]
config=$2
if grep -Fq 'ghcr.io/token?service=ghcr.io' "${config}"; then
  printf '{"token":"mock-registry-reader-token"}\n'
  exit 0
fi

grep -qx 'no-body' "${config}" || {
  echo 'curl: (18) transfer closed with bytes remaining to read' >&2
  exit 18
}
if grep -q '^request = "HEAD"$' "${config}"; then
  echo 'custom HEAD request must not replace curl no-body semantics' >&2
  exit 2
fi
grep -qx 'output = "/dev/null"' "${config}"
grep -qx 'write-out = "%{http_code}"' "${config}"
grep -qx 'header = "Authorization: Bearer mock-registry-reader-token"' "${config}"

case "${MOCK_CURL_RESULT}" in
  200|404|503)
    printf '%s' "${MOCK_CURL_RESULT}"
    ;;
  network)
    echo 'curl: (7) failed to connect' >&2
    exit 7
    ;;
  delay)
    finish_signal() {
      : > "${MOCK_CURL_TERMINATED_FILE}"
      exit 143
    }
    trap finish_signal HUP INT TERM
    printf '%s\n' "$$" > "${MOCK_CURL_PID_FILE}"
    printf '%s\n' "${PPID}" > "${MOCK_HELPER_PID_FILE}"
    : > "${MOCK_CURL_READY_FILE}"
    while :; do :; done
    ;;
  *)
    echo "unexpected mock result: ${MOCK_CURL_RESULT}" >&2
    exit 2
    ;;
esac
EOF
chmod 0700 "${TMP_DIR}/bin/curl"

run_case() {
  result=$1
  expected_status=$2
  expected_message=$3
  case_dir="${TMP_DIR}/case-${result}"
  log="${TMP_DIR}/${result}.log"
  mkdir -p "${case_dir}"

  set +e
  PATH="${TMP_DIR}/bin:${PATH}" MOCK_CURL_RESULT="${result}" \
    SOURCE_SHA=aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa \
    GHCR_USERNAME=mock-user GHCR_TOKEN=mock-ghcr-secret \
    RUNNER_TEMP="${case_dir}" GITHUB_RUN_ID=123 GITHUB_RUN_ATTEMPT=1 \
    sh "${ROOT_DIR}/scripts/ghcr-tag-preflight.sh" > "${log}" 2>&1
  actual_status=$?
  set -e

  [ "${actual_status}" -eq "${expected_status}" ] || {
    echo "GHCR preflight ${result} returned ${actual_status}, expected ${expected_status}" >&2
    exit 1
  }
  if [ -n "${expected_message}" ]; then
    grep -Fq "${expected_message}" "${log}" || {
      echo "GHCR preflight ${result} omitted its fail-closed message" >&2
      exit 1
    }
  fi
  if grep -Fq 'mock-ghcr-secret' "${log}" || grep -Fq 'mock-registry-reader-token' "${log}"; then
    echo "GHCR preflight ${result} exposed credentials" >&2
    exit 1
  fi
  if find "${case_dir}" -type f -print -quit | grep -q .; then
    echo "GHCR preflight ${result} left credential files behind" >&2
    exit 1
  fi
}

run_case 404 0 ''
run_case 200 1 'production commit tag already exists and will not be overwritten'
run_case 503 1 'unexpected GHCR manifest response: HTTP 503'
run_case network 1 'cannot verify whether the production commit tag exists'

run_signal_case() {
  signal_name=$1
  expected_status=$2
  case_dir="${TMP_DIR}/signal-${signal_name}"
  ready="${TMP_DIR}/${signal_name}.ready"
  terminated="${TMP_DIR}/${signal_name}.terminated"
  curl_pid_file="${TMP_DIR}/${signal_name}.curl-pid"
  helper_pid_file="${TMP_DIR}/${signal_name}.helper-pid"
  timeout_marker="${TMP_DIR}/${signal_name}.timeout"
  continuation="${TMP_DIR}/${signal_name}.continued"
  log="${TMP_DIR}/${signal_name}.log"
  mkdir -p "${case_dir}"

  (
    attempts=0
    while [ ! -e "${ready}" ] || [ ! -s "${helper_pid_file}" ]; do
      attempts=$((attempts + 1))
      [ "${attempts}" -lt 1000000 ] || {
        echo "GHCR preflight ${signal_name} fixture did not become ready" >&2
        exit 1
      }
    done
    helper_pid=$(cat "${helper_pid_file}")
    kill -s "${signal_name}" "${helper_pid}"
    sleep 1
    if kill -0 "${helper_pid}" 2>/dev/null; then
      : > "${timeout_marker}"
      kill -KILL "${helper_pid}" 2>/dev/null || :
    fi
  ) &
  signal_sender_pid=$!

  set +e
  PATH="${TMP_DIR}/bin:${PATH}" MOCK_CURL_RESULT=delay \
    MOCK_CURL_READY_FILE="${ready}" MOCK_CURL_TERMINATED_FILE="${terminated}" \
    MOCK_CURL_PID_FILE="${curl_pid_file}" MOCK_HELPER_PID_FILE="${helper_pid_file}" \
    SOURCE_SHA=aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa \
    GHCR_USERNAME=mock-user GHCR_TOKEN=mock-ghcr-secret \
    RUNNER_TEMP="${case_dir}" GITHUB_RUN_ID=123 GITHUB_RUN_ATTEMPT=1 \
    sh "${ROOT_DIR}/scripts/ghcr-tag-preflight.sh" > "${log}" 2>&1
  actual_status=$?
  set -e
  wait "${signal_sender_pid}"
  if [ "${actual_status}" -eq 0 ]; then
    : > "${continuation}"
  fi

  [ "${actual_status}" -eq "${expected_status}" ] || {
    echo "GHCR preflight ${signal_name} returned ${actual_status}, expected ${expected_status}" >&2
    exit 1
  }
  [ ! -e "${continuation}" ] || {
    echo "GHCR preflight ${signal_name} continued into the publish path" >&2
    exit 1
  }
  [ ! -e "${timeout_marker}" ] || {
    echo "GHCR preflight ${signal_name} did not exit from its signal handler" >&2
    exit 1
  }
  [ -e "${terminated}" ] || {
    echo "GHCR preflight ${signal_name} did not terminate its curl child" >&2
    exit 1
  }
  curl_pid=$(cat "${curl_pid_file}")
  if kill -0 "${curl_pid}" 2>/dev/null; then
    echo "GHCR preflight ${signal_name} left its curl child running" >&2
    kill -TERM "${curl_pid}" 2>/dev/null || :
    exit 1
  fi
  if grep -Fq 'mock-ghcr-secret' "${log}" || grep -Fq 'mock-registry-reader-token' "${log}"; then
    echo "GHCR preflight ${signal_name} exposed credentials" >&2
    exit 1
  fi
  if find "${case_dir}" -type f -print -quit | grep -q .; then
    echo "GHCR preflight ${signal_name} left credential files behind" >&2
    exit 1
  fi
}

run_signal_case HUP 129
run_signal_case INT 130
run_signal_case TERM 143

run_spawn_signal_case() {
  signal_name=$1
  expected_status=$2
  case_dir="${TMP_DIR}/spawn-signal-${signal_name}"
  ready="${TMP_DIR}/${signal_name}.spawn-ready"
  terminated="${TMP_DIR}/${signal_name}.spawn-terminated"
  curl_pid_file="${TMP_DIR}/${signal_name}.spawn-curl-pid"
  mock_helper_pid_file="${TMP_DIR}/${signal_name}.spawn-mock-helper-pid"
  helper_pid_file="${TMP_DIR}/${signal_name}.spawn-helper-pid"
  hook_returned="${TMP_DIR}/${signal_name}.spawn-hook-returned"
  hook_secret_leak="${TMP_DIR}/${signal_name}.spawn-hook-secret-leak"
  timeout_marker="${TMP_DIR}/${signal_name}.spawn-timeout"
  continuation="${TMP_DIR}/${signal_name}.spawn-continued"
  hook="${TMP_DIR}/spawn-${signal_name}.sh"
  log="${TMP_DIR}/${signal_name}.spawn.log"
  mkdir -p "${case_dir}"

  cat > "${hook}" <<'EOF'
#!/bin/sh
set -eu

hook_dir=${0%/*}
signal_name=${0##*-}
signal_name=${signal_name%.sh}
if [ "${GHCR_TOKEN+set}" = set ] || [ "${registry_token+set}" = set ] \
  || [ "${SOURCE_SHA+set}" = set ] || [ "${GHCR_USERNAME+set}" = set ] \
  || [ "${RUNNER_TEMP+set}" = set ] || [ "${GHCR_PREFLIGHT_SPAWN_HOOK+set}" = set ] \
  || [ "${MOCK_CURL_RESULT+set}" = set ] || [ "${MOCK_CURL_READY_FILE+set}" = set ]; then
  : > "${hook_dir}/${signal_name}.spawn-hook-secret-leak"
  exit 1
fi
printf '%s\n' "${GHCR_PREFLIGHT_PARENT_PID}" > "${hook_dir}/${signal_name}.spawn-helper-pid"
attempts=0
while [ ! -e "${hook_dir}/${signal_name}.spawn-ready" ]; do
  attempts=$((attempts + 1))
  [ "${attempts}" -lt 1000000 ] || exit 1
done
kill -s "${signal_name}" "${GHCR_PREFLIGHT_PARENT_PID}"
: > "${hook_dir}/${signal_name}.spawn-hook-returned"
EOF
  chmod 0700 "${hook}"

  (
    attempts=0
    while [ ! -s "${helper_pid_file}" ]; do
      attempts=$((attempts + 1))
      [ "${attempts}" -lt 1000000 ] || exit 1
    done
    helper_pid=$(cat "${helper_pid_file}")
    sleep 1
    if kill -0 "${helper_pid}" 2>/dev/null; then
      : > "${timeout_marker}"
      kill -KILL "${helper_pid}" 2>/dev/null || :
    fi
  ) &
  watchdog_pid=$!

  set +e
  PATH="${TMP_DIR}/bin:${PATH}" MOCK_CURL_RESULT=delay \
    MOCK_CURL_READY_FILE="${ready}" MOCK_CURL_TERMINATED_FILE="${terminated}" \
    MOCK_CURL_PID_FILE="${curl_pid_file}" MOCK_HELPER_PID_FILE="${mock_helper_pid_file}" \
    GHCR_PREFLIGHT_SPAWN_HOOK="${hook}" \
    SOURCE_SHA=aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa \
    GHCR_USERNAME=mock-user GHCR_TOKEN=mock-ghcr-secret \
    RUNNER_TEMP="${case_dir}" GITHUB_RUN_ID=123 GITHUB_RUN_ATTEMPT=1 \
    sh "${ROOT_DIR}/scripts/ghcr-tag-preflight.sh" > "${log}" 2>&1
  actual_status=$?
  set -e
  wait "${watchdog_pid}"
  if [ "${actual_status}" -eq 0 ]; then
    : > "${continuation}"
  fi

  [ "${actual_status}" -eq "${expected_status}" ] || {
    echo "GHCR spawn preflight ${signal_name} returned ${actual_status}, expected ${expected_status}" >&2
    exit 1
  }
  [ -e "${hook_returned}" ] || {
    echo "GHCR spawn preflight ${signal_name} did not return from its signal hook" >&2
    exit 1
  }
  [ ! -e "${hook_secret_leak}" ] || {
    echo "GHCR spawn preflight ${signal_name} exposed production state to its hook" >&2
    exit 1
  }
  [ ! -e "${continuation}" ] || {
    echo "GHCR spawn preflight ${signal_name} continued into the publish path" >&2
    exit 1
  }
  [ ! -e "${timeout_marker}" ] || {
    echo "GHCR spawn preflight ${signal_name} lost its pending signal" >&2
    exit 1
  }
  [ -e "${terminated}" ] || {
    echo "GHCR spawn preflight ${signal_name} did not terminate its curl child" >&2
    exit 1
  }
  curl_pid=$(cat "${curl_pid_file}")
  if kill -0 "${curl_pid}" 2>/dev/null; then
    echo "GHCR spawn preflight ${signal_name} left its curl child running" >&2
    kill -TERM "${curl_pid}" 2>/dev/null || :
    exit 1
  fi
  if grep -Fq 'mock-ghcr-secret' "${log}" || grep -Fq 'mock-registry-reader-token' "${log}"; then
    echo "GHCR spawn preflight ${signal_name} exposed credentials" >&2
    exit 1
  fi
  if find "${case_dir}" -type f -print -quit | grep -q .; then
    echo "GHCR spawn preflight ${signal_name} left credential files behind" >&2
    exit 1
  fi
}

run_spawn_signal_case HUP 129
run_spawn_signal_case INT 130
run_spawn_signal_case TERM 143

echo 'GHCR manifest tag preflight behavior verified'
