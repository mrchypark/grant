#!/bin/sh
set -eu

DRY_RUN=0
ISOLATED_DRILL=0
REARM_ORDINAL0=
RELEASE_COMPLETED=0
NAMESPACE=${NAMESPACE:-grant}
RECORD=
CONFIRM_RECOVERY_ID=
STATE_CONFIGMAP=${STATE_CONFIGMAP:-grant-recovery-state}
TIMEOUT=${RESTORE_TIMEOUT:-20m}
MIN_REMAINING_RETENTION_SECONDS=${MIN_REMAINING_RETENTION_SECONDS:-3600}
APPROVAL_SIGNATURE=
APPROVAL_PUBLIC_KEY=
EXPECTED_NAMESPACE_UID=
SEMANTIC_CONFIG=
LEASE_NAME=${RECOVERY_LEASE_NAME:-grant-recovery-lock}
RAUTHY_SECRET_NAME=${RAUTHY_HQL_SECRET_NAME:-rauthy-production-secrets}
GRANT_SECRET_NAME=${GRANT_HQL_SECRET_NAME:-grant-production-secrets}

while [ "$#" -gt 0 ]; do
  case "$1" in
    --dry-run) DRY_RUN=1 ;;
    --isolated-drill) ISOLATED_DRILL=1 ;;
    --rearm-ordinal0) shift; REARM_ORDINAL0=${1:?missing workload} ;;
    --release-completed) RELEASE_COMPLETED=1 ;;
    --namespace) shift; NAMESPACE=${1:?missing namespace} ;;
    --record) shift; RECORD=${1:?missing record} ;;
    --confirm-recovery-id) shift; CONFIRM_RECOVERY_ID=${1:?missing recovery id} ;;
    --approval-signature) shift; APPROVAL_SIGNATURE=${1:?missing signature} ;;
    --approval-public-key) shift; APPROVAL_PUBLIC_KEY=${1:?missing public key} ;;
    --expected-namespace-uid) shift; EXPECTED_NAMESPACE_UID=${1:?missing namespace UID} ;;
    --semantic-config) shift; SEMANTIC_CONFIG=${1:?missing semantic config} ;;
    *) echo "usage: $0 [--dry-run] [--isolated-drill] [--rearm-ordinal0 grant|rauthy] [--release-completed] --record FILE [--namespace NAME] [--confirm-recovery-id ID] [--approval-signature FILE --approval-public-key FILE] [--expected-namespace-uid UID] [--semantic-config FILE]" >&2; exit 2 ;;
  esac
  shift
done

[ -n "${RECORD}" ] && [ -r "${RECORD}" ] || { echo "readable --record is required" >&2; exit 2; }
command -v jq >/dev/null 2>&1 || { echo "missing command: jq" >&2; exit 1; }
ROOT_DIR=$(CDPATH='' cd -P "$(dirname "$0")/.." && pwd)
# shellcheck source=recovery-state-machine.sh
# shellcheck disable=SC1091
. "${ROOT_DIR}/scripts/recovery-state-machine.sh"

