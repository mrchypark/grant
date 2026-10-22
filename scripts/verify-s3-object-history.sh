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

case "${MOCK_CASE:-}" in
  policy-*)
    if [ "${1:-}" = sts ]; then
      [ "${2:-}" = get-caller-identity ]
      printf '%s\n' 'arn:aws:iam::111111111111:role/materializer'
      exit 0
    fi

    [ "$1" = "--endpoint-url" ]
    [ "$2" = "https://s3.example.test" ]
    [ "$3" = "s3api" ]
    command=$4
    shift 4

    case "${command}" in
      get-object-lock-configuration|get-bucket-versioning)
        printf '%s\n' Enabled
        ;;
      list-object-versions)
        prefix=
        while [ "$#" -gt 0 ]; do
          case "$1" in
            --bucket) shift 2 ;;
            --prefix) prefix=$2; shift 2 ;;
            --output) shift 2 ;;
            *) exit 81 ;;
          esac
        done
        if [ -f "${MOCK_STATE_DIR}/target-uploaded" ]; then
          printf '{"Versions":[{"Key":"%s","VersionId":"target-version"}],"DeleteMarkers":[],"IsTruncated":false}\n' "${prefix}"
        else
          printf '{"Versions":[],"DeleteMarkers":[],"IsTruncated":false}\n'
        fi
        ;;
      head-object)
        key=
        while [ "$#" -gt 0 ]; do
          case "$1" in
            --bucket|--version-id|--query|--output) shift 2 ;;
            --key) key=$2; shift 2 ;;
            *) exit 82 ;;
          esac
        done
        case "${key}" in
          restore/*)
            printf 'target-version\tCOMPLIANCE\t%s\n' "$(sed -n '1p' "${MOCK_STATE_DIR}/retain-until")"
            ;;
        esac
        ;;
      get-object)
        output=
        for argument do output=${argument}; done
        printf '%s' "${MOCK_SOURCE_CONTENT}" >"${output}"
        printf '%s\n' '{}'
        ;;
      put-object)
        retain_until=
        while [ "$#" -gt 0 ]; do
          case "$1" in
            --object-lock-retain-until-date) retain_until=$2; shift 2 ;;
            --bucket|--key|--if-none-match|--body|--object-lock-mode|--query|--output) shift 2 ;;
            *) exit 83 ;;
          esac
        done
        printf '%s\n' "${retain_until}" >"${MOCK_STATE_DIR}/retain-until"
        : >"${MOCK_STATE_DIR}/target-uploaded"
        printf '%s\n' target-version
        ;;
      get-bucket-policy)
        if [ -f "${MOCK_STATE_DIR}/installed-policy.json" ]; then
          cat "${MOCK_STATE_DIR}/installed-policy.json"
          exit 0
        fi
        case "${MOCK_CASE}" in
          policy-single-object)
            printf '%s\n' '{"Version":"2012-10-17","Id":"keep-single","Statement":{"Sid":"ExistingSingle","Effect":"Allow","Action":"s3:ListBucket","Resource":"arn:aws:s3:::test-bucket"}}'
            ;;
          policy-array)
            printf '%s\n' '{"Version":"2012-10-17","Id":"keep-array","Statement":[{"Sid":"ExistingFirst","Effect":"Allow","Action":"s3:GetBucketLocation","Resource":"arn:aws:s3:::test-bucket"},{"Sid":"ExistingSecond","Effect":"Deny","Action":["s3:DeleteBucket"],"Resource":"arn:aws:s3:::test-bucket"}]}'
            ;;
          policy-missing)
            printf '%s\n' '{"Version":"2012-10-17","Id":"keep-missing"}'
            ;;
          policy-null)
            printf '%s\n' '{"Version":"2012-10-17","Id":"keep-null","Statement":null}'
            ;;
          policy-absent)
            echo 'An error occurred (NoSuchBucketPolicy) when calling the GetBucketPolicy operation: The bucket policy does not exist' >&2
            exit 254
            ;;
          policy-access-denied)
            echo 'An error occurred (AccessDenied) when calling the GetBucketPolicy operation: Access Denied' >&2
            exit 254
            ;;
          policy-network-error)
            echo 'Could not connect to the endpoint URL: "https://s3.example.test/test-bucket?policy"' >&2
            exit 255
            ;;
          policy-invalid-scalar)
            printf '%s\n' '{"Version":"2012-10-17","Id":"keep-invalid","Statement":"not-a-policy-statement"}'
            ;;
          *) exit 84 ;;
        esac
        ;;
      put-bucket-policy)
        policy_file=
        while [ "$#" -gt 0 ]; do
          case "$1" in
            --bucket) shift 2 ;;
            --policy) policy_file=${2#file://}; shift 2 ;;
            *) exit 85 ;;
          esac
        done
        [ -s "${policy_file}" ]
        cp "${policy_file}" "${MOCK_STATE_DIR}/installed-policy.json"
        ;;
      *) exit 86 ;;
    esac
    exit 0
    ;;
esac

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
    rauthy test-bucket source-key source-version \
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

POLICY_SOURCE_CONTENT='encrypted policy normalization fixture'
printf '%s' "${POLICY_SOURCE_CONTENT}" >"${TMP_DIR}/policy-source.enc"
if command -v sha256sum >/dev/null 2>&1; then
  POLICY_SOURCE_SHA=$(sha256sum "${TMP_DIR}/policy-source.enc" | awk '{print $1}')
else
  POLICY_SOURCE_SHA=$(shasum -a 256 "${TMP_DIR}/policy-source.enc" | awk '{print $1}')
fi
POLICY_TARGET_KEY="restore/rauthy/recovery-test/${POLICY_SOURCE_SHA}.enc"
POLICY_TARGET_ARN="arn:aws:s3:::test-bucket/${POLICY_TARGET_KEY}"
POLICY_WRITER='arn:aws:iam::111111111111:role/materializer'

run_policy_materialize() {
  policy_case=$1
  policy_state_dir="${TMP_DIR}/${policy_case}"
  mkdir -p "${policy_state_dir}"
  PATH="${TMP_DIR}/bin:${PATH}" \
    S3_ENDPOINT_URL=https://s3.example.test \
    AWS_SECRET_ACCESS_KEY=${SECRET_SENTINEL} \
    MOCK_CASE=${policy_case} \
    MOCK_STATE_DIR=${policy_state_dir} \
    MOCK_SOURCE_CONTENT=${POLICY_SOURCE_CONTENT} \
    "${ROOT_DIR}/scripts/materialize-s3-restore.sh" \
    rauthy test-bucket source-key source-version \
    "${POLICY_SOURCE_SHA}" recovery-test "${RETAIN_UNTIL}" \
    "${POLICY_WRITER}" "${policy_state_dir}/materialized.tsv"
}

for policy_case in policy-single-object policy-array policy-missing policy-null policy-absent; do
  policy_output="${TMP_DIR}/${policy_case}.out"
  run_policy_materialize "${policy_case}" >"${policy_output}" 2>&1
  policy_file="${TMP_DIR}/${policy_case}/installed-policy.json"
  [ -s "${policy_file}" ] || { echo "materializer did not install ${policy_case}" >&2; exit 1; }
  case "${policy_case}" in
    policy-single-object)
      jq -e '
        .Id == "keep-single" and (.Statement | length) == 3 and
        .Statement[0] == {Sid:"ExistingSingle",Effect:"Allow",Action:"s3:ListBucket",Resource:"arn:aws:s3:::test-bucket"}
      ' "${policy_file}" >/dev/null
      ;;
    policy-array)
      jq -e '
        .Id == "keep-array" and (.Statement | length) == 4 and
        .Statement[0] == {Sid:"ExistingFirst",Effect:"Allow",Action:"s3:GetBucketLocation",Resource:"arn:aws:s3:::test-bucket"} and
        .Statement[1] == {Sid:"ExistingSecond",Effect:"Deny",Action:["s3:DeleteBucket"],Resource:"arn:aws:s3:::test-bucket"}
      ' "${policy_file}" >/dev/null
      ;;
    policy-missing)
      jq -e '.Id == "keep-missing" and (.Statement | length) == 2' "${policy_file}" >/dev/null
      ;;
    policy-null)
      jq -e '.Id == "keep-null" and (.Statement | length) == 2' "${policy_file}" >/dev/null
      ;;
    policy-absent)
      jq -e 'keys == ["Statement", "Version"] and (.Statement | length) == 2' "${policy_file}" >/dev/null
      ;;
  esac
  jq -e --arg resource "${POLICY_TARGET_ARN}" --arg writer "${POLICY_WRITER}" '
    .Version == "2012-10-17" and (.Statement | type) == "array" and
    .Statement[-2] == {
      Sid:"GrantRecoveryImmutable-rauthy-recovery-test",
      Effect:"Deny",
      Principal:"*",
      Action:["s3:PutObject","s3:DeleteObject","s3:DeleteObjectVersion"],
      Resource:$resource
    } and
    .Statement[-1] == {
      Sid:"GrantRecoveryWriterReadDeny-rauthy-recovery-test",
      Effect:"Deny",
      Principal:{AWS:$writer},
      Action:["s3:GetObject","s3:GetObjectVersion"],
      Resource:$resource
    }
  ' "${policy_file}" >/dev/null || { echo "materializer changed the exact ${policy_case} policy contract" >&2; exit 1; }
  if grep -Fq "${SECRET_SENTINEL}" "${policy_output}"; then
    echo "materializer exposed an AWS secret for ${policy_case}" >&2
    exit 1
  fi
done

for policy_failure_case in policy-access-denied policy-network-error; do
  policy_failure_output="${TMP_DIR}/${policy_failure_case}.out"
  if run_policy_materialize "${policy_failure_case}" >"${policy_failure_output}" 2>&1; then
    echo "materializer accepted bucket policy lookup failure ${policy_failure_case}" >&2
    exit 1
  fi
  case "${policy_failure_case}" in
    policy-access-denied)
      grep -q 'An error occurred (AccessDenied) when calling the GetBucketPolicy operation: Access Denied' "${policy_failure_output}" \
        || { echo "materializer hid the AccessDenied bucket policy diagnostic" >&2; exit 1; }
      grep -q 'failed to query bucket policy (aws exit 254): test-bucket' "${policy_failure_output}" \
        || { echo "materializer did not report the AccessDenied exit status" >&2; exit 1; }
      ;;
    policy-network-error)
      grep -q 'Could not connect to the endpoint URL' "${policy_failure_output}" \
        || { echo "materializer hid the network bucket policy diagnostic" >&2; exit 1; }
      grep -q 'failed to query bucket policy (aws exit 255): test-bucket' "${policy_failure_output}" \
        || { echo "materializer did not report the network failure exit status" >&2; exit 1; }
      ;;
  esac
  [ ! -e "${TMP_DIR}/${policy_failure_case}/installed-policy.json" ] \
    || { echo "materializer installed a policy after ${policy_failure_case}" >&2; exit 1; }
  if grep -Fq "${SECRET_SENTINEL}" "${policy_failure_output}"; then
    echo "materializer exposed an AWS secret for ${policy_failure_case}" >&2
    exit 1
  fi
done

invalid_policy_output="${TMP_DIR}/policy-invalid-scalar.out"
if run_policy_materialize policy-invalid-scalar >"${invalid_policy_output}" 2>&1; then
  echo "materializer accepted a scalar bucket policy Statement" >&2
  exit 1
fi
grep -q 'bucket policy Statement must be an object, array, or null' "${invalid_policy_output}" \
  || { echo "materializer scalar Statement failure is not explicit" >&2; exit 1; }
[ ! -e "${TMP_DIR}/policy-invalid-scalar/installed-policy.json" ] \
  || { echo "materializer installed a policy after scalar Statement rejection" >&2; exit 1; }
if grep -Fq "${SECRET_SENTINEL}" "${invalid_policy_output}"; then
  echo "materializer exposed an AWS secret for invalid scalar policy" >&2
  exit 1
fi

echo "S3 object-version history pagination verified"
