#!/bin/sh
set -eu

ROOT_DIR=$(CDPATH='' cd -P "$(dirname "$0")/.." && pwd)
DRY_RUN=0
NAMESPACE=${NAMESPACE:-grant}
OUTPUT=${OUTPUT:-recovery-pair.json}

while [ "$#" -gt 0 ]; do
  case "$1" in
    --dry-run) DRY_RUN=1 ;;
    --namespace) shift; NAMESPACE=${1:?missing namespace} ;;
    --output) shift; OUTPUT=${1:?missing output path} ;;
    *) echo "usage: $0 [--dry-run] [--namespace NAME] [--output FILE]" >&2; exit 2 ;;
  esac
  shift
done

require_var() {
  eval "value=\${$1:-}"
  [ -n "${value}" ] || { echo "required environment variable is missing: $1" >&2; exit 2; }
}

for name in RECOVERY_ID RECOVERY_GENERATION RESTORE_RETAIN_UNTIL ISOLATED_DRILL_ID \
  GRANT_BUCKET GRANT_SOURCE_KEY GRANT_SOURCE_VERSION_ID GRANT_SOURCE_SHA256 GRANT_IMAGE GRANT_SCHEMA_ID GRANT_KEY_ID GRANT_BACKUP_GENERATION \
  GRANT_KEYRING_SHA256 GRANT_KEYRING_GENERATION GRANT_WRITER_PRINCIPAL_ARN GRANT_RESTORE_READER_PRINCIPAL_ARN \
  RAUTHY_BUCKET RAUTHY_SOURCE_KEY RAUTHY_SOURCE_VERSION_ID RAUTHY_SOURCE_SHA256 RAUTHY_IMAGE RAUTHY_SCHEMA_ID RAUTHY_KEY_ID RAUTHY_BACKUP_GENERATION \
  RAUTHY_KEYRING_SHA256 RAUTHY_KEYRING_GENERATION RAUTHY_WRITER_PRINCIPAL_ARN RAUTHY_RESTORE_READER_PRINCIPAL_ARN; do
  require_var "${name}"
done
case "${RECOVERY_ID}" in ''|*[!a-z0-9-]*|-*|*-) echo "RECOVERY_ID must be a lowercase DNS label" >&2; exit 2 ;; esac
[ "${#RECOVERY_ID}" -le 32 ] || { echo "RECOVERY_ID must be at most 32 characters" >&2; exit 2; }
[ "${GRANT_BUCKET}" != "${RAUTHY_BUCKET}" ] || { echo "Grant and Rauthy must use distinct buckets" >&2; exit 2; }
GRANT_AWS_PROFILE=${GRANT_AWS_PROFILE:-grant-dry-run}
RAUTHY_AWS_PROFILE=${RAUTHY_AWS_PROFILE:-rauthy-dry-run}
[ "${GRANT_AWS_PROFILE}" != "${RAUTHY_AWS_PROFILE}" ] || { echo "distinct workload AWS profiles are required" >&2; exit 2; }
[ "${GRANT_WRITER_PRINCIPAL_ARN}" != "${GRANT_RESTORE_READER_PRINCIPAL_ARN}" ] \
  && [ "${RAUTHY_WRITER_PRINCIPAL_ARN}" != "${RAUTHY_RESTORE_READER_PRINCIPAL_ARN}" ] \
  && [ "${GRANT_RESTORE_READER_PRINCIPAL_ARN}" != "${RAUTHY_RESTORE_READER_PRINCIPAL_ARN}" ] \
  || { echo "writer and restore-reader principals must be separated by role and workload" >&2; exit 2; }

command -v jq >/dev/null 2>&1 || { echo "missing command: jq" >&2; exit 1; }
TMP_DIR=$(mktemp -d "${TMPDIR:-/tmp}/grant-pair.XXXXXX")
cleanup() { rm -rf "${TMP_DIR}"; }
trap cleanup 0 HUP INT TERM

aws_call() {
  profile=$1 endpoint=$2
  shift 2
  if [ -n "${endpoint}" ]; then AWS_PROFILE=${profile} aws --endpoint-url "${endpoint}" "$@"
  else AWS_PROFILE=${profile} aws "$@"; fi
}

epoch_from_rfc3339() {
  if date -u -d "$1" +%s >/dev/null 2>&1; then date -u -d "$1" +%s
  else date -j -u -f '%Y-%m-%dT%H:%M:%SZ' "$1" +%s; fi
}