jq -e '
  .version == 2 and
  ([.recoveryId,.generation,.namespace,
    .grant.bucket,.grant.backupGeneration,.grant.keyringGeneration,.grant.restore.key,.grant.restore.versionId,.grant.restore.sha256,.grant.restore.retainUntil,.grant.restore.immutablePolicySid,.grant.restore.writerReadDenyPolicySid,.grant.materializationPrincipal,.grant.writerPrincipal,.grant.restoreReaderPrincipal,.grant.image,.grant.schemaId,.grant.keyId,.grant.keyringSha256,
    .rauthy.bucket,.rauthy.backupGeneration,.rauthy.keyringGeneration,.rauthy.restore.key,.rauthy.restore.versionId,.rauthy.restore.sha256,.rauthy.restore.retainUntil,.rauthy.restore.immutablePolicySid,.rauthy.restore.writerReadDenyPolicySid,.rauthy.materializationPrincipal,.rauthy.writerPrincipal,.rauthy.restoreReaderPrincipal,.rauthy.image,.rauthy.schemaId,.rauthy.keyId,.rauthy.keyringSha256,
    .trafficWriteFence.startedAt,.trafficWriteFence.completedAt,.trafficWriteFence.snapshots.grant.snapshotId,.trafficWriteFence.snapshots.grant.createdAt,.trafficWriteFence.snapshots.rauthy.snapshotId,.trafficWriteFence.snapshots.rauthy.createdAt]
   | all(type == "string" and length > 0)) and (.grant.endpoint | type == "string") and (.rauthy.endpoint | type == "string")
   and (.grant.keyringSha256 | test("^[0-9a-fA-F]{64}$")) and (.rauthy.keyringSha256 | test("^[0-9a-fA-F]{64}$"))
   and ((.trafficWriteFence.release.cronJobsToResume // []) | type == "array" and all(.[]; type == "string" and length > 0))
' "${RECORD}" >/dev/null || { echo "invalid or incomplete recovery record" >&2; exit 2; }

RECOVERY_ID=$(jq -er '.recoveryId' "${RECORD}")
GENERATION=$(jq -er '.generation' "${RECORD}")
case "${RECOVERY_ID}" in ''|*[!a-z0-9-]*|-*|*-) echo "record recoveryId must be a lowercase DNS label" >&2; exit 2 ;; esac
[ "${#RECOVERY_ID}" -le 32 ] || { echo "record recoveryId must be at most 32 characters" >&2; exit 2; }
DRILL_PASSED=$(jq -r '.isolatedDrill.passed' "${RECORD}")
if [ "${ISOLATED_DRILL}" -eq 0 ] && [ "${DRILL_PASSED}" != "true" ]; then
  echo "production restore requires a record with a passed isolated restore drill" >&2
  exit 1
fi
jq -e '.trafficWriteFence.directWritersStopped == true and (.trafficWriteFence.evidenceSha256 | length > 0) and (.trafficWriteFence.signerFingerprint | length > 0)' "${RECORD}" >/dev/null \
  || { echo "record lacks signed coordinated write-fence evidence" >&2; exit 1; }
[ "$(jq -r '.grant.bucket' "${RECORD}")" != "$(jq -r '.rauthy.bucket' "${RECORD}")" ] || { echo "workloads must use distinct buckets" >&2; exit 1; }
[ "$(jq -r '.grant.materializationPrincipal' "${RECORD}")" != "$(jq -r '.rauthy.materializationPrincipal' "${RECORD}")" ] || { echo "materialization principals must be distinct" >&2; exit 1; }
[ "$(jq -r '.grant.restoreReaderPrincipal' "${RECORD}")" != "$(jq -r '.rauthy.restoreReaderPrincipal' "${RECORD}")" ] || { echo "restore-reader principals must be distinct" >&2; exit 1; }

if [ "${ISOLATED_DRILL}" -eq 0 ]; then
  [ -r "${APPROVAL_SIGNATURE}" ] && [ -r "${APPROVAL_PUBLIC_KEY}" ] || { echo "production restore requires approval signature and public key" >&2; exit 2; }
  : "${RECOVERY_APPROVAL_PUBLIC_KEY_SHA256:?RECOVERY_APPROVAL_PUBLIC_KEY_SHA256 is required}"
  command -v openssl >/dev/null 2>&1 || { echo "missing command: openssl" >&2; exit 1; }
  approval_fingerprint=$(openssl pkey -pubin -in "${APPROVAL_PUBLIC_KEY}" -outform DER | openssl dgst -sha256 | awk '{print $NF}')
  [ "${approval_fingerprint}" = "${RECOVERY_APPROVAL_PUBLIC_KEY_SHA256}" ] || { echo "approval public key is not the pinned trust root" >&2; exit 1; }
  canonical=$(mktemp "${TMPDIR:-/tmp}/recovery-approval.XXXXXX")
  jq -S -c . "${RECORD}" >"${canonical}"
  openssl dgst -sha256 -verify "${APPROVAL_PUBLIC_KEY}" -signature "${APPROVAL_SIGNATURE}" "${canonical}" >/dev/null \
    || { rm -f "${canonical}"; echo "approved record signature verification failed" >&2; exit 1; }
  rm -f "${canonical}"
  jq -e '.approval.passed == true and (.approval.workflowRunId | length > 0) and (.approval.gitCommit | test("^[0-9a-f]{40}$")) and (.approval.semanticEvidenceSha256 | test("^[0-9a-f]{64}$"))' "${RECORD}" >/dev/null \
    || { echo "signed record lacks complete approval evidence" >&2; exit 1; }
  : "${RECOVERY_OPERATOR_GIT_COMMIT:?RECOVERY_OPERATOR_GIT_COMMIT is required}"
  [ "${RECOVERY_OPERATOR_GIT_COMMIT}" = "$(jq -r '.approval.gitCommit' "${RECORD}")" ] \
    || { echo "recovery operator checkout/image is not the commit that passed the signed drill" >&2; exit 1; }
fi

if [ "${DRY_RUN}" -eq 0 ]; then
  [ "${CONFIRM_RECOVERY_ID}" = "${RECOVERY_ID}" ] || {
    echo "live restore requires --confirm-recovery-id ${RECOVERY_ID}" >&2
    exit 2
  }
  for command in kubectl curl aws mktemp openssl; do
    command -v "${command}" >/dev/null 2>&1 || { echo "missing command: ${command}" >&2; exit 1; }
  done
  namespace_object=$(kubectl get namespace "${NAMESPACE}" -o json)
  actual_namespace_uid=$(printf '%s\n' "${namespace_object}" | jq -r '.metadata.uid // empty')
  [ -n "${EXPECTED_NAMESPACE_UID}" ] && [ "${actual_namespace_uid}" = "${EXPECTED_NAMESPACE_UID}" ] || { echo "namespace UID allowlist mismatch" >&2; exit 2; }
  if [ "${ISOLATED_DRILL}" -eq 1 ]; then
    [ "${NAMESPACE}" != "$(jq -r '.namespace' "${RECORD}")" ] || { echo "isolated drill namespace must differ from production" >&2; exit 2; }
    [ "$(printf '%s\n' "${namespace_object}" | jq -r '.metadata.labels["grant.cypark.dev/recovery-drill"] // empty')" = true ] || { echo "namespace is not labeled as a recovery drill" >&2; exit 2; }
    [ "$(printf '%s\n' "${namespace_object}" | jq -r '.metadata.annotations["grant.cypark.dev/source-recovery-id"] // empty')" = "${RECOVERY_ID}" ] || { echo "drill namespace is not bound to this recovery ID" >&2; exit 2; }
  else
    [ "${NAMESPACE}" = "$(jq -r '.namespace' "${RECORD}")" ] && [ "${actual_namespace_uid}" = "$(jq -r '.namespaceUid' "${RECORD}")" ] \
      || { echo "production namespace name/UID differs from the signed record" >&2; exit 2; }
    [ "$(printf '%s\n' "${namespace_object}" | jq -r '.metadata.labels["grant.cypark.dev/recovery-write-fence"] // empty')" = "${RECOVERY_ID}" ] \
      || { echo "production namespace write-fence label is missing or belongs to another recovery" >&2; exit 2; }
  fi
  kubectl -n "${NAMESPACE}" get cronjob -o json | jq -e 'all(.items[]; .spec.suspend == true)' >/dev/null \
    || { echo "all namespace CronJobs must remain suspended during recovery" >&2; exit 1; }
fi

log() { printf '%s\n' "$*" >&2; }
run() {
  if [ "${DRY_RUN}" -eq 1 ]; then
    printf 'dry-run:' >&2
    printf ' %s' "$@" >&2
    printf '\n' >&2
  else
    "$@"
  fi
}

LEASE_HOLDER=
PORT_FORWARD_PID=
SEMANTIC_SERVICE="rauthy-recovery-${RECOVERY_ID}"
SEMANTIC_ROUTE_ACTIVE=0
acquire_lease() {
  [ "${DRY_RUN}" -eq 0 ] || return 0
  LEASE_HOLDER="${RECOVERY_ID}-$(hostname)-$$"
  lease=$(jq -nc --arg name "${LEASE_NAME}" --arg ns "${NAMESPACE}" --arg holder "${LEASE_HOLDER}" \
    '{apiVersion:"coordination.k8s.io/v1",kind:"Lease",metadata:{name:$name,namespace:$ns},spec:{holderIdentity:$holder,acquireTime:(now|todate),renewTime:(now|todate)}}')
  printf '%s\n' "${lease}" | kubectl create -f - >/dev/null 2>&1 || {
    holder=$(kubectl -n "${NAMESPACE}" get lease "${LEASE_NAME}" -o jsonpath='{.spec.holderIdentity}' 2>/dev/null || true)
    echo "recovery lease is already held by ${holder:-unknown}; inspect the checkpoint before manually deleting a stale lease" >&2
    exit 1
  }
}

release_lease() {
  [ -n "${LEASE_HOLDER}" ] || return 0
  holder=$(kubectl -n "${NAMESPACE}" get lease "${LEASE_NAME}" -o jsonpath='{.spec.holderIdentity}' 2>/dev/null || true)
  if [ "${holder}" = "${LEASE_HOLDER}" ]; then kubectl -n "${NAMESPACE}" delete lease "${LEASE_NAME}" --ignore-not-found >/dev/null 2>&1 || true; fi
}

cleanup_all() {
  if [ -n "${PORT_FORWARD_PID}" ]; then kill "${PORT_FORWARD_PID}" >/dev/null 2>&1 || true; wait "${PORT_FORWARD_PID}" >/dev/null 2>&1 || true; fi
  if [ "${DRY_RUN}" -eq 0 ] && [ "${CURRENT_PHASE:-materialized}" != unfenced ] && [ -n "${LEASE_HOLDER}" ]; then
    patch=$(jq -nc --arg id "${RECOVERY_ID}" '{spec:{selector:{"grant-recovery-fence":$id}}}')
    kubectl -n "${NAMESPACE}" patch service grant --type merge -p "${patch}" >/dev/null 2>&1 || true
    kubectl -n "${NAMESPACE}" patch service rauthy --type merge -p "${patch}" >/dev/null 2>&1 || true
  fi
  if [ "${DRY_RUN}" -eq 0 ] && [ "${SEMANTIC_ROUTE_ACTIVE}" -eq 1 ]; then
    restore_patch='{"spec":{"template":{"spec":{"containers":[{"name":"grant","env":[{"name":"GRANT_RAUTHY_BASE_URL","value":null,"valueFrom":{"configMapKeyRef":{"name":"grant-production-config","key":"GRANT_RAUTHY_BASE_URL"}}}]}]}}}}'
    kubectl -n "${NAMESPACE}" patch statefulset grant-voter --type strategic -p "${restore_patch}" >/dev/null 2>&1 || true
    kubectl -n "${NAMESPACE}" delete service "${SEMANTIC_SERVICE}" --ignore-not-found >/dev/null 2>&1 || true
  fi
  release_lease
}

acquire_lease
trap cleanup_all 0 HUP INT TERM

CURRENT_PHASE=materialized
STATE_OWNER=none
if [ "${DRY_RUN}" -eq 0 ] && kubectl -n "${NAMESPACE}" get configmap "${STATE_CONFIGMAP}" >/dev/null 2>&1; then
  state_id=$(kubectl -n "${NAMESPACE}" get configmap "${STATE_CONFIGMAP}" -o jsonpath='{.data.recovery-id}')
  state_phase=$(kubectl -n "${NAMESPACE}" get configmap "${STATE_CONFIGMAP}" -o jsonpath='{.data.phase}')
  case "${state_id}" in
    none)
      [ "${state_phase}" = unfenced ] || { echo "unowned recovery state must be at the unfenced phase" >&2; exit 1; }
      CURRENT_PHASE=materialized
      STATE_OWNER=none
      ;;
    '' ) echo "recovery state is missing recovery-id" >&2; exit 1 ;;
    "${RECOVERY_ID}")
      recovery_phase_known "${state_phase}" || { echo "unknown or missing persisted recovery phase: ${state_phase:-empty}" >&2; exit 1; }
      [ "$(kubectl -n "${NAMESPACE}" get configmap "${STATE_CONFIGMAP}" -o jsonpath='{.data.generation}')" = "${GENERATION}" ] \
        || { echo "checkpoint generation differs from signed record" >&2; exit 1; }
      [ "$(kubectl -n "${NAMESPACE}" get configmap "${STATE_CONFIGMAP}" -o jsonpath='{.data.grant-sha256}')" = "$(jq -r '.grant.restore.sha256' "${RECORD}")" ] \
        && [ "$(kubectl -n "${NAMESPACE}" get configmap "${STATE_CONFIGMAP}" -o jsonpath='{.data.rauthy-sha256}')" = "$(jq -r '.rauthy.restore.sha256' "${RECORD}")" ] \
        || { echo "checkpoint restore checksums differ from signed record" >&2; exit 1; }
      CURRENT_PHASE=${state_phase}
      STATE_OWNER=${RECOVERY_ID}
      ;;
    *) echo "another recovery is active: ${state_id}" >&2; exit 1 ;;
  esac
