#!/bin/sh
set -eu

ROOT_DIR=$(CDPATH='' cd -P "$(dirname "$0")/.." && pwd)
TMP_DIR=$(mktemp -d "${TMPDIR:-/tmp}/restore-once-test.XXXXXX")
cleanup() { rm -rf "${TMP_DIR}"; }
trap cleanup 0 HUP INT TERM

mkdir -p "${TMP_DIR}/restore" "${TMP_DIR}/lifecycle" "${TMP_DIR}/data"
printf '%s\n' recovery-test >"${TMP_DIR}/restore/recovery-id"
printf '%s\n' recovery-test >"${TMP_DIR}/lifecycle/recovery-id"
printf '%s\n' s3:restore/test.enc >"${TMP_DIR}/restore/HQL_BACKUP_RESTORE"
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
[ "$(cat "${TMP_DIR}/first")" = 's3:restore/test.enc|test-reader-key|test-reader-secret' ] || { echo "first restore authorization was not consumed with reader credentials" >&2; exit 1; }
run_wrapper "${TMP_DIR}/second"
[ "$(cat "${TMP_DIR}/second")" = 'unset|unset|unset' ] || { echo "restore authorization replay was not suppressed" >&2; exit 1; }

printf '%s\n' different-recovery >"${TMP_DIR}/restore/recovery-id"
if run_wrapper "${TMP_DIR}/mismatch" >/dev/null 2>&1; then
  echo "mismatched recovery authorization was accepted" >&2
  exit 1
fi

rm -rf "${TMP_DIR}/data/consumed"
printf '%s\n' recovery-test >"${TMP_DIR}/restore/recovery-id"
rm -f "${TMP_DIR}/restore/HQL_S3_SECRET"
if run_wrapper "${TMP_DIR}/incomplete-reader" >/dev/null 2>&1; then
  echo "restore authorization without both reader credentials was accepted" >&2
  exit 1
fi

echo "restore-once replay protection verified"
