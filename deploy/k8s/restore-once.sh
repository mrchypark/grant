#!/bin/sh
set -eu

EXECUTABLE=${RESTORE_ONCE_EXEC:?RESTORE_ONCE_EXEC is required}
MARKER=${RESTORE_ONCE_MARKER:?RESTORE_ONCE_MARKER is required}
RESTORE_VALUE_FILE=${RESTORE_ONCE_VALUE_FILE:-/restore/HQL_BACKUP_RESTORE}
RESTORE_ID_FILE=${RESTORE_ONCE_ID_FILE:-/restore/recovery-id}
LIFECYCLE_MODE_FILE=${RESTORE_ONCE_LIFECYCLE_MODE_FILE:-/lifecycle/mode}
RECOVERY_ID_FILE=${RESTORE_ONCE_RECOVERY_ID_FILE:-/lifecycle/recovery-id}
RESTORE_S3_KEY_FILE=${RESTORE_ONCE_S3_KEY_FILE:-/restore/HQL_S3_KEY}
RESTORE_S3_SECRET_FILE=${RESTORE_ONCE_S3_SECRET_FILE:-/restore/HQL_S3_SECRET}
HEALTH_URL=${RESTORE_ONCE_HEALTH_URL:-http://127.0.0.1:8080/healthz}
READY_URL=${RESTORE_ONCE_READY_URL:-http://127.0.0.1:8080/readyz}
PROBE_EXEC=${RESTORE_ONCE_PROBE_EXEC:-curl}
READY_TIMEOUT=${RESTORE_ONCE_READY_TIMEOUT_SECONDS:-300}
READY_INTERVAL=${RESTORE_ONCE_READY_INTERVAL_SECONDS:-2}
READY_SUCCESSES=${RESTORE_ONCE_READY_SUCCESSES:-3}
PROBE_TIMEOUT=${RESTORE_ONCE_PROBE_TIMEOUT_SECONDS:-2}
SIGNAL_GRACE=${RESTORE_ONCE_SIGNAL_GRACE_SECONDS:-10}
SPAWN_HOOK=${RESTORE_ONCE_SPAWN_HOOK:-}
CHILD_PID=
MARKER_TMP=
SPAWNING_CHILD=0
PENDING_SIGNAL=
PENDING_SIGNAL_STATUS=

case "${READY_TIMEOUT}:${READY_INTERVAL}:${READY_SUCCESSES}:${PROBE_TIMEOUT}:${SIGNAL_GRACE}" in
  *[!0-9:]*|:*|*::*|*:) echo "restore-once timeout settings must be positive integers" >&2; exit 2 ;;
esac
[ "${READY_TIMEOUT}" -gt 0 ] && [ "${READY_INTERVAL}" -gt 0 ] && [ "${READY_SUCCESSES}" -gt 0 ] \
  && [ "${PROBE_TIMEOUT}" -gt 0 ] && [ "${SIGNAL_GRACE}" -gt 0 ] \
  || { echo "restore-once timeout settings must be positive integers" >&2; exit 2; }

cleanup_tmp() {
  if [ -n "${MARKER_TMP}" ]; then rm -f "${MARKER_TMP}" || true; fi
}

wait_child() {
  set +e
  wait "${CHILD_PID}"
  child_status=$?
  set -e
  CHILD_PID=
}

stop_child() {
  signal=$1
  trap '' HUP INT TERM
  if [ -n "${CHILD_PID}" ] && kill -0 "${CHILD_PID}" 2>/dev/null; then
    kill -s "${signal}" "${CHILD_PID}" 2>/dev/null || true
    remaining=${SIGNAL_GRACE}
    while [ "${remaining}" -gt 0 ] && kill -0 "${CHILD_PID}" 2>/dev/null; do
      sleep 1
      remaining=$((remaining - 1))
    done
    if kill -0 "${CHILD_PID}" 2>/dev/null; then kill -KILL "${CHILD_PID}" 2>/dev/null || true; fi
  fi
  if [ -n "${CHILD_PID}" ]; then wait_child; else child_status=1; fi
  cleanup_tmp
}

forward_signal() {
  signal=$1
  fallback_status=$2
  if [ -z "${CHILD_PID}" ]; then
    cleanup_tmp
    exit "${fallback_status}"
  fi
  stop_child "${signal}"
  [ "${child_status}" -ne 0 ] || child_status=${fallback_status}
  exit "${child_status}"
}

handle_signal() {
  signal=$1
  fallback_status=$2
  if [ "${SPAWNING_CHILD}" -eq 1 ]; then
    PENDING_SIGNAL=${signal}
    PENDING_SIGNAL_STATUS=${fallback_status}
    return 0
  fi
  forward_signal "${signal}" "${fallback_status}"
}

write_marker() {
  marker_state=$1
  marker_id=$2
  marker_restore=$3
  marker_parent=${MARKER%/*}
  [ "${marker_parent}" != "${MARKER}" ] && mkdir -p "${marker_parent}"
  MARKER_TMP="${MARKER}.tmp.$$"
  umask 077
  {
    printf '%s\n' "${marker_state}"
    printf '%s\n' "${marker_id}"
    printf '%s\n' "${marker_restore}"
  } >"${MARKER_TMP}"
  mv -f "${MARKER_TMP}" "${MARKER}"
  MARKER_TMP=
}

probe_url() {
  "${PROBE_EXEC}" -fsS --max-time "${PROBE_TIMEOUT}" "$1" >/dev/null 2>&1
}

validate_recovery_id() {
  checked_id=$1
  case "${checked_id}" in ''|*[!a-z0-9-]*|-*|*-) return 1 ;; esac
  [ "${#checked_id}" -le 32 ]
}

validate_restore_directive() {
  checked_id=$1
  checked_restore=$2
  checked_prefix="s3:restore/grant/${checked_id}/"
  case "${checked_restore}" in
    "${checked_prefix}"*.enc) ;;
    *) return 1 ;;
  esac
  checked_object=${checked_restore#"${checked_prefix}"}
  checked_sha=${checked_object%.enc}
  case "${checked_sha}" in ''|*[!0-9a-f]*) return 1 ;; esac
  [ "${#checked_sha}" -eq 64 ] && [ "${checked_object}" = "${checked_sha}.enc" ]
}

trap cleanup_tmp 0
trap 'handle_signal HUP 129' HUP
trap 'handle_signal INT 130' INT
trap 'handle_signal TERM 143' TERM

unset HQL_BACKUP_RESTORE HQL_S3_KEY HQL_S3_SECRET
[ -r "${LIFECYCLE_MODE_FILE}" ] && [ -r "${RECOVERY_ID_FILE}" ] \
  || { echo "incomplete lifecycle state mount" >&2; exit 1; }
lifecycle_mode=$(sed -n '1{s/[[:space:]]//g;p;}' "${LIFECYCLE_MODE_FILE}")
recovery_id=$(sed -n '1{s/[[:space:]]//g;p;}' "${RECOVERY_ID_FILE}")
case "${lifecycle_mode}" in fresh|steady|restore) ;; *) echo "invalid lifecycle mode" >&2; exit 1 ;; esac
validate_recovery_id "${recovery_id}" || { echo "lifecycle recovery ID must be a lowercase DNS label" >&2; exit 1; }

marker_present=0
marker_state=
marker_id=
marker_restore=
if [ -e "${MARKER}" ]; then
  [ -f "${MARKER}" ] && [ -r "${MARKER}" ] || { echo "restore state is not a readable regular file" >&2; exit 1; }
  [ "$(wc -l <"${MARKER}" | tr -d ' ')" -eq 3 ] || { echo "restore state is incomplete or malformed" >&2; exit 1; }
  marker_state=$(sed -n '1p' "${MARKER}")
  marker_id=$(sed -n '2p' "${MARKER}")
  marker_restore=$(sed -n '3p' "${MARKER}")
  case "${marker_state}" in pending|completed) ;; *) echo "restore state has an unknown phase" >&2; exit 1 ;; esac
  if ! validate_recovery_id "${marker_id}" || ! validate_restore_directive "${marker_id}" "${marker_restore}"; then
    echo "restore state has an invalid recovery binding" >&2
    exit 1
  fi
  marker_present=1
fi

authorization_files=0
for authorization_file in "${RESTORE_VALUE_FILE}" "${RESTORE_ID_FILE}" "${RESTORE_S3_KEY_FILE}" "${RESTORE_S3_SECRET_FILE}"; do
  if [ -r "${authorization_file}" ]; then authorization_files=$((authorization_files + 1)); fi
done
[ "${authorization_files}" -eq 0 ] || [ "${authorization_files}" -eq 4 ] \
  || { echo "incomplete restore authorization mount" >&2; exit 1; }

if [ "${authorization_files}" -eq 4 ]; then
  [ "${lifecycle_mode}" = restore ] || { echo "restore authorization requires lifecycle restore mode" >&2; exit 1; }
  restore_id=$(sed -n '1{s/[[:space:]]//g;p;}' "${RESTORE_ID_FILE}")
  validate_recovery_id "${restore_id}" || { echo "restore recovery ID must be a lowercase DNS label" >&2; exit 1; }
  [ -n "${restore_id}" ] && [ "${restore_id}" = "${recovery_id}" ] || {
    echo "restore authorization does not match the lifecycle recovery ID" >&2
    exit 1
  }
  restore_value=$(sed -n '1p' "${RESTORE_VALUE_FILE}")
  validate_restore_directive "${restore_id}" "${restore_value}" \
    || { echo "restore directive must reference the authorized lowercase SHA-256 Grant recovery object" >&2; exit 1; }

  if [ "${marker_present}" -eq 1 ]; then
    [ "${marker_id}" = "${restore_id}" ] && [ "${marker_restore}" = "${restore_value}" ] \
      || { echo "restore authorization does not match persisted recovery state" >&2; exit 1; }
  fi
  case "${marker_state}" in
    completed)
      echo "restore-once wrapper suppressing replay for ${restore_id}"
      exec "${EXECUTABLE}" "$@"
      ;;
    ''|pending) ;;
  esac

  HQL_BACKUP_RESTORE=${restore_value}
  HQL_S3_KEY=$(sed -n '1p' "${RESTORE_S3_KEY_FILE}")
  HQL_S3_SECRET=$(sed -n '1p' "${RESTORE_S3_SECRET_FILE}")
  [ -n "${HQL_BACKUP_RESTORE}" ] && [ -n "${HQL_S3_KEY}" ] && [ -n "${HQL_S3_SECRET}" ] \
    || { echo "empty restore directive or reader credential" >&2; exit 1; }
  command -v "${PROBE_EXEC}" >/dev/null 2>&1 || { echo "missing restore readiness probe: ${PROBE_EXEC}" >&2; exit 1; }

  write_marker pending "${restore_id}" "${restore_value}"
  restore_binding=${restore_value}
  export HQL_BACKUP_RESTORE HQL_S3_KEY HQL_S3_SECRET
  SPAWNING_CHILD=1
  "${EXECUTABLE}" "$@" &
  if [ -n "${SPAWN_HOOK}" ]; then "${SPAWN_HOOK}"; fi
  CHILD_PID=$!
  SPAWNING_CHILD=0
  if [ -n "${PENDING_SIGNAL}" ]; then
    forward_signal "${PENDING_SIGNAL}" "${PENDING_SIGNAL_STATUS}"
  fi
  unset HQL_BACKUP_RESTORE HQL_S3_KEY HQL_S3_SECRET restore_value

  deadline=$(( $(date +%s) + READY_TIMEOUT ))
  consecutive=0
  while :; do
    now=$(date +%s)
    [ "${now}" -lt "${deadline}" ] || break
    if ! kill -0 "${CHILD_PID}" 2>/dev/null; then
      wait_child
      exit "${child_status}"
    fi
    probe_succeeded=0
    if probe_url "${HEALTH_URL}"; then
      now=$(date +%s)
      if [ "${now}" -lt "${deadline}" ] && probe_url "${READY_URL}"; then
        now=$(date +%s)
        if [ "${now}" -lt "${deadline}" ] && kill -0 "${CHILD_PID}" 2>/dev/null; then
          probe_succeeded=1
        fi
      fi
    fi
    if [ "${probe_succeeded}" -eq 1 ]; then
      consecutive=$((consecutive + 1))
    else
      consecutive=0
    fi
    if [ "${consecutive}" -ge "${READY_SUCCESSES}" ] && kill -0 "${CHILD_PID}" 2>/dev/null; then
        write_marker completed "${restore_id}" "${restore_binding}"
        echo "restore-once wrapper completed authorization ${restore_id}"
        wait_child
        exit "${child_status}"
    fi
    now=$(date +%s)
    [ "${now}" -lt "${deadline}" ] || break
    remaining=$((deadline - now))
    sleep_seconds=${READY_INTERVAL}
    if [ "${remaining}" -lt "${sleep_seconds}" ]; then sleep_seconds=${remaining}; fi
    sleep "${sleep_seconds}"
  done

  echo "restore-once readiness timed out for ${restore_id}" >&2
  stop_child TERM
  exit 124
fi

if [ "${marker_present}" -eq 1 ]; then
  case "${marker_state}" in
    pending) echo "pending restore requires the matching authorization mount" >&2; exit 1 ;;
    completed)
      if [ "${lifecycle_mode}" = restore ] && [ "${marker_id}" != "${recovery_id}" ]; then
        echo "completed restore state does not match lifecycle recovery ID" >&2
        exit 1
      fi
      ;;
  esac
elif [ "${lifecycle_mode}" = restore ]; then
  echo "restore mode requires authorization or matching completed state" >&2
  exit 1
fi

exec "${EXECUTABLE}" "$@"