fi

if [ "${RELEASE_COMPLETED}" -eq 1 ] && [ "${DRY_RUN}" -eq 0 ] && [ "${STATE_OWNER}" = none ]; then
  last_id=$(kubectl -n "${NAMESPACE}" get configmap "${STATE_CONFIGMAP}" -o jsonpath='{.data.last-recovery-id}' 2>/dev/null || true)
  [ "${last_id}" = "${RECOVERY_ID}" ] && { log "recovery ${RECOVERY_ID} was already released"; exit 0; }
  echo "cannot release recovery ${RECOVERY_ID}: it does not own the terminal checkpoint" >&2
  exit 1
fi

at_least() { [ "$(recovery_phase_rank "${CURRENT_PHASE}")" -ge "$(recovery_phase_rank "$1")" ]; }

update_state() {
  phase=$1 grant_mode=$2 rauthy_mode=$3
  recovery_assert_transition "${CURRENT_PHASE}" "${phase}"
  if [ "${DRY_RUN}" -eq 1 ]; then
    log "dry-run: checkpoint ${phase} (grant=${grant_mode}, rauthy=${rauthy_mode})"
    CURRENT_PHASE=${phase}
    return
  fi
  grant_revision=$(kubectl -n "${NAMESPACE}" get statefulset grant-voter -o jsonpath='{.status.updateRevision}' 2>/dev/null || true)
  rauthy_revision=$(kubectl -n "${NAMESPACE}" get statefulset rauthy -o jsonpath='{.status.updateRevision}' 2>/dev/null || true)
  current_object=$(kubectl -n "${NAMESPACE}" get configmap "${STATE_CONFIGMAP}" -o json)
  persisted_id=$(printf '%s\n' "${current_object}" | jq -r '.data["recovery-id"] // empty')
  persisted_phase=$(printf '%s\n' "${current_object}" | jq -r '.data.phase // empty')
  if [ "${CURRENT_PHASE}" = materialized ]; then
    [ "${persisted_id}" = none ] && [ "${persisted_phase}" = unfenced ] || { echo "initial checkpoint changed concurrently" >&2; exit 1; }
  else
    [ "${persisted_id}" = "${RECOVERY_ID}" ] && [ "${persisted_phase}" = "${CURRENT_PHASE}" ] || { echo "checkpoint changed concurrently" >&2; exit 1; }
  fi
  updated=$(printf '%s\n' "${current_object}" | jq \
    --arg id "${RECOVERY_ID}" --arg generation "${GENERATION}" --arg phase "${phase}" --arg gm "${grant_mode}" --arg rm "${rauthy_mode}" \
    --arg gs "$(jq -r '.grant.restore.sha256' "${RECORD}")" --arg rs "$(jq -r '.rauthy.restore.sha256' "${RECORD}")" \
    --arg gr "${grant_revision}" --arg rr "${rauthy_revision}" '
      .metadata.labels["app.kubernetes.io/managed-by"]="grant-recovery-operator" |
      .data += {"recovery-id":$id,generation:$generation,phase:$phase,"grant-mode":$gm,"rauthy-mode":$rm,"grant-sha256":$gs,"rauthy-sha256":$rs,"grant-revision":$gr,"rauthy-revision":$rr}
    ')
  printf '%s\n' "${updated}" | kubectl replace -f - >/dev/null || { echo "checkpoint CAS failed" >&2; exit 1; }
  CURRENT_PHASE=${phase}
  if [ "${RESTORE_STOP_AFTER_PHASE:-}" = "${phase}" ]; then
    log "intentional drill stop after checkpoint ${phase}; traffic remains fenced"
    exit 75
  fi
}

set_active_lifecycle_modes() {
  grant_mode=$1 rauthy_mode=$2
  if [ "${DRY_RUN}" -eq 1 ]; then
    log "dry-run: ensure active lifecycle modes (grant=${grant_mode}, rauthy=${rauthy_mode}) at ${CURRENT_PHASE}"
    return
  fi
  current_object=$(kubectl -n "${NAMESPACE}" get configmap "${STATE_CONFIGMAP}" -o json)
  [ "$(printf '%s\n' "${current_object}" | jq -r '.data["recovery-id"] // empty')" = "${RECOVERY_ID}" ] \
    && [ "$(printf '%s\n' "${current_object}" | jq -r '.data.phase // empty')" = "${CURRENT_PHASE}" ] \
    || { echo "checkpoint changed before lifecycle mode transition" >&2; exit 1; }
  current_grant_mode=$(printf '%s\n' "${current_object}" | jq -r '.data["grant-mode"] // empty')
  current_rauthy_mode=$(printf '%s\n' "${current_object}" | jq -r '.data["rauthy-mode"] // empty')
  if [ "${current_grant_mode}" = "${grant_mode}" ] && [ "${current_rauthy_mode}" = "${rauthy_mode}" ]; then
    return
  fi
  updated=$(printf '%s\n' "${current_object}" | jq --arg gm "${grant_mode}" --arg rm "${rauthy_mode}" \
    '.data["grant-mode"]=$gm | .data["rauthy-mode"]=$rm')
  printf '%s\n' "${updated}" | kubectl replace -f - >/dev/null \
    || { echo "lifecycle mode transition CAS failed" >&2; exit 1; }
}

sha256_file() {
  if command -v sha256sum >/dev/null 2>&1; then sha256sum "$1" | awk '{print $1}'
  else shasum -a 256 "$1" | awk '{print $1}'; fi
}

epoch_from_rfc3339() {
  if date -u -d "$1" +%s >/dev/null 2>&1; then date -u -d "$1" +%s
  else date -j -u -f '%Y-%m-%dT%H:%M:%SZ' "$1" +%s; fi
}

fence_start_epoch=$(epoch_from_rfc3339 "$(jq -r '.trafficWriteFence.startedAt' "${RECORD}")")
fence_end_epoch=$(epoch_from_rfc3339 "$(jq -r '.trafficWriteFence.completedAt' "${RECORD}")")
[ "${fence_start_epoch}" -le "${fence_end_epoch}" ] || { echo "invalid signed fence interval" >&2; exit 1; }
for workload in grant rauthy; do
  snapshot_epoch=$(epoch_from_rfc3339 "$(jq -r ".trafficWriteFence.snapshots.${workload}.createdAt" "${RECORD}")")
  [ "${snapshot_epoch}" -ge "${fence_start_epoch}" ] && [ "${snapshot_epoch}" -le "${fence_end_epoch}" ] \
    || { echo "${workload} signed snapshot timestamp is outside the fence interval" >&2; exit 1; }
done

verify_restore_object() {
  workload=$1
  bucket=$(jq -r ".${workload}.bucket" "${RECORD}")
  key=$(jq -r ".${workload}.restore.key" "${RECORD}")
  version=$(jq -r ".${workload}.restore.versionId" "${RECORD}")
  expected_sha=$(jq -r ".${workload}.restore.sha256" "${RECORD}")
  retain_until=$(jq -r ".${workload}.restore.retainUntil" "${RECORD}")
  policy_sid=$(jq -r ".${workload}.restore.immutablePolicySid" "${RECORD}")
  writer_deny_sid=$(jq -r ".${workload}.restore.writerReadDenyPolicySid" "${RECORD}")
  materialization_principal=$(jq -r ".${workload}.materializationPrincipal" "${RECORD}")
  writer_principal=$(jq -r ".${workload}.writerPrincipal" "${RECORD}")
  expected_restore_principal=$(jq -r ".${workload}.restoreReaderPrincipal" "${RECORD}")
  if [ "${DRY_RUN}" -eq 1 ]; then
    log "dry-run: would verify current immutable s3://${bucket}/${key}?versionId=${version} sha256=${expected_sha}"
    return
  fi
  case "${workload}" in
    grant) profile=${GRANT_AWS_PROFILE:?GRANT_AWS_PROFILE is required}; endpoint=${GRANT_S3_ENDPOINT_URL:-} ;;
    rauthy) profile=${RAUTHY_AWS_PROFILE:?RAUTHY_AWS_PROFILE is required}; endpoint=${RAUTHY_S3_ENDPOINT_URL:-} ;;
  esac
  [ "${endpoint}" = "$(jq -r ".${workload}.endpoint" "${RECORD}")" ] || { echo "${workload} S3 endpoint differs from signed record" >&2; exit 1; }
  aws_s3api() {
    if [ -n "${endpoint}" ]; then AWS_PROFILE=${profile} aws --endpoint-url "${endpoint}" s3api "$@"
    else AWS_PROFILE=${profile} aws s3api "$@"; fi
  }
  restore_principal=$(AWS_PROFILE=${profile} aws sts get-caller-identity --query Arn --output text)
  [ "${restore_principal}" = "${expected_restore_principal}" ] || { echo "${workload} restore reader differs from the signed record" >&2; exit 1; }
  [ "${restore_principal}" != "${materialization_principal}" ] || { echo "${workload} restore reader must differ from the materialization principal" >&2; exit 1; }
  [ "${restore_principal}" != "${writer_principal}" ] || { echo "${workload} restore reader must differ from the normal backup writer" >&2; exit 1; }
  head_values=$(aws_s3api head-object --bucket "${bucket}" --key "${key}" --query '[VersionId,ObjectLockMode,ObjectLockRetainUntilDate]' --output text)
  current_version=$(printf '%s\n' "${head_values}" | awk '{print $1}')
  lock_mode=$(printf '%s\n' "${head_values}" | awk '{print $2}')
  lock_until=$(printf '%s\n' "${head_values}" | awk '{print $3}' | sed 's/+00:00$/Z/')
  [ "${current_version}" = "${version}" ] || { echo "${workload} restore key current version changed" >&2; exit 1; }
  [ "${lock_mode}" = "COMPLIANCE" ] || { echo "${workload} restore key is not immutable" >&2; exit 1; }
  [ "${lock_until}" = "${retain_until}" ] || { echo "${workload} retention differs from the signed record" >&2; exit 1; }
  retain_epoch=$(epoch_from_rfc3339 "${retain_until}")
  [ "${retain_epoch}" -ge "$(( $(date -u +%s) + MIN_REMAINING_RETENTION_SECONDS ))" ] \
    || { echo "${workload} Object Lock expires before the required recovery window" >&2; exit 1; }
  target_arn="arn:aws:s3:::${bucket}/${key}"
  policy=$(aws_s3api get-bucket-policy --bucket "${bucket}" --query Policy --output text)
  printf '%s\n' "${policy}" | jq -e --arg sid "${policy_sid}" --arg writerSid "${writer_deny_sid}" --arg writer "${writer_principal}" --arg resource "${target_arn}" '
    def actions: (.Action | if type == "array" then . else [.] end);
    any(.Statement[]?; .Sid == $sid and .Effect == "Deny" and .Principal == "*" and .Resource == $resource and
      (actions | index("s3:PutObject")) != null and (actions | index("s3:DeleteObject")) != null and (actions | index("s3:DeleteObjectVersion")) != null) and
    any(.Statement[]?; .Sid == $writerSid and .Effect == "Deny" and .Principal.AWS == $writer and .Resource == $resource and
      (actions | index("s3:GetObject")) != null and (actions | index("s3:GetObjectVersion")) != null)
  ' >/dev/null || { echo "${workload} exact-key immutable bucket policy is missing" >&2; exit 1; }
  tmp=$(mktemp "${TMPDIR:-/tmp}/${workload}-restore.XXXXXX")
  aws_s3api get-object --bucket "${bucket}" --key "${key}" --version-id "${version}" "${tmp}" >/dev/null
  actual_sha=$(sha256_file "${tmp}")
  rm -f "${tmp}"
  [ "${actual_sha}" = "${expected_sha}" ] || { echo "${workload} restore checksum changed" >&2; exit 1; }
}

