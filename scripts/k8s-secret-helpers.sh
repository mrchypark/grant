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

is_private_rauthy_launcher_image() {
  case "${1:-}" in
    ghcr.io/mrchypark/grant-rauthy-launcher|ghcr.io/mrchypark/grant-rauthy-launcher:*|ghcr.io/mrchypark/grant-rauthy-launcher@*) return 0 ;;
    *) return 1 ;;
  esac
}

prepare_rauthy_registry_auth() {
  is_private_rauthy_launcher_image "${RAUTHY_IMAGE:-}" || return 0
  [ -n "${GHCR_USERNAME:-}" ] \
    || { echo "GHCR_USERNAME is required for the private Rauthy launcher image" >&2; return 1; }
  [ -n "${GHCR_TOKEN:-}" ] \
    || { echo "GHCR_TOKEN with read:packages is required for the private Rauthy launcher image" >&2; return 1; }
  RAUTHY_DOCKER_CONTEXT=$(docker context show) \
    || { echo "cannot read the active Docker context" >&2; return 1; }
  [ -n "${RAUTHY_DOCKER_CONTEXT}" ] \
    || { echo "active Docker context is empty" >&2; return 1; }
  docker --context "${RAUTHY_DOCKER_CONTEXT}" info >/dev/null \
    || { echo "active Docker context is unavailable: ${RAUTHY_DOCKER_CONTEXT}" >&2; return 1; }
  export RAUTHY_DOCKER_CONTEXT
  old_umask=$(umask)
  umask 077
  RAUTHY_REGISTRY_CONFIG_DIR=$(mktemp -d "${TMPDIR:-/tmp}/rauthy-registry.XXXXXX")
  umask "${old_umask}"
  export RAUTHY_REGISTRY_CONFIG_DIR
  printf '%s' "${GHCR_TOKEN}" \
    | docker --context "${RAUTHY_DOCKER_CONTEXT}" --config "${RAUTHY_REGISTRY_CONFIG_DIR}" \
      login ghcr.io --username "${GHCR_USERNAME}" --password-stdin >/dev/null
  [ -s "${RAUTHY_REGISTRY_CONFIG_DIR}/config.json" ] \
    || { echo "GHCR login did not create a Docker credential file" >&2; return 1; }
}

apply_rauthy_registry_pull_secret() {
  is_private_rauthy_launcher_image "${RAUTHY_IMAGE:-}" || return 0
  [ -n "${RAUTHY_REGISTRY_CONFIG_DIR:-}" ] && [ -s "${RAUTHY_REGISTRY_CONFIG_DIR}/config.json" ] \
    || { echo "private Rauthy launcher pull credentials are unavailable" >&2; return 1; }
  apply_secret_from_files rauthy-launcher-pull \
    --type=kubernetes.io/dockerconfigjson \
    --from-file=".dockerconfigjson=${RAUTHY_REGISTRY_CONFIG_DIR}/config.json"
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
