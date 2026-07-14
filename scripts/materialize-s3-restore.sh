#!/bin/sh
set -eu

SCRIPT_DIR=$(CDPATH='' cd -P "$(dirname "$0")" && pwd)
DRY_RUN=0
if [ "${1:-}" = "--dry-run" ]; then
  DRY_RUN=1
  shift
fi

if [ "$#" -ne 9 ]; then
  echo "usage: $0 [--dry-run] WORKLOAD BUCKET SOURCE_KEY VERSION_ID SHA256 RECOVERY_ID RETAIN_UNTIL WRITER_PRINCIPAL OUTPUT" >&2
  exit 2
fi

WORKLOAD=$1
BUCKET=$2
SOURCE_KEY=$3
SOURCE_VERSION=$4
SOURCE_SHA=$5
RECOVERY_ID=$6
RETAIN_UNTIL=$7
WRITER_PRINCIPAL=$8
OUTPUT=$9
S3_ENDPOINT_URL=${S3_ENDPOINT_URL:-}
S3_POLL_TIMEOUT_SECONDS=${S3_POLL_TIMEOUT_SECONDS:-600}
MIN_RETENTION_SECONDS=${MIN_RETENTION_SECONDS:-86400}

case "${WORKLOAD}" in grant|rauthy) ;; *) echo "invalid workload: ${WORKLOAD}" >&2; exit 2 ;; esac
case "${RECOVERY_ID}" in ''|*[!a-z0-9-]*|-*|*-) echo "recovery id must be lowercase DNS-label characters" >&2; exit 2 ;; esac
[ "${#RECOVERY_ID}" -le 32 ] || { echo "recovery id must be at most 32 characters" >&2; exit 2; }
case "${SOURCE_SHA}" in *[!0-9a-fA-F]*|'') echo "SHA256 must be hexadecimal" >&2; exit 2 ;; esac
[ "${#SOURCE_SHA}" -eq 64 ] || { echo "SHA256 must contain 64 hex characters" >&2; exit 2; }
SOURCE_SHA=$(printf '%s' "${SOURCE_SHA}" | tr 'A-F' 'a-f')
case "${BUCKET}${SOURCE_KEY}${SOURCE_VERSION}${RETAIN_UNTIL}" in *'
'*) echo "metadata must not contain newlines" >&2; exit 2 ;; esac

TARGET_KEY="restore/${WORKLOAD}/${RECOVERY_ID}/${SOURCE_SHA}.enc"
POLICY_SID="GrantRecoveryImmutable-${WORKLOAD}-${RECOVERY_ID}"
WRITER_DENY_SID="GrantRecoveryWriterReadDeny-${WORKLOAD}-${RECOVERY_ID}"
TARGET_ARN="arn:aws:s3:::${BUCKET}/${TARGET_KEY}"

if [ "${DRY_RUN}" -eq 1 ]; then
  printf '%s\t%s\t%s\t%s\t%s\t%s\t%s\n' "${TARGET_KEY}" "dry-run-version" "${SOURCE_SHA}" "${RETAIN_UNTIL}" "${POLICY_SID}" "${WRITER_DENY_SID}" "dry-run-${WORKLOAD}-principal" >"${OUTPUT}"
  echo "dry-run: would materialize s3://${BUCKET}/${SOURCE_KEY}?versionId=${SOURCE_VERSION} as immutable s3://${BUCKET}/${TARGET_KEY}" >&2
  exit 0
fi

command -v aws >/dev/null 2>&1 || { echo "missing command: aws" >&2; exit 1; }
command -v mktemp >/dev/null 2>&1 || { echo "missing command: mktemp" >&2; exit 1; }
command -v jq >/dev/null 2>&1 || { echo "missing command: jq" >&2; exit 1; }
[ -x "${SCRIPT_DIR}/check-s3-object-history.sh" ] || { echo "missing executable: check-s3-object-history.sh" >&2; exit 1; }

aws_s3api() {
  if [ -n "${S3_ENDPOINT_URL}" ]; then
    aws --endpoint-url "${S3_ENDPOINT_URL}" s3api "$@"
  else
    aws s3api "$@"
  fi
}

sha256_file() {
  if command -v sha256sum >/dev/null 2>&1; then
    sha256sum "$1" | awk '{print $1}'
  elif command -v shasum >/dev/null 2>&1; then
    shasum -a 256 "$1" | awk '{print $1}'
  else
    echo "missing sha256sum or shasum" >&2
    return 1
  fi
}

epoch_from_rfc3339() {
  value=$1
  if date -u -d "${value}" +%s >/dev/null 2>&1; then
    date -u -d "${value}" +%s
  else
    date -j -u -f '%Y-%m-%dT%H:%M:%SZ' "${value}" +%s
  fi
}

retain_epoch=$(epoch_from_rfc3339 "${RETAIN_UNTIL}") || {
  echo "RETAIN_UNTIL must be a valid UTC RFC3339 timestamp" >&2
  exit 2
}
now_epoch=$(date -u +%s)
[ "${retain_epoch}" -ge "$((now_epoch + MIN_RETENTION_SECONDS))" ] || {
  echo "RETAIN_UNTIL must be at least ${MIN_RETENTION_SECONDS}s in the future" >&2
  exit 2
}