assert_runtime_binding() {
  workload=$1
  if [ "${DRY_RUN}" -eq 1 ]; then log "dry-run: would bind ${workload} bucket/endpoint/generation/schema/key to deployment"; return; fi
  case "${workload}" in
    grant) config=grant-production-backup; secret=grant-production-secrets ;;
    rauthy) config=rauthy-production-backup; secret=rauthy-production-secrets ;;
  esac
  object=$(kubectl -n "${NAMESPACE}" get configmap "${config}" -o json)
  for binding in HQL_S3_BUCKET:bucket HQL_S3_URL:endpoint BACKUP_GENERATION:backupGeneration RECOVERY_SCHEMA_ID:schemaId KEYRING_GENERATION:keyringGeneration; do
    config_key=${binding%%:*}; record_key=${binding#*:}
    actual=$(printf '%s\n' "${object}" | jq -r --arg key "${config_key}" '.data[$key] // empty')
    expected=$(jq -r ".${workload}.${record_key}" "${RECORD}")
    [ "${actual}" = "${expected}" ] || { echo "${workload} ${config_key} differs from signed record" >&2; exit 1; }
  done
  encoded=$(kubectl -n "${NAMESPACE}" get secret "${secret}" -o jsonpath='{.data.ENC_KEY_ACTIVE}')
  if printf '%s' "${encoded}" | base64 --decode >/dev/null 2>&1; then active=$(printf '%s' "${encoded}" | base64 --decode)
  else active=$(printf '%s' "${encoded}" | base64 -D); fi
  [ "${active}" = "$(jq -r ".${workload}.keyId" "${RECORD}")" ] || { echo "${workload} active encryption key differs from signed record" >&2; exit 1; }
  encoded_keyring=$(kubectl -n "${NAMESPACE}" get secret "${secret}" -o jsonpath='{.data.ENC_KEYS}')
  keyring_file=$(mktemp "${TMPDIR:-/tmp}/${workload}-keyring.XXXXXX")
  if printf '%s' "${encoded_keyring}" | base64 --decode >"${keyring_file}" 2>/dev/null; then :
  else printf '%s' "${encoded_keyring}" | base64 -D >"${keyring_file}"; fi
  keyring_sha=$(sha256_file "${keyring_file}")
  rm -f "${keyring_file}"
  [ "${keyring_sha}" = "$(jq -r ".${workload}.keyringSha256" "${RECORD}" | tr 'A-F' 'a-f')" ] \
    || { echo "${workload} complete encryption keyring digest differs from signed record" >&2; exit 1; }
}

