#!/bin/sh
set -eu

ROOT_DIR=$(CDPATH='' cd -P "$(dirname "$0")/.." && pwd)
TMP_DIR=$(mktemp -d "${TMPDIR:-/tmp}/lifecycle-values-test.XXXXXX")
cleanup() { rm -rf "${TMP_DIR}"; }
trap cleanup 0 HUP INT TERM

mkdir -p "${TMP_DIR}/bin" "${TMP_DIR}/lifecycle" "${TMP_DIR}/restore"
printf '%s\n' '#!/bin/sh' 'printf "%s\n" grant-voter-0' >"${TMP_DIR}/bin/hostname"
printf '%s\n' '#!/bin/sh' 'exit 1' >"${TMP_DIR}/bin/wget"
printf '%s\n' '#!/bin/sh' 'exit 97' >"${TMP_DIR}/bin/sleep"
chmod 0700 "${TMP_DIR}/bin/hostname" "${TMP_DIR}/bin/wget" "${TMP_DIR}/bin/sleep"

run_gate() {
  PATH="${TMP_DIR}/bin:${PATH}" \
    LIFECYCLE_MODE_FILE="${TMP_DIR}/lifecycle/mode" \
    LIFECYCLE_RECOVERY_ID_FILE="${TMP_DIR}/lifecycle/recovery-id" \
    LIFECYCLE_RESTORE_ID_FILE="${TMP_DIR}/restore/recovery-id" \
    LIFECYCLE_HEADLESS_SERVICE=grant-headless LIFECYCLE_REPLICAS=3 \
    LIFECYCLE_PEER_PORT=8080 LIFECYCLE_PEER_PATH=/readyz LIFECYCLE_RETRY_SECONDS=1 \
    "${ROOT_DIR}/deploy/k8s/lifecycle-gate.sh"
}

write_values() {
  printf '%b' "$1" >"${TMP_DIR}/lifecycle/mode"
  printf '%b' "$2" >"${TMP_DIR}/lifecycle/recovery-id"
  printf '%b' "$3" >"${TMP_DIR}/restore/recovery-id"
}

expect_allowed() {
  label=$1
  shift
  write_values "$@"
  run_gate >"${TMP_DIR}/${label}.log" 2>&1 \
    || { echo "valid lifecycle values were rejected: ${label}" >&2; exit 1; }
}

expect_rejected() {
  label=$1
  shift
  write_values "$@"
  if run_gate >"${TMP_DIR}/${label}.log" 2>&1; then
    echo "invalid lifecycle values were accepted: ${label}" >&2
    exit 1
  fi
}

expect_allowed lf 'restore\n' 'recovery-test-1\n' 'recovery-test-1\n'
expect_allowed crlf 'restore\r\n' 'recovery-test-1\r\n' 'recovery-test-1\r\n'
expect_allowed max-id-crlf 'restore\r\n' 'aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa\r\n' 'aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa\r\n'

expect_rejected mode-leading-space ' restore\n' 'recovery-test-1\n' 'recovery-test-1\n'
expect_rejected mode-trailing-space 'restore \n' 'recovery-test-1\n' 'recovery-test-1\n'
expect_rejected mode-internal-space 'rest ore\n' 'recovery-test-1\n' 'recovery-test-1\n'
expect_rejected mode-tab 'restore\t\n' 'recovery-test-1\n' 'recovery-test-1\n'
expect_rejected mode-extra-line 'restore\nsteady\n' 'recovery-test-1\n' 'recovery-test-1\n'
expect_rejected mode-nul 'restore\000\n' 'recovery-test-1\n' 'recovery-test-1\n'
expect_rejected mode-empty '' 'recovery-test-1\n' 'recovery-test-1\n'
expect_rejected mode-bare-cr 'restore\r' 'recovery-test-1\n' 'recovery-test-1\n'
expect_rejected id-leading-space 'restore\n' ' recovery-test-1\n' ' recovery-test-1\n'
expect_rejected id-trailing-space 'restore\n' 'recovery-test-1 \n' 'recovery-test-1 \n'
expect_rejected id-internal-space 'restore\n' 'recovery test-1\n' 'recovery test-1\n'
expect_rejected id-tab 'restore\n' 'recovery\ttest-1\n' 'recovery\ttest-1\n'
expect_rejected lifecycle-id-extra-line 'restore\n' 'recovery-test-1\nother\n' 'recovery-test-1\n'
expect_rejected restore-id-extra-line 'restore\n' 'recovery-test-1\n' 'recovery-test-1\nother\n'
expect_rejected lifecycle-id-nul 'restore\n' 'recovery\000-test-1\n' 'recovery-test-1\n'
expect_rejected restore-id-nul 'restore\n' 'recovery-test-1\n' 'recovery\000-test-1\n'
expect_rejected lifecycle-id-empty 'restore\n' '' 'recovery-test-1\n'
expect_rejected restore-id-empty 'restore\n' 'recovery-test-1\n' ''
expect_rejected lifecycle-id-bare-cr 'restore\n' 'recovery-test-1\r' 'recovery-test-1\n'
expect_rejected restore-id-bare-cr 'restore\n' 'recovery-test-1\n' 'recovery-test-1\r'
expect_rejected lifecycle-id-oversize 'restore\n' 'aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa\n' 'aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa\n'
expect_rejected divergent-id-suffixes 'restore\n' 'recovery-test-lifecycle\n' 'recovery-test-restore\n'

expect_fifo_rejected_without_blocking() {
  label=$1
  fifo_target=$2
  write_values 'restore\n' 'recovery-test-1\n' 'recovery-test-1\n'
  rm -f "${fifo_target}"
  mkfifo "${fifo_target}"
  run_gate >"${TMP_DIR}/${label}.log" 2>&1 &
  gate_pid=$!
  attempt=0
  while [ "${attempt}" -lt 3 ] && kill -0 "${gate_pid}" 2>/dev/null; do
    /bin/sleep 1
    attempt=$((attempt + 1))
  done
  if kill -0 "${gate_pid}" 2>/dev/null; then
    kill -KILL "${gate_pid}" 2>/dev/null || true
    wait "${gate_pid}" 2>/dev/null || true
    echo "lifecycle gate blocked while opening non-regular input: ${label}" >&2
    exit 1
  fi
  if wait "${gate_pid}"; then
    echo "lifecycle gate accepted non-regular input: ${label}" >&2
    exit 1
  fi
  rm -f "${fifo_target}"
}

expect_fifo_rejected_without_blocking mode-fifo "${TMP_DIR}/lifecycle/mode"
expect_fifo_rejected_without_blocking lifecycle-id-fifo "${TMP_DIR}/lifecycle/recovery-id"
expect_fifo_rejected_without_blocking restore-id-fifo "${TMP_DIR}/restore/recovery-id"

echo 'strict lifecycle value contract verified'