if [ "${DRY_RUN}" -eq 1 ]; then
  NAMESPACE_UID=dry-run-namespace-uid
  FENCE_STARTED_AT=2000-01-01T00:00:00Z
  FENCE_COMPLETED_AT=2099-01-01T00:00:00Z
  FENCE_EVIDENCE_SHA256=dry-run-fence-evidence
  FENCE_SIGNER_FINGERPRINT=dry-run-fence-signer
  GRANT_SNAPSHOT_ID=dry-run-grant-snapshot
  GRANT_SNAPSHOT_CREATED_AT=2000-01-01T00:00:01Z
  RAUTHY_SNAPSHOT_ID=dry-run-rauthy-snapshot
  RAUTHY_SNAPSHOT_CREATED_AT=2000-01-01T00:00:01Z
  CRONJOBS_TO_RESUME_JSON='[]'
else
  for name in FENCE_EVIDENCE FENCE_SIGNATURE FENCE_PUBLIC_KEY RECOVERY_FENCE_PUBLIC_KEY_SHA256; do require_var "${name}"; done
  for command in kubectl aws openssl; do command -v "${command}" >/dev/null 2>&1 || { echo "missing command: ${command}" >&2; exit 1; }; done
  [ -r "${FENCE_EVIDENCE}" ] && [ -r "${FENCE_SIGNATURE}" ] && [ -r "${FENCE_PUBLIC_KEY}" ] || { echo "fence evidence/signature/public key must be readable" >&2; exit 2; }
  jq -S -c . "${FENCE_EVIDENCE}" >"${TMP_DIR}/fence.canonical.json"
  actual_fence_fingerprint=$(openssl pkey -pubin -in "${FENCE_PUBLIC_KEY}" -outform DER | openssl dgst -sha256 | awk '{print $NF}')
  [ "${actual_fence_fingerprint}" = "${RECOVERY_FENCE_PUBLIC_KEY_SHA256}" ] || { echo "fence signer public key is not the pinned trust root" >&2; exit 1; }
  openssl dgst -sha256 -verify "${FENCE_PUBLIC_KEY}" -signature "${FENCE_SIGNATURE}" "${TMP_DIR}/fence.canonical.json" >/dev/null \
    || { echo "fence evidence signature verification failed" >&2; exit 1; }
  namespace_object=$(kubectl get namespace "${NAMESPACE}" -o json)
  NAMESPACE_UID=$(printf '%s\n' "${namespace_object}" | jq -r '.metadata.uid // empty')
  label=$(printf '%s\n' "${namespace_object}" | jq -r '.metadata.labels["grant.cypark.dev/recovery-write-fence"] // empty')
  [ "${label}" = "${RECOVERY_ID}" ] || { echo "namespace is not labeled for this recovery write fence" >&2; exit 1; }
  jq -e --arg id "${RECOVERY_ID}" --arg ns "${NAMESPACE}" --arg uid "${NAMESPACE_UID}" \
    --arg gk "${GRANT_SOURCE_KEY}" --arg gv "${GRANT_SOURCE_VERSION_ID}" --arg rk "${RAUTHY_SOURCE_KEY}" --arg rv "${RAUTHY_SOURCE_VERSION_ID}" '
      .version == 1 and .recoveryId == $id and .namespace == $ns and .namespaceUid == $uid and
      .directWritersStopped == true and .grantSourceVersionId == $gv and .rauthySourceVersionId == $rv and
      (.startedAt | type == "string" and length > 0) and (.completedAt | type == "string" and length > 0) and
      (.snapshots.grant.snapshotId | type == "string" and length > 0) and
      .snapshots.grant.sourceKey == $gk and .snapshots.grant.sourceVersionId == $gv and
      (.snapshots.grant.createdAt | type == "string" and length > 0) and
      (.snapshots.rauthy.snapshotId | type == "string" and length > 0) and
      .snapshots.rauthy.sourceKey == $rk and .snapshots.rauthy.sourceVersionId == $rv and
      (.snapshots.rauthy.createdAt | type == "string" and length > 0) and
      ((.release.cronJobsToResume // []) | type == "array" and all(.[]; type == "string" and length > 0))
    ' "${TMP_DIR}/fence.canonical.json" >/dev/null || { echo "fence evidence does not bind this namespace and source pair" >&2; exit 1; }
  for service in grant rauthy; do
    selector=$(kubectl -n "${NAMESPACE}" get service "${service}" -o json | jq -r '.spec.selector["grant-recovery-fence"] // empty')
    [ "${selector}" = "${RECOVERY_ID}" ] || { echo "service ${service} is not fenced for ${RECOVERY_ID}" >&2; exit 1; }
    kubectl -n "${NAMESPACE}" get endpoints "${service}" -o json | jq -e '(.subsets // []) | length == 0' >/dev/null \
      || { echo "service ${service} still has endpoints" >&2; exit 1; }
  done
  kubectl -n "${NAMESPACE}" get cronjob -o json | jq -e 'all(.items[]; .spec.suspend == true)' >/dev/null \
    || { echo "every CronJob in the recovery namespace must be suspended" >&2; exit 1; }
  FENCE_STARTED_AT=$(jq -er '.startedAt' "${TMP_DIR}/fence.canonical.json")
  FENCE_COMPLETED_AT=$(jq -er '.completedAt' "${TMP_DIR}/fence.canonical.json")
  start_epoch=$(epoch_from_rfc3339 "${FENCE_STARTED_AT}")
  end_epoch=$(epoch_from_rfc3339 "${FENCE_COMPLETED_AT}")
  [ "${start_epoch}" -le "${end_epoch}" ] || { echo "invalid fence evidence interval" >&2; exit 1; }
  GRANT_SNAPSHOT_ID=$(jq -er '.snapshots.grant.snapshotId' "${TMP_DIR}/fence.canonical.json")
  GRANT_SNAPSHOT_CREATED_AT=$(jq -er '.snapshots.grant.createdAt' "${TMP_DIR}/fence.canonical.json")
  RAUTHY_SNAPSHOT_ID=$(jq -er '.snapshots.rauthy.snapshotId' "${TMP_DIR}/fence.canonical.json")
  RAUTHY_SNAPSHOT_CREATED_AT=$(jq -er '.snapshots.rauthy.createdAt' "${TMP_DIR}/fence.canonical.json")
  CRONJOBS_TO_RESUME_JSON=$(jq -c '.release.cronJobsToResume // []' "${TMP_DIR}/fence.canonical.json")
  for workload in grant rauthy; do
    case "${workload}" in
      grant) profile=${GRANT_AWS_PROFILE}; endpoint=${GRANT_S3_ENDPOINT_URL:-}; bucket=${GRANT_BUCKET}; key=${GRANT_SOURCE_KEY}; version=${GRANT_SOURCE_VERSION_ID}; snapshot_created=${GRANT_SNAPSHOT_CREATED_AT} ;;
      rauthy) profile=${RAUTHY_AWS_PROFILE}; endpoint=${RAUTHY_S3_ENDPOINT_URL:-}; bucket=${RAUTHY_BUCKET}; key=${RAUTHY_SOURCE_KEY}; version=${RAUTHY_SOURCE_VERSION_ID}; snapshot_created=${RAUTHY_SNAPSHOT_CREATED_AT} ;;
    esac
    modified=$(aws_call "${profile}" "${endpoint}" s3api head-object --bucket "${bucket}" --key "${key}" --version-id "${version}" --query LastModified --output text)
    modified_epoch=$(epoch_from_rfc3339 "$(printf '%s\n' "${modified}" | sed 's/+00:00$/Z/')")
    snapshot_epoch=$(epoch_from_rfc3339 "${snapshot_created}")
    [ "${snapshot_epoch}" -ge "${start_epoch}" ] && [ "${snapshot_epoch}" -le "${end_epoch}" ] \
      || { echo "${workload} signed snapshot creation was outside the write-fence interval" >&2; exit 1; }
    [ "${snapshot_epoch}" -le "${modified_epoch}" ] \
      || { echo "${workload} snapshot creation timestamp is after its S3 upload completion" >&2; exit 1; }
    [ "${modified_epoch}" -ge "${start_epoch}" ] && [ "${modified_epoch}" -le "${end_epoch}" ] \
      || { echo "${workload} source version was not created inside the signed write-fence interval" >&2; exit 1; }
  done
  FENCE_EVIDENCE_SHA256=$(openssl dgst -sha256 "${TMP_DIR}/fence.canonical.json" | awk '{print $NF}')
  FENCE_SIGNER_FINGERPRINT=${actual_fence_fingerprint}
