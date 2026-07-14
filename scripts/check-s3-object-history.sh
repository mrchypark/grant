#!/bin/sh
set -eu

if [ "$#" -ne 2 ] && [ "$#" -ne 3 ]; then
  echo "usage: $0 BUCKET TARGET_KEY [EXPECTED_VERSION_ID]" >&2
  exit 2
fi

BUCKET=$1
TARGET_KEY=$2
EXPECTED_VERSION=${3:-}
S3_ENDPOINT_URL=${S3_ENDPOINT_URL:-}

command -v aws >/dev/null 2>&1 || { echo "missing command: aws" >&2; exit 1; }
command -v jq >/dev/null 2>&1 || { echo "missing command: jq" >&2; exit 1; }
command -v mktemp >/dev/null 2>&1 || { echo "missing command: mktemp" >&2; exit 1; }

aws_s3api() {
  if [ -n "${S3_ENDPOINT_URL}" ]; then
    aws --endpoint-url "${S3_ENDPOINT_URL}" s3api "$@"
  else
    aws s3api "$@"
  fi
}

TMP_DIR=$(mktemp -d "${TMPDIR:-/tmp}/grant-s3-history.XXXXXX")
cleanup() { rm -rf "${TMP_DIR}"; }
trap cleanup 0 HUP INT TERM

history_file="${TMP_DIR}/history.json"
# AWS CLI pagination remains enabled so all service pages are aggregated before jq evaluates them.
aws_s3api list-object-versions \
  --bucket "${BUCKET}" \
  --prefix "${TARGET_KEY}" \
  --output json >"${history_file}"

jq -e '
  type == "object" and
  ((.Versions // []) | type == "array") and
  ((.DeleteMarkers // []) | type == "array")
' "${history_file}" >/dev/null || {
  echo "invalid S3 object-version history response" >&2
  exit 1
}

version_count=$(jq --arg key "${TARGET_KEY}" '[.Versions[]? | select(.Key == $key)] | length' "${history_file}")
expected_count=$(jq --arg key "${TARGET_KEY}" --arg version "${EXPECTED_VERSION}" \
  '[.Versions[]? | select(.Key == $key and .VersionId == $version)] | length' "${history_file}")
delete_marker_count=$(jq --arg key "${TARGET_KEY}" \
  '[.DeleteMarkers[]? | select(.Key == $key)] | length' "${history_file}")

if [ -z "${EXPECTED_VERSION}" ]; then
  [ "${version_count}" -eq 0 ] && [ "${delete_marker_count}" -eq 0 ] || {
    echo "refusing to reuse restore key with object-version history: s3://${BUCKET}/${TARGET_KEY}" >&2
    exit 1
  }
else
  [ "${version_count}" -eq 1 ] && [ "${expected_count}" -eq 1 ] && [ "${delete_marker_count}" -eq 0 ] || {
    echo "restore key history changed during materialization: s3://${BUCKET}/${TARGET_KEY}" >&2
    exit 1
  }
fi
