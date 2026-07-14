#!/bin/sh
set -eu

ROOT_DIR=$(CDPATH='' cd -P "$(dirname "$0")/.." && pwd)
TMP_DIR=$(mktemp -d "${TMPDIR:-/tmp}/grant-s3-history-test.XXXXXX")
cleanup() { rm -rf "${TMP_DIR}"; }
trap cleanup 0 HUP INT TERM

TARGET_KEY=restore/rauthy/recovery-test/aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa.enc
EXPECTED_VERSION=expected-version
SECRET_SENTINEL=must-not-appear-in-output

mkdir -p "${TMP_DIR}/bin"
cat >"${TMP_DIR}/bin/aws" <<'MOCK_AWS'
#!/bin/sh
set -eu

[ "$1" = "--endpoint-url" ]
[ "$2" = "https://s3.example.test" ]
[ "$3" = "s3api" ]
[ -n "${4:-}" ]

if [ "$4" = "get-object-lock-configuration" ]; then
  [ "$5" = "--bucket" ]
  [ "$6" = "test-bucket" ]
  [ "$7" = "--query" ]
  [ "$8" = "ObjectLockConfiguration.ObjectLockEnabled" ]
  [ "$9" = "--output" ]
  [ "${10}" = "text" ]
  case "${MOCK_CASE}" in
    lock-command-failure)
      echo "mock object lock API unavailable" >&2
      exit 42
      ;;
    lock-disabled)
      printf 'Disabled\n'
      exit 0
      ;;
    *) exit 1 ;;
  esac
fi

[ "$4" = "list-object-versions" ]
shift 4

bucket=
prefix=
while [ "$#" -gt 0 ]; do
  case "$1" in
    --bucket) bucket=$2; shift 2 ;;
    --prefix) prefix=$2; shift 2 ;;
    --output) [ "$2" = json ]; shift 2 ;;
    *) echo "unexpected mock AWS argument: $1" >&2; exit 1 ;;
  esac
done
[ "${bucket}" = test-bucket ]
[ "${prefix}" = "${MOCK_TARGET_KEY}" ]

case "${MOCK_CASE}" in
  empty)
    printf '{"Versions":[{"Key":"%s.suffix","VersionId":"collision"}],"DeleteMarkers":[],"IsTruncated":false}\n' "${MOCK_TARGET_KEY}"
    ;;
  existing-version)
    printf '{"Versions":[{"Key":"%s","VersionId":"old-version"}],"DeleteMarkers":[],"IsTruncated":false}\n' "${MOCK_TARGET_KEY}"
    ;;
  delete-marker)
    printf '{"Versions":[{"Key":"%s.suffix","VersionId":"collision"}],"DeleteMarkers":[{"Key":"%s","VersionId":"deleted-version"}]}\n' "${MOCK_TARGET_KEY}" "${MOCK_TARGET_KEY}"
    ;;
  postcondition)
    printf '{"Versions":[{"Key":"%s.suffix","VersionId":"collision"},{"Key":"%s","VersionId":"%s"}],"DeleteMarkers":[]}\n' "${MOCK_TARGET_KEY}" "${MOCK_TARGET_KEY}" "${MOCK_EXPECTED_VERSION}"
    ;;
  invalid-response)
    printf '{"Versions":{},"DeleteMarkers":[]}'
    ;;
  *) exit 1 ;;
esac
MOCK_AWS
chmod +x "${TMP_DIR}/bin/aws"

run_check() {
  mock_case=$1
  shift
  PATH="${TMP_DIR}/bin:${PATH}" \
    S3_ENDPOINT_URL=https://s3.example.test \
    AWS_SECRET_ACCESS_KEY=${SECRET_SENTINEL} \
    MOCK_CASE=${mock_case} \
    MOCK_TARGET_KEY=${TARGET_KEY} \
    MOCK_EXPECTED_VERSION=${EXPECTED_VERSION} \
    "${ROOT_DIR}/scripts/check-s3-object-history.sh" test-bucket "${TARGET_KEY}" "$@"
}

run_check empty
run_check postcondition "${EXPECTED_VERSION}"

for failure_case in existing-version delete-marker invalid-response; do
  if run_check "${failure_case}" >"${TMP_DIR}/${failure_case}.out" 2>&1; then
    echo "S3 history mock unexpectedly accepted ${failure_case}" >&2
    exit 1
  fi
  if grep -Fq "${SECRET_SENTINEL}" "${TMP_DIR}/${failure_case}.out"; then
    echo "S3 history check exposed an AWS secret" >&2
    exit 1
  fi
done

if run_check postcondition wrong-version >"${TMP_DIR}/wrong-version.out" 2>&1; then
  echo "S3 history postcondition accepted the wrong version" >&2
  exit 1
fi

future_retain_until() {
  if value=$(date -u -d '+7 days' '+%Y-%m-%dT%H:%M:%SZ' 2>/dev/null); then
    printf '%s\n' "${value}"
  elif value=$(date -j -u -v+7d '+%Y-%m-%dT%H:%M:%SZ' 2>/dev/null); then
    printf '%s\n' "${value}"
  else
    echo "cannot calculate a future Object Lock retention timestamp" >&2
    return 1
  fi
}

RETAIN_UNTIL=$(future_retain_until)

run_materialize() {
  mock_case=$1
  PATH="${TMP_DIR}/bin:${PATH}" \
    S3_ENDPOINT_URL=https://s3.example.test \
    AWS_SECRET_ACCESS_KEY=${SECRET_SENTINEL} \
    MOCK_CASE=${mock_case} \
    "${ROOT_DIR}/scripts/materialize-s3-restore.sh" \
    grant test-bucket source-key source-version \
    aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa \
    recovery-test "${RETAIN_UNTIL}" \
    arn:aws:iam::111111111111:role/materializer "${TMP_DIR}/materialized.tsv"
}

if run_materialize lock-command-failure >"${TMP_DIR}/lock-command-failure.out" 2>&1; then
  echo "materializer accepted an Object Lock API failure" >&2
  exit 1
fi
grep -q 'mock object lock API unavailable' "${TMP_DIR}/lock-command-failure.out" \
  || { echo "materializer hid the AWS Object Lock command error" >&2; exit 1; }
grep -q 'failed to query bucket Object Lock configuration (aws exit 42): test-bucket' \
  "${TMP_DIR}/lock-command-failure.out" \
  || { echo "materializer did not report the Object Lock command exit" >&2; exit 1; }

if run_materialize lock-disabled >"${TMP_DIR}/lock-disabled.out" 2>&1; then
  echo "materializer accepted a disabled Object Lock response" >&2
  exit 1
fi
grep -q 'bucket Object Lock is not enabled: test-bucket' "${TMP_DIR}/lock-disabled.out" \
  || { echo "materializer did not distinguish disabled Object Lock" >&2; exit 1; }
if grep -q 'failed to query bucket Object Lock configuration' "${TMP_DIR}/lock-disabled.out"; then
  echo "materializer confused disabled Object Lock with an AWS command failure" >&2
  exit 1
fi
for output in "${TMP_DIR}/lock-command-failure.out" "${TMP_DIR}/lock-disabled.out"; do
  if grep -Fq "${SECRET_SENTINEL}" "${output}"; then
    echo "materializer exposed an AWS secret while checking Object Lock" >&2
    exit 1
  fi
done

echo "S3 object-version history pagination verified"