fence_services() {
  patch=$(jq -nc --arg id "${RECOVERY_ID}" '{spec:{selector:{"grant-recovery-fence":$id}}}')
  run kubectl -n "${NAMESPACE}" patch service grant --type merge -p "${patch}"
  run kubectl -n "${NAMESPACE}" patch service rauthy --type merge -p "${patch}"
}

unfence_services() {
  run kubectl -n "${NAMESPACE}" patch service grant --type merge -p '{"spec":{"selector":{"app":"grant","role":"voter","grant-recovery-fence":null}}}'
  run kubectl -n "${NAMESPACE}" patch service rauthy --type merge -p '{"spec":{"selector":{"app":"rauthy","grant-recovery-fence":null}}}'
}

assert_image() {
  workload=$1 sts=$2 container=$3
  expected=$(jq -r ".${workload}.image" "${RECORD}")
  if [ "${DRY_RUN}" -eq 1 ]; then log "dry-run: would assert ${sts}/${container} image ${expected}"; return; fi
  actual=$(kubectl -n "${NAMESPACE}" get statefulset "${sts}" -o json | jq -r --arg name "${container}" '.spec.template.spec.containers[] | select(.name==$name) | .image')
  [ "${actual}" = "${expected}" ] || { echo "${sts} image mismatch: ${actual} != ${expected}" >&2; exit 1; }
}

arm_restore() {
  workload=$1 sts=$2 container=$3 secret=$4 key=$5
  if [ "${DRY_RUN}" -eq 1 ]; then
    log "dry-run: would create non-optional one-shot Secret ${secret} for s3:${key} and arm ${sts}/${container}"
    return
  fi
  case "${workload}" in
    grant) restore_profile=${GRANT_AWS_PROFILE:?GRANT_AWS_PROFILE is required} ;;
    rauthy) restore_profile=${RAUTHY_AWS_PROFILE:?RAUTHY_AWS_PROFILE is required} ;;
  esac
  restore_key=$(aws configure get aws_access_key_id --profile "${restore_profile}")
  restore_secret=$(aws configure get aws_secret_access_key --profile "${restore_profile}")
  [ -n "${restore_key}" ] && [ -n "${restore_secret}" ] || { echo "${workload} restore-reader static credentials are unavailable" >&2; exit 1; }
  b64() { printf '%s' "$1" | base64 | tr -d '\r\n'; }
  rid_b64=$(b64 "${RECOVERY_ID}")
  directive_b64=$(b64 "s3:${key}")
  key_b64=$(b64 "${restore_key}")
  secret_b64=$(b64 "${restore_secret}")
  printf '{"apiVersion":"v1","kind":"Secret","metadata":{"name":"%s","namespace":"%s"},"type":"Opaque","data":{"recovery-id":"%s","HQL_BACKUP_RESTORE":"%s","HQL_S3_KEY":"%s","HQL_S3_SECRET":"%s"}}\n' \
    "${secret}" "${NAMESPACE}" "${rid_b64}" "${directive_b64}" "${key_b64}" "${secret_b64}" | kubectl apply -f - >/dev/null
  unset restore_key restore_secret key_b64 secret_b64
  patch=$(jq -nc --arg id "${RECOVERY_ID}" '{spec:{updateStrategy:{type:"RollingUpdate",rollingUpdate:{partition:0}},template:{metadata:{annotations:{"grant.cypark.dev/restore-authorization":$id}}}}}')
  kubectl -n "${NAMESPACE}" patch statefulset "${sts}" --type strategic -p "${patch}" >/dev/null
}

consume_restore_secret() {
  secret=$1
  run kubectl -n "${NAMESPACE}" delete secret "${secret}" --ignore-not-found
  if [ "${DRY_RUN}" -eq 0 ] && kubectl -n "${NAMESPACE}" get secret "${secret}" >/dev/null 2>&1; then
    echo "one-shot restore Secret still exists: ${secret}" >&2
    exit 1
  fi
}

wait_ready() {
  sts=$1 replicas=$2
  run kubectl -n "${NAMESPACE}" rollout status "statefulset/${sts}" --timeout="${TIMEOUT}"
  ordinal=0
  while [ "${ordinal}" -lt "${replicas}" ]; do
    run kubectl -n "${NAMESPACE}" wait --for=condition=Ready "pod/${sts}-${ordinal}" --timeout="${TIMEOUT}"
    ordinal=$((ordinal + 1))
  done
}

decode_secret() {
  encoded=$(kubectl -n "${NAMESPACE}" get secret "$1" -o jsonpath='{.data.HQL_SECRET_API}')
  if printf '%s' "${encoded}" | base64 --decode >/dev/null 2>&1; then printf '%s' "${encoded}" | base64 --decode
  else printf '%s' "${encoded}" | base64 -D; fi
}

