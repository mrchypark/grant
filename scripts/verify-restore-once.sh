#!/bin/sh
set -eu

ROOT_DIR=$(CDPATH='' cd -P "$(dirname "$0")/.." && pwd)
TMP_DIR=$(mktemp -d "${TMPDIR:-/tmp}/restore-once-test.XXXXXX")
cleanup() { rm -rf "${TMP_DIR}"; }
trap cleanup 0 HUP INT TERM

mkdir -p "${TMP_DIR}/restore" "${TMP_DIR}/lifecycle" "${TMP_DIR}/data"
RESTORE_SHA=aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa
VALID_RESTORE="s3:restore/grant/recovery-test/${RESTORE_SHA}.enc"
MARKER="${TMP_DIR}/data/restore-state"
READER_SECRET=restore-reader-secret-must-not-reach-state-or-log
printf '%s\n' recovery-test >"${TMP_DIR}/restore/recovery-id"
printf '%s\n' restore >"${TMP_DIR}/lifecycle/mode"
printf '%s\n' recovery-test >"${TMP_DIR}/lifecycle/recovery-id"
printf '%s\n' "${VALID_RESTORE}" >"${TMP_DIR}/restore/HQL_BACKUP_RESTORE"
printf '%s\n' test-reader-key >"${TMP_DIR}/restore/HQL_S3_KEY"
printf '%s\n' "${READER_SECRET}" >"${TMP_DIR}/restore/HQL_S3_SECRET"

cat >"${TMP_DIR}/child.sh" <<'EOF'
#!/bin/sh
set -eu
printf '%s|%s|%s\n' "${HQL_BACKUP_RESTORE-unset}" "${HQL_S3_KEY-unset}" "${HQL_S3_SECRET-unset}" >"${CAPTURE}"
case "${CHILD_MODE}" in
  fail) exit 37 ;;
  signal) kill -TERM "$$"; sleep 10 ;;
  wait|complete)
    trap ': >"${CHILD_SIGNAL_MARKER}"; exit 143' TERM
    : >"${CHILD_READY_MARKER}"
    while :; do
      if [ "${CHILD_MODE}" = complete ] && [ "$(sed -n '1p' "${MARKER_PATH}" 2>/dev/null || true)" = completed ]; then
        exit 0
      fi
      sleep 1
    done
    ;;
  ignore)
    trap '' TERM
    printf '%s\n' "$$" >"${CHILD_PID_FILE}"
    : >"${CHILD_READY_MARKER}"
    while :; do sleep 1; done
    ;;
  capture) exit 0 ;;
  *) exit 98 ;;
esac
EOF

cat >"${TMP_DIR}/probe.sh" <<'EOF'
#!/bin/sh
set -eu
url=
for argument in "$@"; do url=${argument}; done
printf '%s\n' "${url}" >>"${PROBE_LOG}"
[ "${HQL_BACKUP_RESTORE-unset}" = unset ] && [ "${HQL_S3_KEY-unset}" = unset ] && [ "${HQL_S3_SECRET-unset}" = unset ] \
  || { : >"${PROBE_SECRET_LEAK}"; exit 1; }
[ "$(sed -n '1p' "${MARKER_PATH}")" = pending ] || exit 1
if grep -Fq "${READER_SECRET}" "${MARKER_PATH}" 2>/dev/null; then : >"${PROBE_SECRET_LEAK}"; exit 1; fi
case "${PROBE_MODE}" in
  success) exit 0 ;;
  fail) exit 1 ;;
  sequence)
    case "${url}" in
      */healthz) exit 0 ;;
      */readyz)
        sequence_count=$(sed -n '1p' "${PROBE_SEQUENCE_COUNT}")
        sequence_count=$((sequence_count + 1))
        printf '%s\n' "${sequence_count}" >"${PROBE_SEQUENCE_COUNT}"
        [ "${sequence_count}" -ne 3 ]
        ;;
      *) exit 1 ;;
    esac
    ;;
  *) exit 1 ;;
esac
EOF
cat >"${TMP_DIR}/spawn-signal.sh" <<'EOF'
#!/bin/sh
set -eu
attempts=0
while [ "${attempts}" -lt 5 ] && [ ! -e "${CHILD_READY_MARKER}" ]; do
  attempts=$((attempts + 1))
  sleep 1
done
[ -e "${CHILD_READY_MARKER}" ]
kill -TERM "${PPID}"
EOF
chmod 0700 "${TMP_DIR}/child.sh" "${TMP_DIR}/probe.sh" "${TMP_DIR}/spawn-signal.sh"

