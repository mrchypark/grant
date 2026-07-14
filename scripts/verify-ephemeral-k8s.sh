#!/bin/sh
set -eu

ROOT_DIR=$(CDPATH='' cd -P "$(dirname "$0")/.." && pwd)
MODE=${1:-static}
NAMESPACE=${NAMESPACE:-grant}
GRANT_SMOKE_PORT=${GRANT_SMOKE_PORT:-18080}
GRANT_RAUTHY_CLIENT_ID=${GRANT_RAUTHY_CLIENT_ID:-grant}
GRANT_RAUTHY_CLIENT_SECRET=${GRANT_RAUTHY_CLIENT_SECRET:-devRauthyClientSecret0000000000000000000000000000000000000000000}
GRANT_RAUTHY_CLIENT_READ_API_KEY=${GRANT_RAUTHY_CLIENT_READ_API_KEY:-grant-client-read\$devRauthyClientReadApiKeySecret00000000000000000000000000000000000}
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

assert_ha_contract() {
  manifest=$1
  label=$2
  [ "$(grep -c '^  podManagementPolicy: Parallel$' "${manifest}" || true)" -eq 2 ] \
    || fail "${label} must render two Parallel StatefulSets"
  [ "$(grep -c '^  replicas: 3$' "${manifest}" || true)" -eq 2 ] \
    || fail "${label} must render Grant and Rauthy at three replicas"
  [ "$(grep -c '^  minAvailable: 2$' "${manifest}" || true)" -eq 2 ] \
    || fail "${label} must render two quorum PDBs"
  grep -q 'name: lifecycle-gate' "${manifest}" || fail "${label} is missing lifecycle gates"
  if grep -E 'image: (busybox|ghcr.io/sebadob/rauthy)' "${manifest}" | grep -v '@sha256:' >/dev/null; then
    fail "${label} contains an unpinned third-party image"
  fi
}