fi

materialize() {
  workload=$1 profile=$2 bucket=$3 key=$4 version=$5 sha=$6 endpoint=$7 writer=$8 output=$9
  if [ "${DRY_RUN}" -eq 1 ]; then flag=--dry-run; else flag=; fi
  # shellcheck disable=SC2086
  AWS_PROFILE=${profile} S3_ENDPOINT_URL=${endpoint} "${ROOT_DIR}/scripts/materialize-s3-restore.sh" ${flag} \
    "${workload}" "${bucket}" "${key}" "${version}" "${sha}" "${RECOVERY_ID}" "${RESTORE_RETAIN_UNTIL}" "${writer}" "${output}"
}

materialize grant "${GRANT_AWS_PROFILE}" "${GRANT_BUCKET}" "${GRANT_SOURCE_KEY}" "${GRANT_SOURCE_VERSION_ID}" "${GRANT_SOURCE_SHA256}" "${GRANT_S3_ENDPOINT_URL:-}" "${GRANT_WRITER_PRINCIPAL_ARN}" "${TMP_DIR}/grant.tsv"
materialize rauthy "${RAUTHY_AWS_PROFILE}" "${RAUTHY_BUCKET}" "${RAUTHY_SOURCE_KEY}" "${RAUTHY_SOURCE_VERSION_ID}" "${RAUTHY_SOURCE_SHA256}" "${RAUTHY_S3_ENDPOINT_URL:-}" "${RAUTHY_WRITER_PRINCIPAL_ARN}" "${TMP_DIR}/rauthy.tsv"

