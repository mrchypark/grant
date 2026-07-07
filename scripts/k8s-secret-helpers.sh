#!/bin/sh

apply_secret_from_literals() {
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

apply_grant_secret() {
  apply_secret_from_literals grant-secrets \
    --from-literal=GRANT_MASTER_KEY="${GRANT_MASTER_KEY}" \
    --from-literal=HQL_SECRET_RAFT="${HQL_SECRET_RAFT}" \
    --from-literal=HQL_SECRET_API="${HQL_SECRET_API}" \
    --from-literal=TRAILBASE_GRANT_INTERNAL_SECRET="${TRAILBASE_GRANT_INTERNAL_SECRET}" \
    --from-literal=ENC_KEYS="${ENC_KEYS}" \
    --from-literal=ENC_KEY_ACTIVE="${ENC_KEY_ACTIVE}"
}
