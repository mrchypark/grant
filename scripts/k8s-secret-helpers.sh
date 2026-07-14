#!/bin/sh

apply_secret_from_files() {
  name="$1"
  shift

  if [ -n "${KUBE_CONTEXT:-}" ]; then
    kubectl --context "${KUBE_CONTEXT}" -n "${NAMESPACE}" create secret generic "${name}" \
      "$@" \
      --dry-run=client -o yaml | kubectl --context "${KUBE_CONTEXT}" apply -f -
    return 0
  fi

  kubectl -n "${NAMESPACE}" create secret generic "${name}" \
    "$@" \
    --dry-run=client -o yaml | kubectl apply -f -
}

apply_grant_secret() (
  old_umask=$(umask)
  umask 077
  secret_dir=$(mktemp -d "${TMPDIR:-/tmp}/grant-secrets.XXXXXX")
  trap 'rm -rf "${secret_dir}"' 0 HUP INT TERM
  umask "${old_umask}"
  write_secret_file() { printf '%s' "$2" >"${secret_dir}/$1"; }
  write_secret_file GRANT_MASTER_KEY "${GRANT_MASTER_KEY}"
  write_secret_file GRANT_APP_AUTH_KEY "${GRANT_APP_AUTH_KEY}"
  write_secret_file HQL_SECRET_RAFT "${HQL_SECRET_RAFT}"
  write_secret_file HQL_SECRET_API "${HQL_SECRET_API}"
  write_secret_file GRANT_RAUTHY_CLIENT_ID "${GRANT_RAUTHY_CLIENT_ID}"
  write_secret_file GRANT_RAUTHY_CLIENT_SECRET "${GRANT_RAUTHY_CLIENT_SECRET}"
  write_secret_file GRANT_RAUTHY_CLIENT_READ_API_KEY "${GRANT_RAUTHY_CLIENT_READ_API_KEY}"
  write_secret_file ENC_KEYS "${ENC_KEYS}"
  write_secret_file ENC_KEY_ACTIVE "${ENC_KEY_ACTIVE}"
  if apply_secret_from_files grant-secrets \
    --from-file="GRANT_MASTER_KEY=${secret_dir}/GRANT_MASTER_KEY" \
    --from-file="GRANT_APP_AUTH_KEY=${secret_dir}/GRANT_APP_AUTH_KEY" \
    --from-file="HQL_SECRET_RAFT=${secret_dir}/HQL_SECRET_RAFT" \
    --from-file="HQL_SECRET_API=${secret_dir}/HQL_SECRET_API" \
    --from-file="GRANT_RAUTHY_CLIENT_ID=${secret_dir}/GRANT_RAUTHY_CLIENT_ID" \
    --from-file="GRANT_RAUTHY_CLIENT_SECRET=${secret_dir}/GRANT_RAUTHY_CLIENT_SECRET" \
    --from-file="GRANT_RAUTHY_CLIENT_READ_API_KEY=${secret_dir}/GRANT_RAUTHY_CLIENT_READ_API_KEY" \
    --from-file="ENC_KEYS=${secret_dir}/ENC_KEYS" \
    --from-file="ENC_KEY_ACTIVE=${secret_dir}/ENC_KEY_ACTIVE"; then status=0
  else status=$?; fi
  rm -rf "${secret_dir}"
  return "${status}"
)
