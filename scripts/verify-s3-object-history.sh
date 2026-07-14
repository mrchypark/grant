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

echo "S3 object-version history pagination verified"
