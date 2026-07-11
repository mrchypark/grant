#!/bin/sh
set -eu

ROOT_DIR=$(CDPATH='' cd -P "$(dirname "$0")/.." && pwd)
MODE=${1:-static}
NAMESPACE=${NAMESPACE:-grant}
GRANT_SMOKE_PORT=${GRANT_SMOKE_PORT:-18080}
GRANT_RAUTHY_CLIENT_ID=${GRANT_RAUTHY_CLIENT_ID:-grant}
GRANT_RAUTHY_CLIENT_SECRET=${GRANT_RAUTHY_CLIENT_SECRET:-devRauthyClientSecret0000000000000000000000000000000000000000000}
GRANT_RAUTHY_ADMIN_API_KEY=${GRANT_RAUTHY_ADMIN_API_KEY:-grant-admin\$devRauthyAdminApiKeySecret000000000000000000000000000000000000000}
REPLACE_RAUTHY_POD=${REPLACE_RAUTHY_POD:-0}
TMP_DIR=
PORT_FORWARD_PID=

fail() {
  echo "$*" >&2
  exit 1
}

need() {
  command -v "$1" >/dev/null 2>&1 || fail "missing command: $1"
}

cleanup() {
  if [ -n "${PORT_FORWARD_PID}" ]; then
    kill "${PORT_FORWARD_PID}" >/dev/null 2>&1 || true
    wait "${PORT_FORWARD_PID}" >/dev/null 2>&1 || true
  fi
  if [ -n "${TMP_DIR}" ]; then
    rm -rf "${TMP_DIR}"
  fi
}
trap cleanup 0 HUP INT TERM

assert_no_pvc_references() {
  manifest=$1
  label=$2
  if grep -Eq '(^kind: PersistentVolumeClaim$|volumeClaimTemplates:|persistentVolumeClaim:|claimName:)' "${manifest}"; then
    fail "${label} render contains a PVC reference"
  fi
  awk '
    previous_empty_dir && /^[[:space:]]+name: rauthy-data$/ { found = 1 }
    { previous_empty_dir = $0 ~ /^[[:space:]]*- emptyDir: \{\}$/ }
    END { exit found ? 0 : 1 }
  ' "${manifest}" \
    || fail "${label} render does not use emptyDir for rauthy-data"
}

assert_restricted_bootstrap() {
  manifest=$1
  label=$2
  rauthy_manifest="${TMP_DIR}/${label}-rauthy.yaml"
  init_manifest="${TMP_DIR}/${label}-rauthy-init.yaml"
  awk '
    $0 == "  serviceName: rauthy-headless" { emit = 1 }
    emit && /^---$/ { exit }
    emit { print }
  ' "${manifest}" >"${rauthy_manifest}"
  awk '
    $0 == "      initContainers:" { emit = 1; next }
    emit && $0 == "      securityContext:" { exit }
    emit { print }
  ' "${rauthy_manifest}" >"${init_manifest}"

  count_line() {
    [ "$(grep -cFx "$2" "$1" || true)" -eq "$3" ]
  }
  if ! {
    count_line "${rauthy_manifest}" '        seccompProfile:' 1 \
    && count_line "${rauthy_manifest}" '          type: RuntimeDefault' 1 \
    && count_line "${init_manifest}" '        name: render-bootstrap' 1 \
    && count_line "${init_manifest}" '        securityContext:' 1 \
    && count_line "${init_manifest}" '          runAsUser: 10001' 1 \
    && count_line "${init_manifest}" '          runAsGroup: 10001' 1 \
    && count_line "${init_manifest}" '          runAsNonRoot: true' 1 \
    && count_line "${init_manifest}" '          allowPrivilegeEscalation: false' 1 \
    && count_line "${init_manifest}" '            - ALL' 1 \
    && count_line "${init_manifest}" '        envFrom:' 0 \
    && [ "$(grep -c '^        - name: ' "${init_manifest}" || true)" -eq 2 ] \
    && count_line "${init_manifest}" '        - name: GRANT_RAUTHY_CLIENT_SECRET' 1 \
    && count_line "${init_manifest}" '        - name: GRANT_RAUTHY_ADMIN_API_KEY' 1 \
    && count_line "${init_manifest}" '            secretKeyRef:' 2 \
    && count_line "${init_manifest}" '              key: GRANT_RAUTHY_CLIENT_SECRET' 1 \
    && count_line "${init_manifest}" '              key: GRANT_RAUTHY_ADMIN_API_KEY' 1 \
    && count_line "${init_manifest}" '              name: grant-secrets' 2
  }; then
    fail "${label} render does not harden the bootstrap init container"
  fi
}

