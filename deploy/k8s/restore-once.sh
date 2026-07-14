#!/bin/sh
set -eu

EXECUTABLE=${RESTORE_ONCE_EXEC:?RESTORE_ONCE_EXEC is required}
MARKER=${RESTORE_ONCE_MARKER:?RESTORE_ONCE_MARKER is required}
RESTORE_VALUE_FILE=${RESTORE_ONCE_VALUE_FILE:-/restore/HQL_BACKUP_RESTORE}
RESTORE_ID_FILE=${RESTORE_ONCE_ID_FILE:-/restore/recovery-id}
RECOVERY_ID_FILE=${RESTORE_ONCE_RECOVERY_ID_FILE:-/lifecycle/recovery-id}
RESTORE_S3_KEY_FILE=${RESTORE_ONCE_S3_KEY_FILE:-/restore/HQL_S3_KEY}
RESTORE_S3_SECRET_FILE=${RESTORE_ONCE_S3_SECRET_FILE:-/restore/HQL_S3_SECRET}

unset HQL_BACKUP_RESTORE
if [ -r "${RESTORE_VALUE_FILE}" ] || [ -r "${RESTORE_ID_FILE}" ]; then
  [ -r "${RESTORE_VALUE_FILE}" ] && [ -r "${RESTORE_ID_FILE}" ] && [ -r "${RECOVERY_ID_FILE}" ] \
    && [ -r "${RESTORE_S3_KEY_FILE}" ] && [ -r "${RESTORE_S3_SECRET_FILE}" ] || {
    echo "incomplete restore authorization mount" >&2
    exit 1
  }
  restore_id=$(sed -n '1{s/[[:space:]]//g;p;}' "${RESTORE_ID_FILE}")
  recovery_id=$(sed -n '1{s/[[:space:]]//g;p;}' "${RECOVERY_ID_FILE}")
  case "${restore_id}" in ''|*[!a-z0-9-]*|-*|*-) echo "restore recovery ID must be a lowercase DNS label" >&2; exit 1 ;; esac
  [ "${#restore_id}" -le 32 ] || { echo "restore recovery ID must be at most 32 characters" >&2; exit 1; }
  [ -n "${restore_id}" ] && [ "${restore_id}" = "${recovery_id}" ] || {
    echo "restore authorization does not match the lifecycle recovery ID" >&2
    exit 1
  }
  marker_parent=${MARKER%/*}
  [ "${marker_parent}" != "${MARKER}" ] && mkdir -p "${marker_parent}"
  if mkdir "${MARKER}" 2>/dev/null; then
    HQL_BACKUP_RESTORE=$(sed -n '1p' "${RESTORE_VALUE_FILE}")
    HQL_S3_KEY=$(sed -n '1p' "${RESTORE_S3_KEY_FILE}")
    HQL_S3_SECRET=$(sed -n '1p' "${RESTORE_S3_SECRET_FILE}")
    [ -n "${HQL_BACKUP_RESTORE}" ] && [ -n "${HQL_S3_KEY}" ] && [ -n "${HQL_S3_SECRET}" ] \
      || { echo "empty restore directive or reader credential" >&2; exit 1; }
    export HQL_BACKUP_RESTORE HQL_S3_KEY HQL_S3_SECRET
    printf '%s\n' "${restore_id}" >"${MARKER}/recovery-id"
    echo "restore-once wrapper consumed authorization ${restore_id}"
  else
    marker_id=$(sed -n '1p' "${MARKER}/recovery-id" 2>/dev/null || true)
    [ "${marker_id}" = "${restore_id}" ] || {
      echo "restore marker belongs to a different or incomplete recovery" >&2
      exit 1
    }
    echo "restore-once wrapper suppressing replay for ${restore_id}"
  fi
fi

exec "${EXECUTABLE}" "$@"