TMP_DIR=$(mktemp -d "${TMPDIR:-/tmp}/grant-materialize.XXXXXX")
cleanup() { rm -rf "${TMP_DIR}"; }
trap cleanup 0 HUP INT TERM

if LOCK_ENABLED=$(aws_s3api get-object-lock-configuration \
  --bucket "${BUCKET}" \
  --query 'ObjectLockConfiguration.ObjectLockEnabled' \
  --output text); then
  :
else
  lock_status=$?
  echo "failed to query bucket Object Lock configuration (aws exit ${lock_status}): ${BUCKET}" >&2
  exit 1
fi
[ "${LOCK_ENABLED}" = "Enabled" ] || { echo "bucket Object Lock is not enabled: ${BUCKET}" >&2; exit 1; }
VERSIONING_STATUS=$(aws_s3api get-bucket-versioning \
  --bucket "${BUCKET}" \
  --query Status \
  --output text)
[ "${VERSIONING_STATUS}" = "Enabled" ] || { echo "bucket versioning is not enabled: ${BUCKET}" >&2; exit 1; }
CALLER_ARN=$(aws sts get-caller-identity --query Arn --output text)
[ -n "${CALLER_ARN}" ] && [ "${CALLER_ARN}" != None ] || { echo "cannot identify AWS materialization principal" >&2; exit 1; }

S3_ENDPOINT_URL="${S3_ENDPOINT_URL}" \
  "${SCRIPT_DIR}/check-s3-object-history.sh" "${BUCKET}" "${TARGET_KEY}"

elapsed=0
until aws_s3api head-object --bucket "${BUCKET}" --key "${SOURCE_KEY}" --version-id "${SOURCE_VERSION}" >/dev/null 2>&1; do
  [ "${elapsed}" -lt "${S3_POLL_TIMEOUT_SECONDS}" ] || {
    echo "timed out polling asynchronous S3 backup version: s3://${BUCKET}/${SOURCE_KEY}?versionId=${SOURCE_VERSION}" >&2
    exit 1
  }
  sleep 5
  elapsed=$((elapsed + 5))
done

aws_s3api get-object \
  --bucket "${BUCKET}" \
  --key "${SOURCE_KEY}" \
  --version-id "${SOURCE_VERSION}" \
  "${TMP_DIR}/source.enc" >/dev/null

ACTUAL_SOURCE_SHA=$(sha256_file "${TMP_DIR}/source.enc")
[ "${ACTUAL_SOURCE_SHA}" = "${SOURCE_SHA}" ] || {
  echo "source checksum mismatch: expected ${SOURCE_SHA}, got ${ACTUAL_SOURCE_SHA}" >&2
  exit 1
}

TARGET_VERSION=$(aws_s3api put-object \
  --bucket "${BUCKET}" \
  --key "${TARGET_KEY}" \
  --if-none-match '*' \
  --body "${TMP_DIR}/source.enc" \
  --object-lock-mode COMPLIANCE \
  --object-lock-retain-until-date "${RETAIN_UNTIL}" \
  --query VersionId \
  --output text)
[ -n "${TARGET_VERSION}" ] && [ "${TARGET_VERSION}" != "None" ] || {
  echo "destination object has no version id; bucket versioning is required" >&2
  exit 1
}
S3_ENDPOINT_URL="${S3_ENDPOINT_URL}" \
  "${SCRIPT_DIR}/check-s3-object-history.sh" "${BUCKET}" "${TARGET_KEY}" "${TARGET_VERSION}"

POLICY_ERROR_FILE="${TMP_DIR}/get-bucket-policy.stderr"
if EXISTING_POLICY=$(LC_ALL=C aws_s3api get-bucket-policy --bucket "${BUCKET}" --query Policy --output text 2>"${POLICY_ERROR_FILE}"); then
  printf '%s\n' "${EXISTING_POLICY}" >"${TMP_DIR}/bucket-policy.json"
else
  policy_status=$?
  # AWS CLI service failures share one exit status and expose the service error
  # code only in this canonical prefix. Match the exact code, never message text.
  policy_error_code=$(sed -n 's/^An error occurred (\([^)]*\)) when calling the GetBucketPolicy operation:.*$/\1/p' "${POLICY_ERROR_FILE}" | sed -n '1p')
  if [ "${policy_status}" -eq 254 ] && [ "${policy_error_code}" = NoSuchBucketPolicy ]; then
    printf '%s\n' '{"Version":"2012-10-17","Statement":[]}' >"${TMP_DIR}/bucket-policy.json"
  else
    cat "${POLICY_ERROR_FILE}" >&2
    echo "failed to query bucket policy (aws exit ${policy_status}): ${BUCKET}" >&2
    exit 1
  fi