check_membership() {
  sts=$1 headless=$2 secret_name=$3 local_port=$4
  if [ "${DRY_RUN}" -eq 1 ]; then log "dry-run: would verify 3-voter membership and catch-up for ${sts}"; return; fi
  api_secret=$(decode_secret "${secret_name}")
  sanitized_secret=$(printf '%s' "${api_secret}" | tr -d '\r\n')
  [ "${sanitized_secret}" = "${api_secret}" ] || { echo "invalid API secret" >&2; exit 1; }
  curl_config=$(mktemp "${TMPDIR:-/tmp}/${sts}-curl.XXXXXX")
  escaped_secret=$(printf '%s' "${api_secret}" | sed 's/[\\"]/\\&/g')
  printf 'header = "X-API-SECRET: %s"\nheader = "Accept: application/json"\n' "${escaped_secret}" >"${curl_config}"
  chmod 0600 "${curl_config}"
  if [ "${sts}" = rauthy ]; then raft_types='sqlite cache'; else raft_types=sqlite; fi
  attempts=0
  while [ "${attempts}" -lt 60 ]; do
    ok=true
    for raft_type in ${raft_types}; do
      reference_leader='' reference_term='' reference_index=''
      ordinal=0
      while [ "${ordinal}" -lt 3 ]; do
        port=$((local_port + ordinal))
        log_file=$(mktemp "${TMPDIR:-/tmp}/${sts}-${ordinal}-port-forward.XXXXXX")
        metrics_file=$(mktemp "${TMPDIR:-/tmp}/${sts}-${ordinal}-${raft_type}.XXXXXX")
        kubectl -n "${NAMESPACE}" port-forward "pod/${sts}-${ordinal}" "${port}:8200" >"${log_file}" 2>&1 &
        PORT_FORWARD_PID=$!
        ready=0
        count=0
        while [ "${count}" -lt 20 ]; do
          if grep -q 'Forwarding from' "${log_file}"; then ready=1; break; fi
          count=$((count + 1)); sleep 1
        done
        if [ "${ready}" -eq 1 ] && curl -fsS --config "${curl_config}" "http://127.0.0.1:${port}/cluster/metrics/${raft_type}" >"${metrics_file}" 2>/dev/null; then
          jq -e --argjson id "$((ordinal + 1))" '
            .id == $id and (.state == "Leader" or .state == "Follower") and .current_leader != null and
            (.last_applied.index | type == "number") and
            (.membership_config.membership.configs | length == 1) and
            ((.membership_config.membership.configs[0] | sort) == [1,2,3])
          ' "${metrics_file}" >/dev/null || ok=false
          leader=$(jq -r '.current_leader' "${metrics_file}")
          term=$(jq -r '.current_term' "${metrics_file}")
          index=$(jq -r '.last_applied.index' "${metrics_file}")
          if [ -z "${reference_leader}" ]; then reference_leader=${leader}; reference_term=${term}; reference_index=${index}
          elif [ "${leader}" != "${reference_leader}" ] || [ "${term}" != "${reference_term}" ] || [ "${index}" != "${reference_index}" ]; then ok=false; fi
        else
          ok=false
        fi
        kill "${PORT_FORWARD_PID}" >/dev/null 2>&1 || true
        wait "${PORT_FORWARD_PID}" >/dev/null 2>&1 || true
        PORT_FORWARD_PID=
        rm -f "${log_file}" "${metrics_file}"
        ordinal=$((ordinal + 1))
      done
    done
    [ "${ok}" = true ] && break
    attempts=$((attempts + 1))
    sleep 2
  done
  rm -f "${curl_config}"
  [ "${attempts}" -lt 60 ] || { echo "timed out verifying ${sts} 3-voter membership and equal applied index" >&2; exit 1; }
}

expand_and_cleanup() {
  workload=$1 sts=$2 secret=$3 headless=$4 api_secret_name=$5 local_port=$6
  if [ "${workload}" = rauthy ]; then voters_phase=rauthy-voters-caught-up
  else voters_phase=grant-voters-caught-up; fi
  if ! at_least "${voters_phase}"; then
    if [ "${workload}" = rauthy ]; then
      # Fresh emptyDir peers have no completed launcher state or restore Secret. They must join
      # ordinal 0 in steady mode; restore mode intentionally fails closed without authorization.
      if [ "${DRY_RUN}" -eq 1 ]; then
        log "dry-run: would require absent ${secret} and revalidate ${sts}-0 Ready before peer scale-up"
      else
        if kubectl -n "${NAMESPACE}" get secret "${secret}" >/dev/null 2>&1; then
          echo "Rauthy restore Secret still exists before peer scale-up" >&2
          exit 1
        fi
        wait_ready "${sts}" 1
      fi
      set_active_lifecycle_modes restore steady
    fi
    run kubectl -n "${NAMESPACE}" patch statefulset "${sts}" --type merge -p '{"spec":{"updateStrategy":{"type":"RollingUpdate","rollingUpdate":{"partition":1}},"template":{"metadata":{"annotations":{"grant.cypark.dev/restore-authorization":null}}}}}'
    run kubectl -n "${NAMESPACE}" scale "statefulset/${sts}" --replicas=3
    wait_ready "${sts}" 3
    check_membership "${sts}" "${headless}" "${api_secret_name}" "${local_port}"
    if [ "${workload}" = rauthy ]; then update_state rauthy-voters-caught-up restore steady
    else update_state grant-voters-caught-up steady steady; fi
  fi
  run kubectl -n "${NAMESPACE}" patch statefulset "${sts}" --type merge -p '{"spec":{"updateStrategy":{"type":"RollingUpdate","rollingUpdate":{"partition":0}}}}'
  wait_ready "${sts}" 3
  check_membership "${sts}" "${headless}" "${api_secret_name}" "${local_port}"
  if [ "${DRY_RUN}" -eq 0 ]; then
    kubectl -n "${NAMESPACE}" get statefulset "${sts}" -o json | jq -e '[.spec.template.spec.containers[].env[]? | select(.name=="HQL_BACKUP_RESTORE")] | length == 0' >/dev/null
    kubectl -n "${NAMESPACE}" get secret "${secret}" >/dev/null 2>&1 && { echo "restore Secret unexpectedly exists" >&2; exit 1; }
  fi
}