assert_recovery_dry_run() {
  pair="${TMP_DIR}/pair.json"
  RECOVERY_ID=ci-static RECOVERY_GENERATION=ci-generation RESTORE_RETAIN_UNTIL=2030-01-01T00:00:00Z \
  ISOLATED_DRILL_ID=pending \
  GRANT_BUCKET=grant-ci-backups GRANT_SOURCE_KEY=backup_node_1_1.sqlite GRANT_SOURCE_VERSION_ID=grant-v1 \
  GRANT_SOURCE_SHA256=aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa \
  GRANT_IMAGE=registry.example.com/grant@sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa \
  GRANT_SCHEMA_ID=grant-schema-ci GRANT_KEY_ID=grant-key-ci \
  GRANT_BACKUP_GENERATION=grant-ci-g1 \
  GRANT_KEYRING_SHA256=cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc \
  GRANT_KEYRING_GENERATION=grant-keyring-ci GRANT_WRITER_PRINCIPAL_ARN=arn:aws:iam::111111111111:role/grant-writer \
  GRANT_RESTORE_READER_PRINCIPAL_ARN=arn:aws:iam::111111111111:role/grant-reader \
  RAUTHY_BUCKET=rauthy-ci-backups RAUTHY_SOURCE_KEY=backup_node_1_1.sqlite RAUTHY_SOURCE_VERSION_ID=rauthy-v1 \
  RAUTHY_SOURCE_SHA256=bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb \
  RAUTHY_IMAGE=ghcr.io/sebadob/rauthy:0.36.0@sha256:e2a670c79e04ddf5947fbd6cd5dc2ec7115bb9f21132ddf03129d7b66d26f11d \
  RAUTHY_SCHEMA_ID=rauthy-0.36.0 RAUTHY_KEY_ID=rauthy-key-ci \
  RAUTHY_BACKUP_GENERATION=rauthy-ci-g1 \
  RAUTHY_KEYRING_SHA256=dddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddd \
  RAUTHY_KEYRING_GENERATION=rauthy-keyring-ci RAUTHY_WRITER_PRINCIPAL_ARN=arn:aws:iam::222222222222:role/rauthy-writer \
  RAUTHY_RESTORE_READER_PRINCIPAL_ARN=arn:aws:iam::222222222222:role/rauthy-reader \
    "${ROOT_DIR}/scripts/create-recovery-pair.sh" --dry-run --output "${pair}"
  jq -e '
    (.grant.restoreReaderPrincipal | length > 0) and (.rauthy.restoreReaderPrincipal | length > 0) and
    (.grant.writerPrincipal | length > 0) and (.rauthy.writerPrincipal | length > 0) and
    (.grant.keyringSha256 | test("^[0-9a-f]{64}$")) and (.rauthy.keyringSha256 | test("^[0-9a-f]{64}$")) and
    (.trafficWriteFence.snapshots.grant.snapshotId | length > 0) and
    (.trafficWriteFence.snapshots.rauthy.snapshotId | length > 0)
  ' "${pair}" >/dev/null || fail "recovery pair is missing reader, writer, keyring, or signed snapshot bindings"
  if "${ROOT_DIR}/scripts/restore-full-cluster.sh" --dry-run --record "${pair}" >/dev/null 2>&1; then
    fail "unapproved recovery record was accepted for production restore"
  fi
  "${ROOT_DIR}/scripts/restore-full-cluster.sh" --dry-run --isolated-drill --record "${pair}" >/dev/null
  "${ROOT_DIR}/scripts/restore-full-cluster.sh" --dry-run --isolated-drill --release-completed --record "${pair}" >/dev/null

  openssl genpkey -algorithm RSA -pkeyopt rsa_keygen_bits:2048 -out "${TMP_DIR}/approval-private.pem" >/dev/null 2>&1
  openssl pkey -in "${TMP_DIR}/approval-private.pem" -pubout -out "${TMP_DIR}/approval-public.pem" >/dev/null 2>&1
  jq '.isolatedDrill={id:"static-test",passed:true,completedAt:"2026-07-14T00:00:00Z"} |
      .approval={passed:true,approvedAt:"2026-07-14T00:00:00Z",workflowRunId:"static-test",gitCommit:"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",semanticEvidenceSha256:"bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"}' \
    "${pair}" >"${TMP_DIR}/approved.json"
  jq -S -c . "${TMP_DIR}/approved.json" >"${TMP_DIR}/approved.canonical.json"
  openssl dgst -sha256 -sign "${TMP_DIR}/approval-private.pem" -out "${TMP_DIR}/approved.sig" "${TMP_DIR}/approved.canonical.json"
  fingerprint=$(openssl pkey -pubin -in "${TMP_DIR}/approval-public.pem" -outform DER | openssl dgst -sha256 | awk '{print $NF}')
  RECOVERY_APPROVAL_PUBLIC_KEY_SHA256=${fingerprint} RECOVERY_OPERATOR_GIT_COMMIT=aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa "${ROOT_DIR}/scripts/restore-full-cluster.sh" --dry-run \
    --record "${TMP_DIR}/approved.json" --approval-signature "${TMP_DIR}/approved.sig" --approval-public-key "${TMP_DIR}/approval-public.pem" >/dev/null
  jq '.generation="tampered"' "${TMP_DIR}/approved.json" >"${TMP_DIR}/tampered.json"
  if RECOVERY_APPROVAL_PUBLIC_KEY_SHA256=${fingerprint} RECOVERY_OPERATOR_GIT_COMMIT=aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa "${ROOT_DIR}/scripts/restore-full-cluster.sh" --dry-run \
    --record "${TMP_DIR}/tampered.json" --approval-signature "${TMP_DIR}/approved.sig" --approval-public-key "${TMP_DIR}/approval-public.pem" >/dev/null 2>&1; then
    fail "tampered approved recovery record passed signature verification"
  fi
}

