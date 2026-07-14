#!/bin/sh
set -eu

ROOT_DIR=$(CDPATH='' cd -P "$(dirname "$0")/.." && pwd)
TMP_DIR=$(mktemp -d "${TMPDIR:-/tmp}/restore-once-test.XXXXXX")
cleanup() { rm -rf "${TMP_DIR}"; }
trap cleanup 0 HUP INT TERM

mkdir -p "${TMP_DIR}/restore" "${TMP_DIR}/lifecycle" "${TMP_DIR}/data"
RESTORE_SHA=aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa
VALID_RESTORE="s3:restore/grant/recovery-test/${RESTORE_SHA}.enc"
printf '%s\n' recovery-test >"${TMP_DIR}/restore/recovery-id"
printf '%s\n' recovery-test >"${TMP_DIR}/lifecycle/recovery-id"
printf '%s\n' "${VALID_RESTORE}" >"${TMP_DIR}/restore/HQL_BACKUP_RESTORE"
printf '%s\n' test-reader-key >"${TMP_DIR}/restore/HQL_S3_KEY"
printf '%s\n' test-reader-secret >"${TMP_DIR}/restore/HQL_S3_SECRET"
# The inner variables intentionally expand only when the generated probe runs.
# shellcheck disable=SC2016
printf '%s\n' '#!/bin/sh' 'printf "%s|%s|%s\n" "${HQL_BACKUP_RESTORE-unset}" "${HQL_S3_KEY-unset}" "${HQL_S3_SECRET-unset}" >"${CAPTURE}"' >"${TMP_DIR}/capture.sh"
chmod 0700 "${TMP_DIR}/capture.sh"

run_wrapper() {
  CAPTURE=$1 RESTORE_ONCE_EXEC="${TMP_DIR}/capture.sh" RESTORE_ONCE_MARKER="${TMP_DIR}/data/consumed" \
    RESTORE_ONCE_VALUE_FILE="${TMP_DIR}/restore/HQL_BACKUP_RESTORE" RESTORE_ONCE_ID_FILE="${TMP_DIR}/restore/recovery-id" \
    RESTORE_ONCE_S3_KEY_FILE="${TMP_DIR}/restore/HQL_S3_KEY" RESTORE_ONCE_S3_SECRET_FILE="${TMP_DIR}/restore/HQL_S3_SECRET" \
    RESTORE_ONCE_RECOVERY_ID_FILE="${TMP_DIR}/lifecycle/recovery-id" \
    "${ROOT_DIR}/deploy/k8s/restore-once.sh"
}

run_wrapper "${TMP_DIR}/first"
[ "$(cat "${TMP_DIR}/first")" = "${VALID_RESTORE}|test-reader-key|test-reader-secret" ] || { echo "first restore authorization was not consumed with reader credentials" >&2; exit 1; }
run_wrapper "${TMP_DIR}/second"
[ "$(cat "${TMP_DIR}/second")" = 'unset|unset|unset' ] || { echo "restore authorization replay was not suppressed" >&2; exit 1; }

expect_rejected() {
  label=$1
  restore_id=$2
  lifecycle_id=$3
  directive=$4
  rm -rf "${TMP_DIR}/data/consumed"
  printf '%s\n' "${restore_id}" >"${TMP_DIR}/restore/recovery-id"
  printf '%s\n' "${lifecycle_id}" >"${TMP_DIR}/lifecycle/recovery-id"
  printf '%s\n' "${directive}" >"${TMP_DIR}/restore/HQL_BACKUP_RESTORE"
  if run_wrapper "${TMP_DIR}/${label}" >/dev/null 2>&1; then
    echo "invalid restore authorization was accepted: ${label}" >&2
    exit 1
  fi
  [ ! -e "${TMP_DIR}/data/consumed" ] || {
    echo "invalid restore authorization created the replay marker: ${label}" >&2
    exit 1
  }
}

expect_rejected stale-recovery stale-recovery recovery-test \
  "s3:restore/grant/stale-recovery/${RESTORE_SHA}.enc"
expect_rejected other-recovery recovery-test recovery-test \
  "s3:restore/grant/other-recovery/${RESTORE_SHA}.enc"
expect_rejected cross-workload recovery-test recovery-test \
  "s3:restore/rauthy/recovery-test/${RESTORE_SHA}.enc"
expect_rejected live-backup recovery-test recovery-test \
  's3:backup_node_1_1.sqlite'
expect_rejected uppercase-sha recovery-test recovery-test \
  's3:restore/grant/recovery-test/AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA.enc'
expect_rejected path-characters recovery-test recovery-test \
  "s3:restore/grant/recovery-test/${RESTORE_SHA}/extra.enc"
expect_rejected short-sha recovery-test recovery-test \
  's3:restore/grant/recovery-test/abcd.enc'

rm -rf "${TMP_DIR}/data/consumed"
printf '%s\n' recovery-test >"${TMP_DIR}/restore/recovery-id"
printf '%s\n' recovery-test >"${TMP_DIR}/lifecycle/recovery-id"
printf '%s\n' "${VALID_RESTORE}" >"${TMP_DIR}/restore/HQL_BACKUP_RESTORE"
rm -f "${TMP_DIR}/restore/HQL_S3_SECRET"
if run_wrapper "${TMP_DIR}/incomplete-reader" >/dev/null 2>&1; then
  echo "restore authorization without both reader credentials was accepted" >&2
  exit 1
fi

echo "restore-once recovery binding and replay protection verified"