TAB=$(printf '\t')
IFS="${TAB}" read -r GRANT_RESTORE_KEY GRANT_RESTORE_VERSION GRANT_RESTORE_SHA GRANT_LOCK_UNTIL GRANT_POLICY_SID GRANT_WRITER_DENY_SID GRANT_PRINCIPAL <"${TMP_DIR}/grant.tsv"
IFS="${TAB}" read -r RAUTHY_RESTORE_KEY RAUTHY_RESTORE_VERSION RAUTHY_RESTORE_SHA RAUTHY_LOCK_UNTIL RAUTHY_POLICY_SID RAUTHY_WRITER_DENY_SID RAUTHY_PRINCIPAL <"${TMP_DIR}/rauthy.tsv"
[ "${GRANT_PRINCIPAL}" != "${RAUTHY_PRINCIPAL}" ] || { echo "Grant and Rauthy materialization principals must be distinct" >&2; exit 1; }
[ "${GRANT_PRINCIPAL}" != "${GRANT_WRITER_PRINCIPAL_ARN}" ] && [ "${GRANT_PRINCIPAL}" != "${GRANT_RESTORE_READER_PRINCIPAL_ARN}" ] \
  && [ "${RAUTHY_PRINCIPAL}" != "${RAUTHY_WRITER_PRINCIPAL_ARN}" ] && [ "${RAUTHY_PRINCIPAL}" != "${RAUTHY_RESTORE_READER_PRINCIPAL_ARN}" ] \
  || { echo "materialization principals must differ from writer and restore-reader principals" >&2; exit 1; }