verify_static() {
  need kubectl
  need grep
  TMP_DIR=$(mktemp -d "${TMPDIR:-/tmp}/grant-kustomize.XXXXXX")

  kubectl kustomize "${ROOT_DIR}/deploy/k8s" >"${TMP_DIR}/production.yaml"
  kubectl kustomize "${ROOT_DIR}/deploy/overlays/ephemeral" >"${TMP_DIR}/ephemeral.yaml"
  kubectl kustomize "${ROOT_DIR}/deploy/overlays/ied" >"${TMP_DIR}/ied.yaml"

  grep -q 'volumeClaimTemplates:' "${TMP_DIR}/production.yaml" \
    || fail "production render must retain the Rauthy PVC template"
  assert_restricted_bootstrap "${TMP_DIR}/production.yaml" production
  assert_restricted_bootstrap "${TMP_DIR}/ephemeral.yaml" ephemeral
  assert_restricted_bootstrap "${TMP_DIR}/ied.yaml" ied
  assert_no_pvc_references "${TMP_DIR}/ephemeral.yaml" ephemeral
  assert_no_pvc_references "${TMP_DIR}/ied.yaml" ied
  grep -Eq 'name: rauthy-bootstrap-renderer-[a-z0-9]+' "${TMP_DIR}/ephemeral.yaml" \
    || fail "ephemeral render is missing the generated bootstrap renderer ConfigMap"

  echo "Kubernetes storage profiles verified"
}

assert_live_storage() {
  pvc_names=$(kubectl -n "${NAMESPACE}" get pvc -o name)
  [ -z "${pvc_names}" ] || fail "ephemeral namespace contains PVCs: ${pvc_names}"

  kubectl -n "${NAMESPACE}" get statefulset rauthy -o json \
    | jq -e '(.spec.volumeClaimTemplates // []) | length == 0' >/dev/null \
    || fail "Rauthy StatefulSet still has volumeClaimTemplates"
  kubectl -n "${NAMESPACE}" get pod rauthy-0 -o json \
    | jq -e '.spec.volumes[] | select(.name == "rauthy-data") | has("emptyDir")' >/dev/null \
    || fail "Rauthy Pod does not use emptyDir for rauthy-data"
}

wait_for_grant() {
  attempts=0
  while [ "${attempts}" -lt 60 ]; do
    if curl -fsS "http://127.0.0.1:${GRANT_SMOKE_PORT}/healthz" >/dev/null 2>&1; then
      return 0
    fi
    attempts=$((attempts + 1))
    sleep 2
  done
  fail "timed out waiting for Grant port-forward"
}

wait_for_rauthy_proxy() {
  attempts=0
  while [ "${attempts}" -lt 60 ]; do
    if curl -fsS "http://127.0.0.1:${GRANT_SMOKE_PORT}/auth/v1/health" >/dev/null 2>&1; then
      return 0
    fi
    attempts=$((attempts + 1))
    sleep 2
  done
  fail "timed out waiting for Rauthy through Grant proxy"
}

assert_runtime() {
  base_url="http://127.0.0.1:${GRANT_SMOKE_PORT}"

  curl -fsS "${base_url}/healthz" \
    | jq -e '.status == "ok" and .readReady == true and .writeReady == true' >/dev/null \
    || fail "Grant health is not ready"
  curl -fsS "${base_url}/auth/v1/health" >/dev/null \
    || fail "Rauthy health proxy failed"
  curl -fsS -H "Authorization: API-Key ${GRANT_RAUTHY_ADMIN_API_KEY}" \
    "${base_url}/auth/v1/clients/${GRANT_RAUTHY_CLIENT_ID}" >/dev/null \
    || fail "bootstrapped Rauthy client is unavailable through Grant proxy"
  curl -fsS -u "${GRANT_RAUTHY_CLIENT_ID}:${GRANT_RAUTHY_CLIENT_SECRET}" \
    -H 'content-type: application/x-www-form-urlencoded' \
    --data 'token=known-invalid-token' \
    "${base_url}/auth/v1/oidc/introspect" \
    | jq -e '.active == false' >/dev/null \
    || fail "Rauthy introspection proxy did not reject the invalid token"
}

verify_live() {
  need kubectl
  need curl
  need jq
  TMP_DIR=$(mktemp -d "${TMPDIR:-/tmp}/grant-smoke.XXXXXX")

  kubectl -n "${NAMESPACE}" rollout status statefulset/rauthy --timeout=15m
  kubectl -n "${NAMESPACE}" rollout status statefulset/grant-voter --timeout=15m
  assert_live_storage

  kubectl -n "${NAMESPACE}" port-forward service/grant "${GRANT_SMOKE_PORT}:80" \
    >"${TMP_DIR}/port-forward.log" 2>&1 &
  PORT_FORWARD_PID=$!
  wait_for_grant
  wait_for_rauthy_proxy
  assert_runtime

  if [ "${REPLACE_RAUTHY_POD}" = "1" ]; then
    old_uid=$(kubectl -n "${NAMESPACE}" get pod rauthy-0 -o jsonpath='{.metadata.uid}')
    kubectl -n "${NAMESPACE}" delete pod rauthy-0 --wait=true
    kubectl -n "${NAMESPACE}" rollout status statefulset/rauthy --timeout=15m
    kubectl -n "${NAMESPACE}" wait --for=condition=Ready pod/rauthy-0 --timeout=15m
    new_uid=$(kubectl -n "${NAMESPACE}" get pod rauthy-0 -o jsonpath='{.metadata.uid}')
    [ "${old_uid}" != "${new_uid}" ] || fail "Rauthy Pod was not replaced"
    assert_live_storage
    wait_for_rauthy_proxy
    assert_runtime
  fi

  echo "Ephemeral Kubernetes runtime verified"
}

case "${MODE}" in
  static) verify_static ;;
  live) verify_live ;;
  *) fail "usage: $0 [static|live]" ;;
esac