run_semantic_gate() {
  if [ "${DRY_RUN}" -eq 1 ]; then log "dry-run: would run fixed Rauthy health and Grant application-authz semantic gate"; return; fi
  [ -r "${SEMANTIC_CONFIG}" ] || { echo "--semantic-config is required before traffic can be unfenced" >&2; exit 1; }
  : "${RECOVERY_SEMANTIC_BEARER_TOKEN:?RECOVERY_SEMANTIC_BEARER_TOKEN is required}"
  : "${RECOVERY_SEMANTIC_APP_AUTH_KEY:?RECOVERY_SEMANTIC_APP_AUTH_KEY is required}"
  jq -e '
    (.organizationId | type == "string" and length > 0) and (.action | type == "string" and length > 0) and
    (.resource.provider | type == "string" and length > 0) and (.resource.type | type == "string" and length > 0) and
    (.resource.id | type == "string" and length > 0) and (.expectedAllowed | type == "boolean")
  ' "${SEMANTIC_CONFIG}" >/dev/null || { echo "invalid semantic config" >&2; exit 1; }
  canonical_semantic=$(mktemp "${TMPDIR:-/tmp}/recovery-semantic.XXXXXX")
  jq -S -c . "${SEMANTIC_CONFIG}" >"${canonical_semantic}"
  semantic_sha=$(openssl dgst -sha256 "${canonical_semantic}" | awk '{print $NF}')
  if [ "${ISOLATED_DRILL}" -eq 0 ]; then
    [ "${semantic_sha}" = "$(jq -r '.approval.semanticEvidenceSha256' "${RECORD}")" ] || { echo "semantic config differs from signed approval evidence" >&2; exit 1; }
  fi
  service_json=$(jq -nc --arg name "${SEMANTIC_SERVICE}" --arg ns "${NAMESPACE}" \
    '{apiVersion:"v1",kind:"Service",metadata:{name:$name,namespace:$ns,labels:{"grant.cypark.dev/recovery-only":"true"}},spec:{selector:{app:"rauthy"},ports:[{name:"http",port:8080,targetPort:8080}]}}')
  printf '%s\n' "${service_json}" | kubectl apply -f - >/dev/null
  route_patch=$(jq -nc --arg base "http://${SEMANTIC_SERVICE}:8080" \
    '{spec:{template:{spec:{containers:[{name:"grant",env:[{name:"GRANT_RAUTHY_BASE_URL",value:$base,valueFrom:null}]}]}}}}')
  kubectl -n "${NAMESPACE}" patch statefulset grant-voter --type strategic -p "${route_patch}" >/dev/null
  SEMANTIC_ROUTE_ACTIVE=1
  wait_ready grant-voter 3
  check_membership grant-voter grant-headless "${GRANT_SECRET_NAME}" 28210

  rauthy_log=$(mktemp "${TMPDIR:-/tmp}/rauthy-semantic-pf.XXXXXX")
  kubectl -n "${NAMESPACE}" port-forward pod/rauthy-0 28301:8080 >"${rauthy_log}" 2>&1 & PORT_FORWARD_PID=$!
  sleep 2
  if ! curl -fsS http://127.0.0.1:28301/auth/v1/health >/dev/null; then
    kill "${PORT_FORWARD_PID}" >/dev/null 2>&1 || true; PORT_FORWARD_PID=
    fence_services; echo "Rauthy semantic health probe failed" >&2; exit 1
  fi
  kill "${PORT_FORWARD_PID}" >/dev/null 2>&1 || true; wait "${PORT_FORWARD_PID}" >/dev/null 2>&1 || true; PORT_FORWARD_PID=; rm -f "${rauthy_log}"

  grant_log=$(mktemp "${TMPDIR:-/tmp}/grant-semantic-pf.XXXXXX")
  kubectl -n "${NAMESPACE}" port-forward pod/grant-voter-0 28300:8080 >"${grant_log}" 2>&1 & PORT_FORWARD_PID=$!
  sleep 2
  semantic_curl=$(mktemp "${TMPDIR:-/tmp}/grant-semantic-curl.XXXXXX")
  org=$(jq -r '.organizationId' "${SEMANTIC_CONFIG}")
  token=$(printf '%s' "${RECOVERY_SEMANTIC_BEARER_TOKEN}" | sed 's/[\\"]/\\&/g')
  app_key=$(printf '%s' "${RECOVERY_SEMANTIC_APP_AUTH_KEY}" | sed 's/[\\"]/\\&/g')
  printf 'header = "Authorization: Bearer %s"\nheader = "x-organization-id: %s"\nheader = "x-grant-app-auth-key: %s"\nheader = "content-type: application/json"\n' \
    "${token}" "${org}" "${app_key}" >"${semantic_curl}"
  jq '{action,resource}' "${SEMANTIC_CONFIG}" >"${canonical_semantic}.request"
  response=$(curl -fsS --config "${semantic_curl}" --data-binary "@${canonical_semantic}.request" http://127.0.0.1:28300/api/v1/application-authz/check 2>/dev/null || true)
  expected=$(jq -r '.expectedAllowed' "${SEMANTIC_CONFIG}")
  actual=$(printf '%s\n' "${response}" | jq -r '.allowed // empty' 2>/dev/null || true)
  kill "${PORT_FORWARD_PID}" >/dev/null 2>&1 || true; wait "${PORT_FORWARD_PID}" >/dev/null 2>&1 || true; PORT_FORWARD_PID=
  rm -f "${grant_log}" "${semantic_curl}" "${canonical_semantic}" "${canonical_semantic}.request"
  [ "${actual}" = "${expected}" ] || { fence_services; echo "Grant application-authz semantic check failed" >&2; exit 1; }
  restore_patch='{"spec":{"template":{"spec":{"containers":[{"name":"grant","env":[{"name":"GRANT_RAUTHY_BASE_URL","value":null,"valueFrom":{"configMapKeyRef":{"name":"grant-production-config","key":"GRANT_RAUTHY_BASE_URL"}}}]}]}}}}'
  kubectl -n "${NAMESPACE}" patch statefulset grant-voter --type strategic -p "${restore_patch}" >/dev/null
  wait_ready grant-voter 3
  check_membership grant-voter grant-headless "${GRANT_SECRET_NAME}" 28210
  kubectl -n "${NAMESPACE}" delete service "${SEMANTIC_SERVICE}" --ignore-not-found >/dev/null
  SEMANTIC_ROUTE_ACTIVE=0
}

release_completed_recovery() {
  [ "${CURRENT_PHASE}" = unfenced ] || { echo "only an unfenced terminal recovery can be released" >&2; exit 1; }
  if [ "${DRY_RUN}" -eq 1 ]; then
    log "dry-run: would require direct-writer release evidence, resume signed CronJobs, remove the namespace fence label, and CAS checkpoint ownership to none"
    return
  fi
  release_annotation=$(kubectl get namespace "${NAMESPACE}" -o json \
    | jq -r '.metadata.annotations["grant.cypark.dev/recovery-direct-writers-released"] // empty')
  [ "${release_annotation}" = "${RECOVERY_ID}" ] || {
    echo "direct writers are not explicitly released for ${RECOVERY_ID}; set the protected namespace annotation after external writers are resumed" >&2
    exit 1
  }
  cron_file=$(mktemp "${TMPDIR:-/tmp}/recovery-cronjobs.XXXXXX")
  jq -r '.trafficWriteFence.release.cronJobsToResume[]?' "${RECORD}" >"${cron_file}"
  while IFS= read -r cronjob; do
    [ -n "${cronjob}" ] || continue
    if [ "${ISOLATED_DRILL}" -eq 1 ]; then continue; fi
    kubectl -n "${NAMESPACE}" get cronjob "${cronjob}" >/dev/null
    kubectl -n "${NAMESPACE}" patch cronjob "${cronjob}" --type merge -p '{"spec":{"suspend":false}}' >/dev/null
  done <"${cron_file}"
  rm -f "${cron_file}"
  kubectl label namespace "${NAMESPACE}" grant.cypark.dev/recovery-write-fence- >/dev/null 2>&1 || true
  current_object=$(kubectl -n "${NAMESPACE}" get configmap "${STATE_CONFIGMAP}" -o json)
  [ "$(printf '%s\n' "${current_object}" | jq -r '.data["recovery-id"]')" = "${RECOVERY_ID}" ] \
    && [ "$(printf '%s\n' "${current_object}" | jq -r '.data.phase')" = unfenced ] \
    || { echo "terminal checkpoint changed before release" >&2; exit 1; }
  released_at=$(date -u '+%Y-%m-%dT%H:%M:%SZ')
  updated=$(printf '%s\n' "${current_object}" | jq --arg id "${RECOVERY_ID}" --arg generation "${GENERATION}" --arg releasedAt "${released_at}" '
    .data += {"recovery-id":"none",phase:"unfenced","grant-mode":"steady","rauthy-mode":"steady",
      "last-recovery-id":$id,"last-generation":$generation,"last-released-at":$releasedAt,
      "last-grant-sha256":.data["grant-sha256"],"last-rauthy-sha256":.data["rauthy-sha256"]}')
  printf '%s\n' "${updated}" | kubectl replace -f - >/dev/null || { echo "terminal checkpoint release CAS failed" >&2; exit 1; }
  kubectl annotate namespace "${NAMESPACE}" "grant.cypark.dev/recovery-last-released=${RECOVERY_ID}" --overwrite >/dev/null
  log "released completed recovery ${RECOVERY_ID}; terminal audit remains in ${STATE_CONFIGMAP}"
}

if at_least unfenced; then
  if [ "${DRY_RUN}" -eq 0 ]; then
    for service in grant rauthy; do
      kubectl -n "${NAMESPACE}" get service "${service}" -o json | jq -e '.spec.selector["grant-recovery-fence"] == null' >/dev/null \
        || { echo "checkpoint says unfenced but service ${service} remains fenced" >&2; exit 1; }
    done
    [ "$(kubectl -n "${NAMESPACE}" get statefulset grant-voter -o jsonpath='{.spec.replicas}')" = 3 ] \
      && [ "$(kubectl -n "${NAMESPACE}" get statefulset rauthy -o jsonpath='{.spec.replicas}')" = 3 ] \
      || { echo "checkpoint says unfenced but clusters are not at three replicas" >&2; exit 1; }
  fi
  if [ "${RELEASE_COMPLETED}" -eq 1 ]; then release_completed_recovery
  else log "full-cluster recovery ${RECOVERY_ID} is already unfenced; no changes required"; fi
  exit 0
fi

if [ "${DRY_RUN}" -eq 0 ]; then
  : "${GRANT_AWS_PROFILE:?GRANT_AWS_PROFILE is required}"
  : "${RAUTHY_AWS_PROFILE:?RAUTHY_AWS_PROFILE is required}"
  [ "${GRANT_AWS_PROFILE}" != "${RAUTHY_AWS_PROFILE}" ] || { echo "distinct restore AWS profiles are required" >&2; exit 1; }
  grant_reader=$(AWS_PROFILE=${GRANT_AWS_PROFILE} aws sts get-caller-identity --query Arn --output text)
  rauthy_reader=$(AWS_PROFILE=${RAUTHY_AWS_PROFILE} aws sts get-caller-identity --query Arn --output text)
  [ "${grant_reader}" != "${rauthy_reader}" ] || { echo "Grant and Rauthy restore readers must be distinct principals" >&2; exit 1; }
fi

fence_services
verify_restore_object rauthy
verify_restore_object grant
assert_image rauthy rauthy rauthy
assert_image grant grant-voter grant
assert_runtime_binding rauthy
assert_runtime_binding grant

case "${REARM_ORDINAL0}" in
  '') ;;
  rauthy)
    [ "${CURRENT_PHASE}" = rauthy-ordinal0-restored ] || { echo "Rauthy re-arm is only valid at rauthy-ordinal0-restored" >&2; exit 2; }
    set_active_lifecycle_modes restore restore
    RAUTHY_KEY=$(jq -r '.rauthy.restore.key' "${RECORD}")
    arm_restore rauthy rauthy rauthy rauthy-restore-once "${RAUTHY_KEY}"
    run kubectl -n "${NAMESPACE}" delete pod rauthy-0 --ignore-not-found
    run kubectl -n "${NAMESPACE}" scale statefulset/rauthy --replicas=1
    wait_ready rauthy 1
    verify_restore_object rauthy
    consume_restore_secret rauthy-restore-once
    set_active_lifecycle_modes restore steady
    ;;
  grant)
    [ "${CURRENT_PHASE}" = grant-ordinal0-restored ] || { echo "Grant re-arm is only valid at grant-ordinal0-restored" >&2; exit 2; }
    GRANT_KEY=$(jq -r '.grant.restore.key' "${RECORD}")
    arm_restore grant grant-voter grant grant-restore-once "${GRANT_KEY}"
    run kubectl -n "${NAMESPACE}" delete pod grant-voter-0 --ignore-not-found
    run kubectl -n "${NAMESPACE}" scale statefulset/grant-voter --replicas=1
    wait_ready grant-voter 1
    verify_restore_object grant
    consume_restore_secret grant-restore-once
    ;;
  *) echo "--rearm-ordinal0 must be grant or rauthy" >&2; exit 2 ;;