verify_static() {
  need kubectl
  need grep
  need jq
  need openssl
  TMP_DIR=$(mktemp -d "${TMPDIR:-/tmp}/grant-kustomize.XXXXXX")

  kubectl kustomize "${ROOT_DIR}/deploy/k8s" >"${TMP_DIR}/base.yaml"
  kubectl kustomize "${ROOT_DIR}/deploy/overlays/ephemeral" >"${TMP_DIR}/ephemeral.yaml"
  kubectl kustomize "${ROOT_DIR}/deploy/overlays/ied" >"${TMP_DIR}/ied.yaml"
  kubectl kustomize "${ROOT_DIR}/deploy/overlays/production" >"${TMP_DIR}/production.yaml"

  assert_no_pvc_references "${TMP_DIR}/base.yaml" base
  assert_no_pvc_references "${TMP_DIR}/ephemeral.yaml" ephemeral
  assert_no_pvc_references "${TMP_DIR}/ied.yaml" ied
  assert_no_pvc_references "${TMP_DIR}/production.yaml" production
  assert_ha_contract "${TMP_DIR}/base.yaml" base
  assert_ha_contract "${TMP_DIR}/ephemeral.yaml" ephemeral
  assert_ha_contract "${TMP_DIR}/ied.yaml" ied
  assert_ha_contract "${TMP_DIR}/production.yaml" production

  [ "$(grep -c 'whenUnsatisfiable: DoNotSchedule' "${TMP_DIR}/production.yaml" || true)" -eq 2 ] \
    || fail "production must use hard topology spread"
  [ "$(grep -c 'requiredDuringSchedulingIgnoredDuringExecution:' "${TMP_DIR}/production.yaml" || true)" -eq 2 ] \
    || fail "production must use hard pod anti-affinity"
  grep -q 'name: grant-recovery-state' "${TMP_DIR}/production.yaml" \
    || fail "production must require the external recovery state"
  grep -q 'name: grant-production-backup' "${TMP_DIR}/production.yaml" \
    || fail "production Grant backup references are missing"
  grep -q 'name: rauthy-production-backup' "${TMP_DIR}/production.yaml" \
    || fail "production Rauthy backup references are missing"
  grep -q 'image: registry.invalid/grant@sha256:REQUIRED_PRODUCTION_DIGEST' "${TMP_DIR}/production.yaml" \
    || fail "production template must retain an admission-invalid required image marker"
  if grep -Eq 'sha256:0{64}|grant\.example\.com|identity\.example\.com|kind: Ingress' "${TMP_DIR}/production.yaml"; then
    fail "production render contains an unresolved digest, example host, or bundled public ingress"
  fi
  grep -q 'name: GRANT_APP_AUTH_KEY' "${TMP_DIR}/production.yaml" \
    || fail "production Grant app-auth key wiring is missing"
  grep -q 'key: GRANT_APP_AUTH_KEY' "${TMP_DIR}/production.yaml" \
    || fail "production Grant app-auth Secret key wiring is missing"
  if grep -E 'localhost|grant\.local|dev-rauthy|grant:local|SuperSecureSecret|AnotherSecureSecret' "${TMP_DIR}/production.yaml" >/dev/null; then
    fail "production render contains a development fallback"
  fi
  grep -q 'KEYRING_GENERATION:' "${ROOT_DIR}/deploy/overlays/production/grant-production-config.example.yaml" \
    || fail "Grant keyring generation binding is missing"
  grep -q 'KEYRING_GENERATION:' "${ROOT_DIR}/deploy/overlays/production/rauthy-production-config.example.yaml" \
    || fail "Rauthy keyring generation binding is missing"
  if grep -n -- '--from-literal=.*[$][{]' "${ROOT_DIR}/scripts/k8s-secret-helpers.sh" >/dev/null; then
    fail "Kubernetes secret helper exposes secret values through child argv"
  fi

  grant_bucket=$(sed -n 's/.*HQL_S3_BUCKET:[[:space:]]*"\([^"]*\)".*/\1/p' "${ROOT_DIR}/deploy/overlays/production/grant-production-config.example.yaml")
  rauthy_bucket=$(sed -n 's/.*HQL_S3_BUCKET:[[:space:]]*"\([^"]*\)".*/\1/p' "${ROOT_DIR}/deploy/overlays/production/rauthy-production-config.example.yaml")
  [ -n "${grant_bucket}" ] && [ -n "${rauthy_bucket}" ] && [ "${grant_bucket}" != "${rauthy_bucket}" ] \
    || fail "Grant and Rauthy examples must use distinct backup buckets"
  grep -Eq 'name: rauthy-bootstrap-renderer-[a-z0-9]+' "${TMP_DIR}/ephemeral.yaml" \
    || fail "ephemeral render is missing the generated bootstrap renderer ConfigMap"
  assert_recovery_dry_run
  "${ROOT_DIR}/scripts/verify-recovery-state-machine.sh"
  "${ROOT_DIR}/scripts/verify-restore-once.sh"

  echo "Kubernetes HA, lifecycle, backup, and restore profiles verified"
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
  curl -fsS -H "Authorization: API-Key ${GRANT_RAUTHY_CLIENT_READ_API_KEY}" \
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