run_wrapper() {
  capture=$1
  child_mode=$2
  probe_mode=$3
  timeout=${4:-4}
  spawn_hook=${5:-}
  set -- env CAPTURE="${capture}" CHILD_MODE="${child_mode}" CHILD_SIGNAL_MARKER="${TMP_DIR}/child-signal" \
    CHILD_READY_MARKER="${TMP_DIR}/child-ready" CHILD_PID_FILE="${TMP_DIR}/child.pid" \
    MARKER_PATH="${MARKER}" PROBE_MODE="${probe_mode}" PROBE_LOG="${TMP_DIR}/probe.log" \
    PROBE_SEQUENCE_COUNT="${TMP_DIR}/probe-sequence-count" \
    PROBE_SECRET_LEAK="${TMP_DIR}/probe-secret-leak" READER_SECRET="${READER_SECRET}" \
    RESTORE_ONCE_EXEC="${TMP_DIR}/child.sh" RESTORE_ONCE_MARKER="${MARKER}" \
    RESTORE_ONCE_VALUE_FILE="${TMP_DIR}/restore/HQL_BACKUP_RESTORE" RESTORE_ONCE_ID_FILE="${TMP_DIR}/restore/recovery-id" \
    RESTORE_ONCE_S3_KEY_FILE="${TMP_DIR}/restore/HQL_S3_KEY" RESTORE_ONCE_S3_SECRET_FILE="${TMP_DIR}/restore/HQL_S3_SECRET" \
    RESTORE_ONCE_LIFECYCLE_MODE_FILE="${TMP_DIR}/lifecycle/mode" \
    RESTORE_ONCE_RECOVERY_ID_FILE="${TMP_DIR}/lifecycle/recovery-id" RESTORE_ONCE_PROBE_EXEC="${TMP_DIR}/probe.sh" \
    RESTORE_ONCE_READY_TIMEOUT_SECONDS="${timeout}" RESTORE_ONCE_READY_INTERVAL_SECONDS=1 \
    RESTORE_ONCE_PROBE_TIMEOUT_SECONDS=1 RESTORE_ONCE_SIGNAL_GRACE_SECONDS=2 \
    RESTORE_ONCE_SPAWN_HOOK="${spawn_hook}" \
    "${ROOT_DIR}/deploy/k8s/restore-once.sh"
  if [ "${RUN_WRAPPER_EXEC:-0}" = 1 ]; then exec "$@"; fi
  "$@"
}

set +e
run_wrapper "${TMP_DIR}/startup-failure.capture" fail fail >"${TMP_DIR}/startup-failure.log" 2>&1
startup_status=$?
set -e
[ "${startup_status}" -eq 37 ] || { echo "restore startup failure status was ${startup_status}, expected 37" >&2; exit 1; }
[ "$(sed -n '1p' "${MARKER}")" = pending ] || { echo "startup failure completed the restore marker" >&2; exit 1; }

set +e
run_wrapper "${TMP_DIR}/child-signal.capture" signal fail >"${TMP_DIR}/child-signal.log" 2>&1
child_signal_status=$?
set -e
[ "${child_signal_status}" -eq 143 ] || { echo "signaled child status was ${child_signal_status}, expected 143" >&2; exit 1; }
[ "$(sed -n '1p' "${MARKER}")" = pending ] || { echo "signaled child completed the restore marker" >&2; exit 1; }

rm -f "${TMP_DIR}/child-ready" "${TMP_DIR}/child-signal"
set +e
run_wrapper "${TMP_DIR}/spawn-signal.capture" wait fail 30 "${TMP_DIR}/spawn-signal.sh" \
  >"${TMP_DIR}/spawn-signal.log" 2>&1
spawn_signal_status=$?
set -e
[ "${spawn_signal_status}" -eq 143 ] \
  || { echo "spawn-window TERM status was ${spawn_signal_status}, expected 143" >&2; exit 1; }
[ -e "${TMP_DIR}/child-signal" ] || { echo "spawn-window TERM was not forwarded after PID assignment" >&2; exit 1; }
[ "$(sed -n '1p' "${MARKER}")" = pending ] || { echo "spawn-window TERM completed the restore marker" >&2; exit 1; }