esac

if ! at_least rauthy-restore-armed; then
  run kubectl -n "${NAMESPACE}" scale statefulset/rauthy --replicas=0
  run kubectl -n "${NAMESPACE}" scale statefulset/grant-voter --replicas=0
  RAUTHY_KEY=$(jq -r '.rauthy.restore.key' "${RECORD}")
  arm_restore rauthy rauthy rauthy rauthy-restore-once "${RAUTHY_KEY}"
  update_state rauthy-restore-armed restore restore
fi
if ! at_least rauthy-ordinal0-restored; then
  if [ "${DRY_RUN}" -eq 0 ] && ! kubectl -n "${NAMESPACE}" get secret rauthy-restore-once >/dev/null 2>&1; then
    RAUTHY_KEY=$(jq -r '.rauthy.restore.key' "${RECORD}")
    arm_restore rauthy rauthy rauthy rauthy-restore-once "${RAUTHY_KEY}"
  fi
  run kubectl -n "${NAMESPACE}" scale statefulset/rauthy --replicas=1
  wait_ready rauthy 1
  verify_restore_object rauthy
  consume_restore_secret rauthy-restore-once
  update_state rauthy-ordinal0-restored restore steady
fi
if ! at_least rauthy-cleanup-complete; then
  expand_and_cleanup rauthy rauthy rauthy-restore-once rauthy-headless "${RAUTHY_SECRET_NAME}" 28200
  update_state rauthy-cleanup-complete restore steady
fi

if ! at_least grant-restore-armed; then
  GRANT_KEY=$(jq -r '.grant.restore.key' "${RECORD}")
  arm_restore grant grant-voter grant grant-restore-once "${GRANT_KEY}"
  update_state grant-restore-armed restore steady
fi
if ! at_least grant-ordinal0-restored; then
  if [ "${DRY_RUN}" -eq 0 ] && ! kubectl -n "${NAMESPACE}" get secret grant-restore-once >/dev/null 2>&1; then
    GRANT_KEY=$(jq -r '.grant.restore.key' "${RECORD}")
    arm_restore grant grant-voter grant grant-restore-once "${GRANT_KEY}"
  fi
  run kubectl -n "${NAMESPACE}" scale statefulset/grant-voter --replicas=1
  wait_ready grant-voter 1
  verify_restore_object grant
  consume_restore_secret grant-restore-once
  update_state grant-ordinal0-restored restore steady
fi
if ! at_least cleanup-complete; then
  expand_and_cleanup grant grant-voter grant-restore-once grant-headless "${GRANT_SECRET_NAME}" 28210
  update_state cleanup-complete steady steady
fi

if ! at_least semantic-verified; then
  check_membership rauthy rauthy-headless "${RAUTHY_SECRET_NAME}" 28200
  check_membership grant-voter grant-headless "${GRANT_SECRET_NAME}" 28210
  verify_restore_object rauthy
  verify_restore_object grant
  run_semantic_gate
  update_state semantic-verified steady steady
fi

if ! at_least unfenced; then
  unfence_services
  update_state unfenced steady steady
fi

if [ "${RELEASE_COMPLETED}" -eq 1 ]; then release_completed_recovery; fi

log "full-cluster recovery ${RECOVERY_ID} reached phase ${CURRENT_PHASE}"
