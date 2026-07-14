#!/bin/sh

aws_principal_error() {
  printf 'AWS principal resolution failed: %s\n' "$*" >&2
  return 1
}

valid_aws_partition() {
  case ${1-} in
    ''|*[!a-z0-9-]*) return 1 ;;
    *) return 0 ;;
  esac
}

valid_aws_account_id() {
  case ${1-} in
    ''|*[!0-9]*) return 1 ;;
  esac
  [ "${#1}" -eq 12 ]
}

valid_aws_role_name() {
  value=${1-}
  case ${value} in
    ''|*[!A-Za-z0-9_+=,.@-]*) return 1 ;;
  esac
  [ "${#value}" -le 64 ]
}

valid_aws_role_session_name() {
  value=${1-}
  case ${value} in
    *[!A-Za-z0-9_+=,.@-]*) return 1 ;;
  esac
  [ "${#value}" -ge 2 ] && [ "${#value}" -le 64 ]
}

validate_resolved_iam_role_arn() {
  resolved=${1-}
  partition=$2
  account=$3
  role_name=$4
  prefix="arn:${partition}:iam::${account}:role/"
  case ${resolved} in
    "${prefix}"*) ;;
    *) return 1 ;;
  esac
  role_path=${resolved#"${prefix}"}
  resolved_name=${role_path##*/}
  [ "${resolved_name}" = "${role_name}" ] || return 1
  case ${role_path} in
    ''|/*|*/|*//*|*[![:graph:]]*) return 1 ;;
  esac
  valid_aws_role_name "${resolved_name}"
}

# Resolve an STS assumed-role session to the authoritative IAM Role.Arn.
# STS ARNs deliberately omit an IAM role path, so reconstructing an IAM ARN
# from the session string is unsafe. The caller profile must be allowed to run
# iam:GetRole for any assumed-role principal that participates in a comparison.
# Static IAM role ARNs remain byte-for-byte unchanged.
canonical_aws_role_principal() (
  LC_ALL=C
  export LC_ALL
  principal=${1-}
  profile=${2-}

  body=${principal#arn:}
  [ "${body}" != "${principal}" ] || {
    printf '%s\n' "${principal}"
    exit 0
  }
  partition=${body%%:*}
  remainder=${body#*:}
  [ "${remainder}" != "${body}" ] || {
    aws_principal_error "malformed ARN"
    exit 1
  }
  service=${remainder%%:*}
  remainder=${remainder#*:}
  region=${remainder%%:*}
  remainder=${remainder#*:}
  account=${remainder%%:*}
  resource=${remainder#*:}
  [ "${resource}" != "${remainder}" ] || {
    aws_principal_error "malformed ARN"
    exit 1
  }

  case ${service} in
    iam)
      case ${resource} in
        role/*)
          if ! valid_aws_partition "${partition}" || [ -n "${region}" ] || ! valid_aws_account_id "${account}"; then
            aws_principal_error "malformed IAM role ARN"
            exit 1
          fi
          role_path=${resource#role/}
          role_name=${role_path##*/}
          case ${role_path} in
            ''|/*|*/|*//*|*[![:graph:]]*) aws_principal_error "malformed IAM role path"; exit 1 ;;
          esac
          valid_aws_role_name "${role_name}" \
            || { aws_principal_error "invalid IAM role name"; exit 1; }
          printf '%s\n' "${principal}"
          exit 0
          ;;
        *)
          printf '%s\n' "${principal}"
          exit 0
          ;;
      esac
      ;;
    sts)
      case ${resource} in
        assumed-role/*) ;;
        *) printf '%s\n' "${principal}"; exit 0 ;;
      esac
      ;;
    *)
      printf '%s\n' "${principal}"
      exit 0
      ;;
  esac

  if ! valid_aws_partition "${partition}" || [ -n "${region}" ] || ! valid_aws_account_id "${account}"; then
    aws_principal_error "malformed STS assumed-role ARN"
    exit 1
  fi
  role_and_session=${resource#assumed-role/}
  role_name=${role_and_session%%/*}
  session_name=${role_and_session#*/}
  [ "${session_name}" != "${role_and_session}" ] && [ "${session_name#*/}" = "${session_name}" ] \
    || { aws_principal_error "assumed-role ARN must contain role-name/session-name"; exit 1; }
  valid_aws_role_name "${role_name}" \
    || { aws_principal_error "invalid assumed-role role name"; exit 1; }
  valid_aws_role_session_name "${session_name}" \
    || { aws_principal_error "invalid assumed-role session name"; exit 1; }
  [ -n "${profile}" ] \
    || { aws_principal_error "AWS profile is required to resolve an assumed-role ARN"; exit 1; }
  command -v aws >/dev/null 2>&1 \
    || { aws_principal_error "aws CLI is required to resolve an assumed-role ARN"; exit 1; }

  resolved=$(AWS_PROFILE=${profile} aws iam get-role --role-name "${role_name}" --query Role.Arn --output text) \
    || { aws_principal_error "iam:GetRole failed for ${role_name}"; exit 1; }
  case ${resolved} in
    ''|None|*'
'*) aws_principal_error "iam:GetRole returned an invalid Role.Arn"; exit 1 ;;
  esac
  validate_resolved_iam_role_arn "${resolved}" "${partition}" "${account}" "${role_name}" \
    || { aws_principal_error "iam:GetRole returned a mismatched Role.Arn"; exit 1; }
  printf '%s\n' "${resolved}"
)