rm -f "${TMP_DIR}/child-ready" "${TMP_DIR}/child-signal"
RUN_WRAPPER_EXEC=1 run_wrapper "${TMP_DIR}/forwarded-signal.capture" wait fail 30 >"${TMP_DIR}/forwarded-signal.log" 2>&1 &
wrapper_pid=$!
attempts=0
while [ "${attempts}" -lt 5 ] && ! kill -0 "${wrapper_pid}" 2>/dev/null; do attempts=$((attempts + 1)); sleep 1; done
sleep 1
kill -TERM "${wrapper_pid}"
set +e
wait "${wrapper_pid}"
forwarded_signal_status=$?
set -e
[ "${forwarded_signal_status}" -eq 143 ] || { echo "forwarded TERM status was ${forwarded_signal_status}, expected 143" >&2; exit 1; }
[ -e "${TMP_DIR}/child-signal" ] || { echo "restore supervisor did not forward TERM to its child" >&2; exit 1; }
[ "$(sed -n '1p' "${MARKER}")" = pending ] || { echo "forwarded TERM completed the restore marker" >&2; exit 1; }
rm -f "${TMP_DIR}/child-signal"

rm -f "${TMP_DIR}/child-ready" "${TMP_DIR}/child.pid"
RUN_WRAPPER_EXEC=1 run_wrapper "${TMP_DIR}/forced-kill.capture" ignore fail 30 >"${TMP_DIR}/forced-kill.log" 2>&1 &
forced_wrapper_pid=$!
attempts=0
while [ "${attempts}" -lt 5 ] && [ ! -s "${TMP_DIR}/child.pid" ]; do attempts=$((attempts + 1)); sleep 1; done
[ -s "${TMP_DIR}/child.pid" ] || { echo "TERM-ignoring child did not start" >&2; exit 1; }
forced_child_pid=$(cat "${TMP_DIR}/child.pid")
kill -TERM "${forced_wrapper_pid}"
set +e
wait "${forced_wrapper_pid}"
forced_wrapper_status=$?
set -e
[ "${forced_wrapper_status}" -eq 137 ] \
  || { echo "forced child termination status was ${forced_wrapper_status}, expected 137" >&2; exit 1; }
if kill -0 "${forced_child_pid}" 2>/dev/null; then
  echo "TERM-ignoring child survived grace expiry and SIGKILL" >&2
  exit 1
fi

set +e
run_wrapper "${TMP_DIR}/timeout.capture" wait fail 1 >"${TMP_DIR}/timeout.log" 2>&1
timeout_status=$?
set -e
[ "${timeout_status}" -eq 124 ] || { echo "restore readiness timeout status was ${timeout_status}, expected 124" >&2; exit 1; }
[ -e "${TMP_DIR}/child-signal" ] || { echo "restore readiness timeout did not terminate its child" >&2; exit 1; }
[ "$(sed -n '1p' "${MARKER}")" = pending ] || { echo "restore timeout completed the restore marker" >&2; exit 1; }

: >"${TMP_DIR}/probe.log"
printf '%s\n' 0 >"${TMP_DIR}/probe-sequence-count"
run_wrapper "${TMP_DIR}/completed.capture" complete sequence 10 >"${TMP_DIR}/completed.log" 2>&1
[ "$(sed -n '1p' "${MARKER}")" = completed ] && [ "$(sed -n '2p' "${MARKER}")" = recovery-test ] \
  && [ "$(sed -n '3p' "${MARKER}")" = "${VALID_RESTORE}" ] \
  || { echo "stale pending restore did not atomically reach completed state" >&2; exit 1; }
[ "$(cat "${TMP_DIR}/completed.capture")" = "${VALID_RESTORE}|test-reader-key|${READER_SECRET}" ] \
  || { echo "successful restore did not receive the authorized reader credentials" >&2; exit 1; }
health_probes=$(grep -cx 'http://127.0.0.1:8080/healthz' "${TMP_DIR}/probe.log" || true)
ready_probes=$(grep -cx 'http://127.0.0.1:8080/readyz' "${TMP_DIR}/probe.log" || true)
[ "${health_probes}" -eq 6 ] || { echo "successful restore did not require three new health probes after a failure" >&2; exit 1; }
[ "${ready_probes}" -eq 6 ] && [ "$(cat "${TMP_DIR}/probe-sequence-count")" -eq 6 ] \
  || { echo "successful restore did not reset consecutive readiness after a failure" >&2; exit 1; }

run_wrapper "${TMP_DIR}/replay.capture" capture fail >"${TMP_DIR}/replay.log" 2>&1
[ "$(cat "${TMP_DIR}/replay.capture")" = 'unset|unset|unset' ] \
  || { echo "completed restore authorization replay was not suppressed" >&2; exit 1; }