umask 077
jq -n \
  --arg recoveryId "${RECOVERY_ID}" --arg generation "${RECOVERY_GENERATION}" --arg createdAt "$(date -u '+%Y-%m-%dT%H:%M:%SZ')" \
  --arg namespace "${NAMESPACE}" --arg namespaceUid "${NAMESPACE_UID}" --arg isolatedDrillId "${ISOLATED_DRILL_ID}" \
  --arg fenceStart "${FENCE_STARTED_AT}" --arg fenceEnd "${FENCE_COMPLETED_AT}" --arg fenceSha "${FENCE_EVIDENCE_SHA256}" --arg fenceSigner "${FENCE_SIGNER_FINGERPRINT}" \
  --arg gsid "${GRANT_SNAPSHOT_ID}" --arg gscreated "${GRANT_SNAPSHOT_CREATED_AT}" --arg rsid "${RAUTHY_SNAPSHOT_ID}" --arg rscreated "${RAUTHY_SNAPSHOT_CREATED_AT}" --argjson cronJobs "${CRONJOBS_TO_RESUME_JSON}" \
  --arg gb "${GRANT_BUCKET}" --arg gsk "${GRANT_SOURCE_KEY}" --arg gsv "${GRANT_SOURCE_VERSION_ID}" --arg gss "${GRANT_SOURCE_SHA256}" \
  --arg grk "${GRANT_RESTORE_KEY}" --arg grv "${GRANT_RESTORE_VERSION}" --arg grs "${GRANT_RESTORE_SHA}" --arg glu "${GRANT_LOCK_UNTIL}" --arg gps "${GRANT_POLICY_SID}" --arg gwds "${GRANT_WRITER_DENY_SID}" --arg gp "${GRANT_PRINCIPAL}" \
  --arg gi "${GRANT_IMAGE}" --arg gs "${GRANT_SCHEMA_ID}" --arg gkid "${GRANT_KEY_ID}" --arg gkring "${GRANT_KEYRING_SHA256}" --arg gkrgen "${GRANT_KEYRING_GENERATION}" --arg ggen "${GRANT_BACKUP_GENERATION}" --arg ge "${GRANT_S3_ENDPOINT_URL:-}" --arg gwp "${GRANT_WRITER_PRINCIPAL_ARN}" --arg grp "${GRANT_RESTORE_READER_PRINCIPAL_ARN}" \
  --arg rb "${RAUTHY_BUCKET}" --arg rsk "${RAUTHY_SOURCE_KEY}" --arg rsv "${RAUTHY_SOURCE_VERSION_ID}" --arg rss "${RAUTHY_SOURCE_SHA256}" \
  --arg rrk "${RAUTHY_RESTORE_KEY}" --arg rrv "${RAUTHY_RESTORE_VERSION}" --arg rrs "${RAUTHY_RESTORE_SHA}" --arg rlu "${RAUTHY_LOCK_UNTIL}" --arg rps "${RAUTHY_POLICY_SID}" --arg rwds "${RAUTHY_WRITER_DENY_SID}" --arg rp "${RAUTHY_PRINCIPAL}" \
  --arg ri "${RAUTHY_IMAGE}" --arg rs "${RAUTHY_SCHEMA_ID}" --arg rkid "${RAUTHY_KEY_ID}" --arg rkring "${RAUTHY_KEYRING_SHA256}" --arg rkrgen "${RAUTHY_KEYRING_GENERATION}" --arg rgen "${RAUTHY_BACKUP_GENERATION}" --arg re "${RAUTHY_S3_ENDPOINT_URL:-}" --arg rwp "${RAUTHY_WRITER_PRINCIPAL_ARN}" --arg rrp "${RAUTHY_RESTORE_READER_PRINCIPAL_ARN}" '
  {version:2,recoveryId:$recoveryId,generation:$generation,createdAt:$createdAt,namespace:$namespace,namespaceUid:$namespaceUid,
   trafficWriteFence:{startedAt:$fenceStart,completedAt:$fenceEnd,evidenceSha256:$fenceSha,signerFingerprint:$fenceSigner,directWritersStopped:true,
     snapshots:{grant:{snapshotId:$gsid,createdAt:$gscreated},rauthy:{snapshotId:$rsid,createdAt:$rscreated}},release:{cronJobsToResume:$cronJobs}},
   isolatedDrill:{id:$isolatedDrillId,passed:false},
   grant:{bucket:$gb,endpoint:$ge,backupGeneration:$ggen,keyringGeneration:$gkrgen,source:{key:$gsk,versionId:$gsv,sha256:$gss},restore:{key:$grk,versionId:$grv,sha256:$grs,retainUntil:$glu,immutablePolicySid:$gps,writerReadDenyPolicySid:$gwds},materializationPrincipal:$gp,writerPrincipal:$gwp,restoreReaderPrincipal:$grp,image:$gi,schemaId:$gs,keyId:$gkid,keyringSha256:$gkring},
   rauthy:{bucket:$rb,endpoint:$re,backupGeneration:$rgen,keyringGeneration:$rkrgen,source:{key:$rsk,versionId:$rsv,sha256:$rss},restore:{key:$rrk,versionId:$rrv,sha256:$rrs,retainUntil:$rlu,immutablePolicySid:$rps,writerReadDenyPolicySid:$rwds},materializationPrincipal:$rp,writerPrincipal:$rwp,restoreReaderPrincipal:$rrp,image:$ri,schemaId:$rs,keyId:$rkid,keyringSha256:$rkring}}
  ' >"${OUTPUT}"

echo "recovery pair written to ${OUTPUT}; write fence remains active until explicitly released" >&2
