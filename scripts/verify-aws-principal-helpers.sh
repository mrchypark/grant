#!/bin/sh
set -eu

ROOT_DIR=$(CDPATH='' cd -P "$(dirname "$0")/.." && pwd)
TMP_DIR=$(mktemp -d "${TMPDIR:-/tmp}/aws-principal-test.XXXXXX")
cleanup() { rm -rf "${TMP_DIR}"; }
trap cleanup 0 HUP INT TERM

mkdir -p "${TMP_DIR}/bin"
AWS_LOG=${TMP_DIR}/aws.log
export AWS_LOG
# shellcheck disable=SC2016
printf '%s\n' '#!/bin/sh' \
  'printf "%s|%s\n" "${AWS_PROFILE-}" "$*" >>"${AWS_LOG}"' \
  '[ "$1" = iam ] && [ "$2" = get-role ] && [ "$3" = --role-name ] || exit 91' \
  'role=$4' \
  'case "${AWS_PROFILE-}:${role}" in' \
  '  grant:reader) printf "%s\n" arn:aws:iam::123456789012:role/platform/recovery/reader ;;' \
  '  grant:writer) printf "%s\n" arn:aws:iam::123456789012:role/platform/recovery/writer ;;' \
  '  other:reader) printf "%s\n" arn:aws:iam::210987654321:role/platform/recovery/reader ;;' \
  '  gov:reader) printf "%s\n" arn:aws-us-gov:iam::123456789012:role/platform/recovery/reader ;;' \
  '  mismatch:reader) printf "%s\n" arn:aws:iam::999999999999:role/platform/recovery/reader ;;' \
  '  fail:reader) exit 92 ;;' \
  '  *) exit 93 ;;' \
  'esac' >"${TMP_DIR}/bin/aws"
chmod 0700 "${TMP_DIR}/bin/aws"
PATH="${TMP_DIR}/bin:${PATH}"
export PATH

# shellcheck source=aws-principal-helpers.sh
# shellcheck disable=SC1091
. "${ROOT_DIR}/scripts/aws-principal-helpers.sh"

assert_equal() {
  expected=$1
  actual=$2
  label=$3
  [ "${actual}" = "${expected}" ] || {
    echo "${label}: expected ${expected}, got ${actual}" >&2
    exit 1
  }
}

assert_distinct() {
  left=$1
  right=$2
  label=$3
  [ "${left}" != "${right}" ] || {
    echo "${label}: identities unexpectedly collapsed to ${left}" >&2
    exit 1
  }
}

expect_rejected() {
  label=$1
  principal=$2
  profile=$3
  if canonical_aws_role_principal "${principal}" "${profile}" >"${TMP_DIR}/${label}.out" 2>"${TMP_DIR}/${label}.err"; then
    echo "${label}: invalid or unresolvable assumed-role principal was accepted" >&2
    exit 1
  fi
}

static_role='arn:aws:iam::123456789012:role/platform/recovery/reader'
session_one='arn:aws:sts::123456789012:assumed-role/reader/irsa-session'
session_two='arn:aws:sts::123456789012:assumed-role/reader/sso-session'

canonical_static=$(canonical_aws_role_principal "${static_role}" grant)
canonical_session_one=$(canonical_aws_role_principal "${session_one}" grant)
canonical_session_two=$(canonical_aws_role_principal "${session_two}" grant)
assert_equal "${static_role}" "${canonical_static}" 'static IAM role preservation'
assert_equal "${static_role}" "${canonical_session_one}" 'authoritative static-to-assumed role equivalence'
assert_equal "${canonical_session_one}" "${canonical_session_two}" 'assumed-role session suffix equivalence'

wrong_expected_path='arn:aws:iam::123456789012:role/wrong/recovery/reader'
assert_distinct "${canonical_session_one}" "$(canonical_aws_role_principal "${wrong_expected_path}" grant)" \
  'same role name with a wrong signed IAM path'

different_role=$(canonical_aws_role_principal 'arn:aws:sts::123456789012:assumed-role/writer/session-name' grant)
different_account=$(canonical_aws_role_principal 'arn:aws:sts::210987654321:assumed-role/reader/session-name' other)
assert_distinct "${canonical_session_one}" "${different_role}" 'different role names'
assert_distinct "${canonical_session_one}" "${different_account}" 'different accounts'

gov_role='arn:aws-us-gov:iam::123456789012:role/platform/recovery/reader'
gov_session='arn:aws-us-gov:sts::123456789012:assumed-role/reader/session-name'
assert_equal "${gov_role}" "$(canonical_aws_role_principal "${gov_session}" gov)" 'partition preservation'

expect_rejected lookup-failure "${session_one}" fail
expect_rejected lookup-account-mismatch "${session_one}" mismatch
expect_rejected missing-profile "${session_one}" ''
expect_rejected role-path-in-sts 'arn:aws:sts::123456789012:assumed-role/platform/reader/session-name' grant
expect_rejected one-character-session 'arn:aws:sts::123456789012:assumed-role/reader/x' grant
expect_rejected long-session "arn:aws:sts::123456789012:assumed-role/reader/$(printf '%065d' 0)" grant
expect_rejected long-role "arn:aws:sts::123456789012:assumed-role/$(printf '%065d' 0)/session-name" grant
expect_rejected malformed-account 'arn:aws:sts::12345678901:assumed-role/reader/session-name' grant

iam_user='arn:aws:iam::123456789012:user/platform/recovery/reader'
federated_user='arn:aws:sts::123456789012:federated-user/reader'
assert_equal "${iam_user}" "$(canonical_aws_role_principal "${iam_user}" grant)" 'IAM user preservation'
assert_equal "${federated_user}" "$(canonical_aws_role_principal "${federated_user}" grant)" 'federated user preservation'

[ "$(wc -l <"${AWS_LOG}" | tr -d ' ')" -eq 7 ] || {
  echo "unexpected iam:GetRole invocation count" >&2
  exit 1
}

echo 'AWS principal authoritative role resolution verified'