[ ! -e "${TMP_DIR}/probe-secret-leak" ] || { echo "restore supervisor leaked reader credentials to its probe" >&2; exit 1; }
if grep -Fq "${READER_SECRET}" "${MARKER}" "${TMP_DIR}"/*.log; then
  echo "restore state or logs exposed the reader secret" >&2
  exit 1
fi
if find "${TMP_DIR}/data" -name '*.tmp.*' -print | grep -q .; then
  echo "restore supervisor left an atomic state temporary file behind" >&2
  exit 1
fi

mv "${TMP_DIR}/restore" "${TMP_DIR}/restore.saved"
run_wrapper "${TMP_DIR}/completed-without-secret.capture" capture fail >"${TMP_DIR}/completed-without-secret.log" 2>&1
[ "$(cat "${TMP_DIR}/completed-without-secret.capture")" = 'unset|unset|unset' ] \
  || { echo "matching completed state could not start without the consumed Secret" >&2; exit 1; }
mv "${TMP_DIR}/restore.saved" "${TMP_DIR}/restore"

printf '%s\n%s\n%s\n' pending recovery-test "${VALID_RESTORE}" >"${MARKER}"
mv "${TMP_DIR}/restore" "${TMP_DIR}/restore.saved"
if run_wrapper "${TMP_DIR}/pending-without-secret.capture" capture fail >/dev/null 2>&1; then
  echo "pending restore started without its authorization" >&2
  exit 1
fi
mv "${TMP_DIR}/restore.saved" "${TMP_DIR}/restore"

ALTERED_RESTORE="s3:restore/grant/recovery-test/$(printf '%064d' 0).enc"
printf '%s\n' "${ALTERED_RESTORE}" >"${TMP_DIR}/restore/HQL_BACKUP_RESTORE"
if run_wrapper "${TMP_DIR}/altered-pending.capture" capture fail >/dev/null 2>&1; then
  echo "pending restore accepted a different object for the same recovery ID" >&2
  exit 1
fi
printf '%s\n' "${VALID_RESTORE}" >"${TMP_DIR}/restore/HQL_BACKUP_RESTORE"

expect_rejected() {
  label=$1
  restore_id=$2
  lifecycle_id=$3
  directive=$4
  rm -f "${MARKER}"
  printf '%s\n' "${restore_id}" >"${TMP_DIR}/restore/recovery-id"
  printf '%s\n' "${lifecycle_id}" >"${TMP_DIR}/lifecycle/recovery-id"
  printf '%s\n' "${directive}" >"${TMP_DIR}/restore/HQL_BACKUP_RESTORE"
  if run_wrapper "${TMP_DIR}/${label}.capture" capture fail >/dev/null 2>&1; then
    echo "invalid restore authorization was accepted: ${label}" >&2
    exit 1
  fi
  [ ! -e "${MARKER}" ] || { echo "invalid restore authorization created state: ${label}" >&2; exit 1; }
}

expect_rejected stale-recovery stale-recovery recovery-test "s3:restore/grant/stale-recovery/${RESTORE_SHA}.enc"
expect_rejected other-recovery recovery-test recovery-test "s3:restore/grant/other-recovery/${RESTORE_SHA}.enc"
expect_rejected cross-workload recovery-test recovery-test "s3:restore/rauthy/recovery-test/${RESTORE_SHA}.enc"
expect_rejected live-backup recovery-test recovery-test 's3:backup_node_1_1.sqlite'
expect_rejected uppercase-sha recovery-test recovery-test 's3:restore/grant/recovery-test/AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA.enc'
expect_rejected path-characters recovery-test recovery-test "s3:restore/grant/recovery-test/${RESTORE_SHA}/extra.enc"
expect_rejected short-sha recovery-test recovery-test 's3:restore/grant/recovery-test/abcd.enc'

rm -f "${MARKER}"
printf '%s\n' recovery-test >"${TMP_DIR}/restore/recovery-id"
printf '%s\n' recovery-test >"${TMP_DIR}/lifecycle/recovery-id"
printf '%s\n' "${VALID_RESTORE}" >"${TMP_DIR}/restore/HQL_BACKUP_RESTORE"
rm -f "${TMP_DIR}/restore/HQL_S3_SECRET"
if run_wrapper "${TMP_DIR}/incomplete-reader.capture" capture fail >/dev/null 2>&1; then
  echo "restore authorization without both reader credentials was accepted" >&2
  exit 1
fi

echo "restore-once health-gated recovery and replay protection verified"