fi
jq --arg sid "${POLICY_SID}" --arg writerSid "${WRITER_DENY_SID}" --arg writer "${WRITER_PRINCIPAL}" --arg resource "${TARGET_ARN}" '
  .Statement = (
    if .Statement == null then []
    elif (.Statement | type) == "array" then .Statement
    elif (.Statement | type) == "object" then [.Statement]
    else error("bucket policy Statement must be an object, array, or null")
    end
  ) |
  if any(.Statement[]; .Sid == $sid or .Sid == $writerSid) then error("recovery policy SID already exists")
  else .Statement += [{
    Sid:$sid,
    Effect:"Deny",
    Principal:"*",
    Action:["s3:PutObject","s3:DeleteObject","s3:DeleteObjectVersion"],
    Resource:$resource
  },{
    Sid:$writerSid,
    Effect:"Deny",
    Principal:{AWS:$writer},
    Action:["s3:GetObject","s3:GetObjectVersion"],
    Resource:$resource
  }] end
' "${TMP_DIR}/bucket-policy.json" >"${TMP_DIR}/bucket-policy-updated.json"
aws_s3api put-bucket-policy --bucket "${BUCKET}" --policy "file://${TMP_DIR}/bucket-policy-updated.json"

INSTALLED_POLICY=$(aws_s3api get-bucket-policy --bucket "${BUCKET}" --query Policy --output text)
printf '%s\n' "${INSTALLED_POLICY}" | jq -e --arg sid "${POLICY_SID}" --arg writerSid "${WRITER_DENY_SID}" --arg writer "${WRITER_PRINCIPAL}" --arg resource "${TARGET_ARN}" '
  def actions: (.Action | if type == "array" then . else [.] end);
  any(.Statement[]?;
    .Sid == $sid and .Effect == "Deny" and .Principal == "*" and .Resource == $resource and
    (actions | index("s3:PutObject")) != null and
    (actions | index("s3:DeleteObject")) != null and
    (actions | index("s3:DeleteObjectVersion")) != null) and
  any(.Statement[]?; .Sid == $writerSid and .Effect == "Deny" and .Principal.AWS == $writer and .Resource == $resource and
    (actions | index("s3:GetObject")) != null and (actions | index("s3:GetObjectVersion")) != null)
' >/dev/null || { echo "immutable exact-key bucket policy was not installed" >&2; exit 1; }

S3_ENDPOINT_URL="${S3_ENDPOINT_URL}" \
  "${SCRIPT_DIR}/check-s3-object-history.sh" "${BUCKET}" "${TARGET_KEY}" "${TARGET_VERSION}"

HEAD_VALUES=$(aws_s3api head-object \
  --bucket "${BUCKET}" \
  --key "${TARGET_KEY}" \
  --version-id "${TARGET_VERSION}" \
  --query '[VersionId,ObjectLockMode,ObjectLockRetainUntilDate]' \
  --output text)
CURRENT_VERSION=$(printf '%s\n' "${HEAD_VALUES}" | awk '{print $1}')
LOCK_MODE=$(printf '%s\n' "${HEAD_VALUES}" | awk '{print $2}')
LOCK_UNTIL=$(printf '%s\n' "${HEAD_VALUES}" | awk '{print $3}')
[ "${CURRENT_VERSION}" = "${TARGET_VERSION}" ] || { echo "destination current version changed before immutability was sealed" >&2; exit 1; }
[ "${LOCK_MODE}" = "COMPLIANCE" ] || { echo "destination object is not COMPLIANCE locked" >&2; exit 1; }
[ -n "${LOCK_UNTIL}" ] && [ "${LOCK_UNTIL}" != "None" ] || { echo "destination lock retention is missing" >&2; exit 1; }
NORMALIZED_LOCK_UNTIL=$(printf '%s\n' "${LOCK_UNTIL}" | sed 's/+00:00$/Z/')
[ "${NORMALIZED_LOCK_UNTIL}" = "${RETAIN_UNTIL}" ] || {
  echo "destination retention differs from requested value: ${LOCK_UNTIL}" >&2
  exit 1
}

aws_s3api get-object \
  --bucket "${BUCKET}" \
  --key "${TARGET_KEY}" \
  --version-id "${TARGET_VERSION}" \
  "${TMP_DIR}/target.enc" >/dev/null
ACTUAL_TARGET_SHA=$(sha256_file "${TMP_DIR}/target.enc")
[ "${ACTUAL_TARGET_SHA}" = "${SOURCE_SHA}" ] || {
  echo "destination checksum mismatch: expected ${SOURCE_SHA}, got ${ACTUAL_TARGET_SHA}" >&2
  exit 1
}

printf '%s\t%s\t%s\t%s\t%s\t%s\t%s\n' \
  "${TARGET_KEY}" "${TARGET_VERSION}" "${ACTUAL_TARGET_SHA}" "${NORMALIZED_LOCK_UNTIL}" "${POLICY_SID}" "${WRITER_DENY_SID}" "${CALLER_ARN}" >"${OUTPUT}"
echo "materialized immutable restore object s3://${BUCKET}/${TARGET_KEY}?versionId=${TARGET_VERSION}" >&2
