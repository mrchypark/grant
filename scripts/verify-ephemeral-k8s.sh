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
EXPECTED_RAUTHY_LAUNCHER_IMAGE=ghcr.io/conalog/grant-rauthy-launcher:rauthy-0.36.0-9d95f0f7513b7e26559a87d22d9fb90ccf4ee530@sha256:d1bf34bc156f7227d782c231c3bb0111d1cb8317017f1539f8ef22cdc10f2f7a
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

read_rauthy_launcher_image() {
  awk '
    /^[[:space:]]*image:[[:space:]]*ghcr[.]io\/conalog\/grant-rauthy-launcher[^[:space:]]*/ {
      image = $0
      sub(/^[[:space:]]*image:[[:space:]]*/, "", image)
      sub(/[[:space:]].*$/, "", image)
      count++
    }
    END {
      if (count != 1 || image == "") exit 1
      print image
    }
  ' "$1"
}

assert_repository_rauthy_launcher_pin() {
  repository_rauthy_image=$(read_rauthy_launcher_image "$1") || return 1
  [ -n "${repository_rauthy_image}" ] \
    && [ "${repository_rauthy_image}" = "${EXPECTED_RAUTHY_LAUNCHER_IMAGE}" ]
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
  grep -q 'command -v od' "${ROOT_DIR}/deploy/k8s/lifecycle-gate.sh" \
    || fail "lifecycle gate does not fail closed when its byte-inspection tool is unavailable"
  if grep -E 'image: (busybox|ghcr.io/sebadob/rauthy)' "${manifest}" | grep -v '@sha256:' >/dev/null; then
    fail "${label} contains an unpinned third-party image"
  fi
  if grep -q 'image: ghcr.io/sebadob/rauthy' "${manifest}"; then
    fail "${label} runs the official distroless Rauthy image without the static launcher"
  fi
  expected_rauthy_image=$(read_rauthy_launcher_image "${ROOT_DIR}/deploy/k8s/rauthy-statefulset.yaml") \
    || fail "repository Rauthy launcher image is missing or ambiguous"
  [ -n "${expected_rauthy_image}" ] \
    || fail "repository Rauthy launcher image is empty"
  grep -Fq "image: ${expected_rauthy_image}" "${manifest}" \
    || fail "${label} does not use the repository Rauthy launcher image"
  [ "$(grep -c 'image: ghcr.io/conalog/grant-rauthy-launcher' "${manifest}" || true)" -eq 1 ] \
    || fail "${label} must render exactly one Rauthy launcher image"
  actual_rauthy_image=$(sed -n 's/^[[:space:]]*image:[[:space:]]*\(ghcr.io\/conalog\/grant-rauthy-launcher[^[:space:]]*\).*/\1/p' "${manifest}")
  [ "${actual_rauthy_image}" = "${expected_rauthy_image}" ] \
    || fail "${label} overrides or corrupts the Rauthy launcher image pin"
  grep -q '/usr/local/bin/rauthy-launcher' "${manifest}" \
    || fail "${label} does not execute the static Rauthy launcher"
  awk '
    previous_rauthy && $0 == "        - serve" { found = 1 }
    { previous_rauthy = $0 == "        - /app/rauthy" }
    END { exit found ? 0 : 1 }
  ' "${manifest}" \
    || fail "${label} does not pass the required Rauthy serve subcommand"
  grep -q 'name: rauthy-launcher-pull' "${manifest}" \
    || fail "${label} does not reference the private GHCR pull Secret"
}

assert_recovery_role_resolution_policy() {
  policy=${ROOT_DIR}/deploy/overlays/production/recovery-role-resolution-policy.example.json
  jq -e '
    .Version == "2012-10-17" and
    any(.Statement[]?;
      .Effect == "Allow" and .Action == "iam:GetRole" and
      (.Resource | type == "array" and length == 3) and
      all(.Resource[]; test("^arn:aws:iam::[0-9]{12}:role/.+")))
  ' "${policy}" >/dev/null \
    || fail "production recovery role-resolution policy does not grant scoped iam:GetRole"
}

assert_recovery_dry_run() {
  pair="${TMP_DIR}/pair.json"
  normalized="${TMP_DIR}/normalized-materialization.tsv"
  "${ROOT_DIR}/scripts/materialize-s3-restore.sh" --dry-run rauthy test-bucket source version \
    BBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBB ci-static 2030-01-01T00:00:00Z \
    arn:aws:iam::222222222222:role/writer "${normalized}"
  grep -q 'restore/rauthy/ci-static/bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb.enc' "${normalized}" \
    || fail "materialized restore SHA is not normalized to lowercase"
  rauthy_image=$(sed -n 's/^[[:space:]]*image:[[:space:]]*\(ghcr.io\/conalog\/grant-rauthy-launcher[^[:space:]]*\).*/\1/p' \
    "${ROOT_DIR}/deploy/k8s/rauthy-statefulset.yaml")
  [ -n "${rauthy_image}" ] || fail "cannot read the Rauthy launcher image pin"
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
  RAUTHY_IMAGE=${rauthy_image} \
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

  restore_early_tmp="${TMP_DIR}/restore-cleanup-early"
  mkdir -p "${restore_early_tmp}"
  printf '%s\n' '{}' >"${TMP_DIR}/invalid-recovery-record.json"
  if TMPDIR=${restore_early_tmp} "${ROOT_DIR}/scripts/restore-full-cluster.sh" \
    --dry-run --isolated-drill --record "${TMP_DIR}/invalid-recovery-record.json" \
    >"${TMP_DIR}/restore-cleanup-early.log" 2>&1; then
    fail "invalid recovery record unexpectedly passed the cleanup fixture"
  fi
  if find "${restore_early_tmp}" -name 'grant-restore-full.*' -print | grep -q .; then
    fail "restore early failure left private temporary state behind"
  fi

  restore_signal_tmp="${TMP_DIR}/restore-cleanup-signal"
  restore_signal_bin="${TMP_DIR}/restore-cleanup-signal-bin"
  restore_signal_marker="${TMP_DIR}/restore-cleanup-signal.marker"
  restore_signal_order_marker="${TMP_DIR}/restore-cleanup-signal-order.marker"
  restore_signal_order_failure="${TMP_DIR}/restore-cleanup-signal-order-failure.marker"
  restore_signal_kubectl_log="${TMP_DIR}/restore-cleanup-signal-kubectl.log"
  restore_signal_secret='restore-signal-secret-must-not-survive'
  mkdir -p "${restore_signal_tmp}" "${restore_signal_bin}"
  cat >"${restore_signal_bin}/kubectl" <<'EOF'
#!/bin/sh
set -eu
printf '%s\n' "$*" >>"${RESTORE_KUBECTL_LOG}"
case "$*" in
  "get namespace ${RESTORE_SIGNAL_NAMESPACE} -o json")
    printf '%s\n' '{"metadata":{"uid":"restore-signal-uid","labels":{"grant.cypark.dev/recovery-drill":"true"},"annotations":{"grant.cypark.dev/source-recovery-id":"ci-static"}}}'
    ;;
  "-n ${RESTORE_SIGNAL_NAMESPACE} get cronjob -o json")
    printf '%s\n' '{"items":[]}'
    ;;
  "create -f -")
    cat >/dev/null
    [ "${RESTORE_LEASE_MODE}" != acquire-fail ] || exit 1
    ;;
  "-n ${RESTORE_SIGNAL_NAMESPACE} get configmap grant-recovery-state")
    restore_dir=
    for candidate in "${TMPDIR}"/grant-restore-full.*; do
      if [ -d "${candidate}" ]; then restore_dir=${candidate}; break; fi
    done
    [ -n "${restore_dir}" ]
    printf '%s\n' "${RESTORE_SIGNAL_SECRET}" >"${restore_dir}/signal-secret"
    : >"${RESTORE_SIGNAL_MARKER}"
    kill -TERM "${PPID}"
    exit 143
    ;;
  "-n ${RESTORE_SIGNAL_NAMESPACE} patch service grant "*|"-n ${RESTORE_SIGNAL_NAMESPACE} patch service rauthy "*)
    if find "${TMPDIR}" -name 'grant-restore-full.*' -print | grep -q .; then
      : >"${RESTORE_SIGNAL_ORDER_FAILURE}"
    else
      : >"${RESTORE_SIGNAL_ORDER_MARKER}"
    fi
    ;;
  "-n ${RESTORE_SIGNAL_NAMESPACE} get lease grant-recovery-lock "*)
    printf '%s' 'another-recovery-operator'
    ;;
  "-n ${RESTORE_SIGNAL_NAMESPACE} delete lease grant-recovery-lock "*)
    : >"${RESTORE_SIGNAL_LEASE_DELETED}"
    ;;
  *)
    echo "unexpected kubectl invocation: $*" >&2
    exit 1
    ;;
esac
EOF
  cat >"${restore_signal_bin}/aws" <<'EOF'
#!/bin/sh
exit 0
EOF
  cat >"${restore_signal_bin}/curl" <<'EOF'
#!/bin/sh
exit 0
EOF
  chmod +x "${restore_signal_bin}/kubectl" "${restore_signal_bin}/aws" "${restore_signal_bin}/curl"
  set +e
  PATH="${restore_signal_bin}:${PATH}" TMPDIR=${restore_signal_tmp} \
    RESTORE_SIGNAL_NAMESPACE=grant-restore-signal RESTORE_SIGNAL_MARKER=${restore_signal_marker} \
    RESTORE_SIGNAL_ORDER_MARKER=${restore_signal_order_marker} RESTORE_SIGNAL_ORDER_FAILURE=${restore_signal_order_failure} \
    RESTORE_SIGNAL_LEASE_DELETED="${TMP_DIR}/restore-cleanup-signal-lease-deleted.marker" \
    RESTORE_SIGNAL_SECRET=${restore_signal_secret} RESTORE_KUBECTL_LOG=${restore_signal_kubectl_log} RESTORE_LEASE_MODE=acquire-success \
    "${ROOT_DIR}/scripts/restore-full-cluster.sh" --isolated-drill --record "${pair}" \
    --namespace grant-restore-signal --confirm-recovery-id ci-static --expected-namespace-uid restore-signal-uid \
    >"${TMP_DIR}/restore-cleanup-signal.log" 2>&1
  restore_signal_status=$?
  set -e
  [ "${restore_signal_status}" -eq 143 ] \
    || fail "restore TERM cleanup exited ${restore_signal_status}, expected 143"
  [ -e "${restore_signal_marker}" ] || fail "restore TERM cleanup fixture did not inject its signal"
  if find "${restore_signal_tmp}" -name 'grant-restore-full.*' -print | grep -q .; then
    fail "restore TERM cleanup left private temporary state behind"
  fi
  if grep -Fq "${restore_signal_secret}" "${TMP_DIR}/restore-cleanup-signal.log"; then
    fail "restore TERM cleanup exposed a temporary secret"
  fi
  [ -e "${restore_signal_order_marker}" ] \
    || fail "restore TERM cleanup did not re-fence after acquiring the recovery lease"
  [ ! -e "${restore_signal_order_failure}" ] \
    || fail "restore TERM cleanup contacted the cluster before deleting private temporary state"
  [ ! -e "${TMP_DIR}/restore-cleanup-signal-lease-deleted.marker" ] \
    || fail "restore TERM cleanup deleted a lease now owned by another operator"
  [ "$(grep -c 'patch service' "${restore_signal_kubectl_log}" || true)" -eq 2 ] \
    || fail "restore TERM cleanup did not re-fence both services"

  : >"${restore_signal_kubectl_log}"
  set +e
  PATH="${restore_signal_bin}:${PATH}" TMPDIR=${restore_signal_tmp} \
    RESTORE_SIGNAL_NAMESPACE=grant-restore-signal RESTORE_SIGNAL_MARKER=${restore_signal_marker} \
    RESTORE_SIGNAL_ORDER_MARKER=${restore_signal_order_marker} RESTORE_SIGNAL_ORDER_FAILURE=${restore_signal_order_failure} \
    RESTORE_SIGNAL_LEASE_DELETED="${TMP_DIR}/restore-cleanup-signal-lease-deleted.marker" \
    RESTORE_SIGNAL_SECRET=${restore_signal_secret} RESTORE_KUBECTL_LOG=${restore_signal_kubectl_log} RESTORE_LEASE_MODE=acquire-fail \
    "${ROOT_DIR}/scripts/restore-full-cluster.sh" --isolated-drill --record "${pair}" \
    --namespace grant-restore-signal --confirm-recovery-id ci-static --expected-namespace-uid restore-signal-uid \
    >"${TMP_DIR}/restore-cleanup-lease-failure.log" 2>&1
  restore_lease_failure_status=$?
  set -e
  [ "${restore_lease_failure_status}" -eq 1 ] \
    || fail "restore lease contention exited ${restore_lease_failure_status}, expected 1"
  if grep -q 'patch service' "${restore_signal_kubectl_log}"; then
    fail "restore cleanup re-fenced services without acquiring the recovery lease"
  fi
  [ ! -e "${TMP_DIR}/restore-cleanup-signal-lease-deleted.marker" ] \
    || fail "restore lease contention deleted another operator's lease"

  if "${ROOT_DIR}/scripts/restore-full-cluster.sh" --dry-run --record "${pair}" >/dev/null 2>&1; then
    fail "unapproved recovery record was accepted for production restore"
  fi
  recovery_dry_log="${TMP_DIR}/full-cluster-recovery.log"
  "${ROOT_DIR}/scripts/restore-full-cluster.sh" --dry-run --isolated-drill --record "${pair}" \
    >/dev/null 2>"${recovery_dry_log}"
  secret_delete_line=$(grep -n 'delete secret rauthy-restore-once' "${recovery_dry_log}" | head -n 1 | cut -d: -f1)
  steady_checkpoint_line=$(grep -n 'checkpoint rauthy-ordinal0-restored (grant=restore, rauthy=steady)' "${recovery_dry_log}" | head -n 1 | cut -d: -f1)
  peer_seed_guard_line=$(grep -n 'require absent rauthy-restore-once and revalidate rauthy-0 Ready before peer scale-up' "${recovery_dry_log}" | head -n 1 | cut -d: -f1)
  steady_guard_line=$(grep -n 'ensure active lifecycle modes (grant=restore, rauthy=steady) at rauthy-ordinal0-restored' "${recovery_dry_log}" | head -n 1 | cut -d: -f1)
  peer_scale_line=$(grep -n 'scale statefulset/rauthy --replicas=3' "${recovery_dry_log}" | head -n 1 | cut -d: -f1)
  if ! { [ -n "${secret_delete_line}" ] && [ -n "${steady_checkpoint_line}" ] \
    && [ -n "${peer_seed_guard_line}" ] && [ -n "${steady_guard_line}" ] && [ -n "${peer_scale_line}" ] \
    && [ "${secret_delete_line}" -lt "${steady_checkpoint_line}" ] \
    && [ "${steady_checkpoint_line}" -le "${peer_seed_guard_line}" ] \
    && [ "${peer_seed_guard_line}" -lt "${steady_guard_line}" ] \
    && [ "${steady_guard_line}" -lt "${peer_scale_line}" ]; }; then
    fail "Rauthy peers can scale before the one-shot Secret is consumed and lifecycle is steady"
  fi
  if grep -q 'checkpoint rauthy-ordinal0-restored (grant=restore, rauthy=restore)' "${recovery_dry_log}"; then
    fail "Rauthy ordinal 0 checkpoint leaves fresh peers in unauthorized restore mode"
  fi
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

assert_ci_local_placeholder_guard() {
  placeholder_log="${TMP_DIR}/ci-local-placeholder.log"
  if RAUTHY_IMAGE='ghcr.io/conalog/grant-rauthy-launcher:rauthy-0.36.0@sha256:RAUTHY_LAUNCHER_IMAGE_DIGEST' \
    bash "${ROOT_DIR}/scripts/ci-local.sh" >"${placeholder_log}" 2>&1; then
    fail "ci-local accepted the unpublished Rauthy launcher placeholder"
  fi
  grep -q 'refusing local bootstrap before image pull' "${placeholder_log}" \
    || fail "ci-local placeholder failure is not explicit or early"
  grep -q 'Override RAUTHY_IMAGE with a published immutable image' "${placeholder_log}" \
    || fail "ci-local placeholder failure does not explain the override"
  if grep -q '\[phase\]' "${placeholder_log}"; then
    fail "ci-local started a build or pull phase before rejecting the placeholder"
  fi
}

assert_ci_local_registry_auth_contract() {
  mock_bin="${TMP_DIR}/ci-local-mock-bin"
  mock_log="${TMP_DIR}/ci-local-mock-commands.log"
  run_log="${TMP_DIR}/ci-local-registry-path.log"
  mkdir -p "${mock_bin}"

  cat >"${mock_bin}/docker" <<'EOF'
#!/bin/sh
set -eu
printf 'docker %s\n' "$*" >>"${CI_LOCAL_MOCK_LOG}"
EOF

  cat >"${mock_bin}/sh" <<'EOF'
#!/bin/sh
set -eu
printf 'sh %s\n' "$*" >>"${CI_LOCAL_MOCK_LOG}"
EOF

  cat >"${mock_bin}/bash" <<'EOF'
#!/bin/sh
set -eu
printf 'bash %s\n' "$*" >>"${CI_LOCAL_MOCK_LOG}"
EOF

  cat >"${mock_bin}/noop" <<'EOF'
#!/bin/sh
exit 0
EOF
  chmod +x "${mock_bin}/docker" "${mock_bin}/sh" "${mock_bin}/bash" "${mock_bin}/noop"
  for command_name in kubectl curl jq vcluster; do
    cp "${mock_bin}/noop" "${mock_bin}/${command_name}"
  done

  for public_image in \
    docker.io/library/busybox:1.37 \
    registry.example.test/custom/rauthy@sha256:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb \
    ghcr.io/conalog/grant-rauthy-launcher-evil:tag; do
    : >"${mock_log}"
    PATH="${mock_bin}:${PATH}" CI_LOCAL_MOCK_LOG="${mock_log}" \
      RAUTHY_IMAGE="${public_image}" GHCR_USERNAME='' GHCR_TOKEN='' KEEP_CLUSTER=1 \
      /bin/bash "${ROOT_DIR}/scripts/ci-local.sh" >"${run_log}" 2>&1
    grep -Fqx "docker pull ${public_image}" "${mock_log}" \
      || fail "ci-local did not use ambient Docker auth for public/custom Rauthy image ${public_image}"
    if grep -Eq 'docker .* login |docker --context .* --config .* pull' "${mock_log}"; then
      fail "ci-local used private launcher auth state for public/custom Rauthy image ${public_image}"
    fi
    grep -q '^bash .*vind-bootstrap.sh' "${mock_log}" \
      || fail "ci-local public/custom image path did not reach cluster bootstrap"
  done

  for private_image in \
    ghcr.io/conalog/grant-rauthy-launcher:rauthy-0.36.0@sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa \
    ghcr.io/conalog/grant-rauthy-launcher:rauthy-0.36.0 \
    ghcr.io/conalog/grant-rauthy-launcher@sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa \
    ghcr.io/conalog/grant-rauthy-launcher; do
    : >"${mock_log}"
    set +e
    PATH="${mock_bin}:${PATH}" CI_LOCAL_MOCK_LOG="${mock_log}" \
      RAUTHY_IMAGE="${private_image}" GHCR_USERNAME='' GHCR_TOKEN='' KEEP_CLUSTER=1 \
      /bin/bash "${ROOT_DIR}/scripts/ci-local.sh" >"${run_log}" 2>&1
    private_status=$?
    set -e
    [ "${private_status}" -ne 0 ] \
      || fail "ci-local accepted private Rauthy launcher ${private_image} without GHCR credentials"
    grep -q 'GHCR_USERNAME is required for the private Rauthy launcher image' "${run_log}" \
      || fail "ci-local private launcher credential failure is not explicit for ${private_image}"
    if grep -Fq "docker pull ${private_image}" "${mock_log}" \
      || grep -q '^bash .*vind-bootstrap.sh' "${mock_log}"; then
      fail "ci-local continued after private launcher authentication failed for ${private_image}"
    fi
  done
}

assert_gke_skaffold_registry_secret_contract() {
  mock_bin="${TMP_DIR}/gke-mock-bin"
  mock_log="${TMP_DIR}/gke-mock-commands.log"
  missing_log="${TMP_DIR}/gke-missing-registry-credential.log"
  success_log="${TMP_DIR}/gke-registry-secret.log"
  config_path_file="${TMP_DIR}/gke-registry-config-path"
  fixture_root="${TMP_DIR}/gke-published-fixture"
  fixture_script="${fixture_root}/scripts/gke-skaffold-run.sh"
  placeholder_root="${TMP_DIR}/gke-placeholder-fixture"
  placeholder_script="${placeholder_root}/scripts/gke-skaffold-run.sh"
  fixture_image='ghcr.io/conalog/grant-rauthy-launcher:rauthy-0.36.0@sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa'
  placeholder_image='ghcr.io/conalog/grant-rauthy-launcher:rauthy-0.36.0@sha256:RAUTHY_LAUNCHER_IMAGE_DIGEST'
  mkdir -p "${mock_bin}" \
    "${fixture_root}/scripts" "${fixture_root}/deploy/k8s" \
    "${placeholder_root}/scripts" "${placeholder_root}/deploy/k8s"
  cp "${ROOT_DIR}/scripts/gke-skaffold-run.sh" "${fixture_script}"
  cp "${ROOT_DIR}/scripts/k8s-secret-helpers.sh" "${fixture_root}/scripts/k8s-secret-helpers.sh"
  sed "s|ghcr.io/conalog/grant-rauthy-launcher:[^[:space:]]*|${fixture_image}|" \
    "${ROOT_DIR}/deploy/k8s/rauthy-statefulset.yaml" >"${fixture_root}/deploy/k8s/rauthy-statefulset.yaml"
  cp "${ROOT_DIR}/scripts/gke-skaffold-run.sh" "${placeholder_script}"
  sed "s|ghcr.io/conalog/grant-rauthy-launcher:[^[:space:]]*|${placeholder_image}|" \
    "${ROOT_DIR}/deploy/k8s/rauthy-statefulset.yaml" >"${placeholder_root}/deploy/k8s/rauthy-statefulset.yaml"

  cat >"${mock_bin}/docker" <<'EOF'
#!/bin/sh
set -eu
printf 'docker %s\n' "$*" >>"${GKE_MOCK_LOG}"

if [ "${1:-}" = context ] && [ "${2:-}" = show ]; then
  echo mock-docker-context
  exit 0
fi

config_dir=
previous=
is_login=0
for argument do
  if [ "${previous}" = --config ]; then config_dir=${argument}; fi
  if [ "${argument}" = login ]; then is_login=1; fi
  previous=${argument}
done
if [ "${is_login}" = 1 ]; then
  IFS= read -r supplied_token || true
  [ "${supplied_token}" = "${GKE_EXPECTED_TOKEN}" ] || exit 91
  [ -n "${config_dir}" ] || exit 92
  mkdir -p "${config_dir}"
  printf '%s\n' '{"auths":{"ghcr.io":{"auth":"mock-only"}}}' >"${config_dir}/config.json"
  if [ -n "${GKE_MOCK_CONFIG_PATH_FILE:-}" ]; then
    printf '%s\n' "${config_dir}" >"${GKE_MOCK_CONFIG_PATH_FILE}"
  fi
fi
EOF

  cat >"${mock_bin}/kubectl" <<'EOF'
#!/bin/sh
set -eu
if [ "${GHCR_TOKEN+x}" = x ] || [ "${GHCR_USERNAME+x}" = x ]; then
  echo 'raw GHCR credentials reached kubectl' >&2
  exit 94
fi
printf 'kubectl %s\n' "$*" >>"${GKE_MOCK_LOG}"

case " $* " in
  *' apply -f - '*) cat >/dev/null ;;
  *' create namespace '*) printf '%s\n' 'apiVersion: v1' 'kind: Namespace' ;;
  *' create secret generic rauthy-launcher-pull '*)
    config_path=
    previous=
    for argument do
      case "${argument}" in
        --from-file=.dockerconfigjson=*) config_path=${argument#--from-file=.dockerconfigjson=} ;;
      esac
      if [ "${previous}" = --from-file ]; then
        case "${argument}" in .dockerconfigjson=*) config_path=${argument#*=} ;; esac
      fi
      previous=${argument}
    done
    [ -s "${config_path}" ] || exit 93
    printf '%s\n' 'apiVersion: v1' 'kind: Secret'
    ;;
  *' create secret generic grant-secrets '*)
    if [ "${RAUTHY_REGISTRY_CONFIG_DIR+x}" = x ]; then
      echo 'temporary registry config reached grant Secret creation' >&2
      exit 96
    fi
    printf '%s\n' 'apiVersion: v1' 'kind: Secret'
    ;;
esac
EOF

  cat >"${mock_bin}/skaffold" <<'EOF'
#!/bin/sh
set -eu
if [ "${GHCR_TOKEN+x}" = x ] || [ "${GHCR_USERNAME+x}" = x ]; then
  echo 'raw GHCR credentials reached skaffold' >&2
  exit 95
fi
if [ "${RAUTHY_REGISTRY_CONFIG_DIR+x}" = x ]; then
  echo 'temporary registry config path reached skaffold' >&2
  exit 96
fi
printf 'skaffold %s\n' "$*" >>"${GKE_MOCK_LOG}"
if [ "${GKE_MOCK_LONG_RUNNING:-0}" = 1 ]; then
  : "${GKE_MOCK_CHILD_PID_FILE:?}"
  : "${GKE_MOCK_CHILD_SIGNAL_FILE:?}"
  printf '%s\n' "$$" >"${GKE_MOCK_CHILD_PID_FILE}"
  if [ "${GKE_MOCK_IGNORE_SIGNALS:-0}" = 1 ]; then
    trap '' HUP INT TERM
  else
    trap 'printf "%s\n" HUP >"${GKE_MOCK_CHILD_SIGNAL_FILE}"; exit 129' HUP
    trap 'printf "%s\n" INT >"${GKE_MOCK_CHILD_SIGNAL_FILE}"; exit 130' INT
    trap 'printf "%s\n" TERM >"${GKE_MOCK_CHILD_SIGNAL_FILE}"; exit 143' TERM
  fi
  while :; do sleep 1; done
fi
exit "${GKE_MOCK_SKAFFOLD_STATUS:-0}"
EOF
  chmod +x "${mock_bin}/docker" "${mock_bin}/kubectl" "${mock_bin}/skaffold"

  : >"${mock_log}"
  set +e
  PATH="${mock_bin}:${PATH}" GKE_MOCK_LOG="${mock_log}" \
    NAMESPACE=grant-chaos KUBE_CONTEXT=gke_ied-cluster \
    SKAFFOLD_PROFILE=ied SKAFFOLD_MODULE=grant SKAFFOLD_EXTRA_ARGS='' RAUTHY_IMAGE='' \
    GHCR_USERNAME=ci-user GHCR_TOKEN=unused \
    bash "${placeholder_script}" >"${missing_log}" 2>&1
  placeholder_status=$?
  set -e
  [ "${placeholder_status}" -ne 0 ] \
    || fail "GKE Skaffold accepted the unpublished Rauthy launcher marker"
  grep -q 'digest is unpublished; refusing GKE deployment before Docker or cluster access' "${missing_log}" \
    || fail "GKE Skaffold placeholder failure is not explicit"
  [ ! -s "${mock_log}" ] \
    || fail "GKE Skaffold touched Docker or the cluster before rejecting the launcher placeholder"

  : >"${mock_log}"
  set +e
  PATH="${mock_bin}:${PATH}" GKE_MOCK_LOG="${mock_log}" \
    NAMESPACE=grant-chaos KUBE_CONTEXT=gke_ied-cluster \
    SKAFFOLD_PROFILE=ied SKAFFOLD_MODULE=grant SKAFFOLD_EXTRA_ARGS='' RAUTHY_IMAGE='' \
    GHCR_USERNAME=ci-user GHCR_TOKEN='' \
    bash "${fixture_script}" >"${missing_log}" 2>&1
  missing_status=$?
  set -e
  if [ "${missing_status}" -eq 0 ]; then
    fail "GKE Skaffold provisioning accepted missing private launcher credentials"
  fi
  if ! grep -q 'GHCR_TOKEN with read:packages is required' "${missing_log}"; then
    sed -n '1,20p' "${missing_log}" >&2
    fail "GKE Skaffold missing-credential failure is not explicit"
  fi
  [ ! -s "${mock_log}" ] \
    || fail "GKE Skaffold touched Docker or the cluster before rejecting missing credentials"

  : >"${mock_log}"
  set +e
  PATH="${mock_bin}:${PATH}" GKE_MOCK_LOG="${mock_log}" \
    NAMESPACE=grant-chaos KUBE_CONTEXT=gke_ied-cluster \
    SKAFFOLD_PROFILE=ied SKAFFOLD_MODULE=grant SKAFFOLD_EXTRA_ARGS='' \
    RAUTHY_IMAGE=docker.io/library/busybox:latest GHCR_USERNAME=ci-user GHCR_TOKEN=unused \
    bash "${fixture_script}" >"${missing_log}" 2>&1
  override_status=$?
  set -e
  [ "${override_status}" -ne 0 ] \
    || fail "GKE Skaffold allowed the private manifest image to be bypassed"
  grep -q 'RAUTHY_IMAGE cannot override it' "${missing_log}" \
    || fail "GKE Skaffold image override failure is not explicit"
  [ ! -s "${mock_log}" ] \
    || fail "GKE Skaffold touched Docker or the cluster before rejecting an image override"

  registry_token='gke-private-registry-token-sentinel'
  : >"${mock_log}"
  rm -f "${config_path_file}"
  PATH="${mock_bin}:${PATH}" GKE_MOCK_LOG="${mock_log}" GKE_EXPECTED_TOKEN="${registry_token}" \
    NAMESPACE=grant-chaos KUBE_CONTEXT=gke_ied-cluster \
    SKAFFOLD_PROFILE=ied SKAFFOLD_MODULE=grant SKAFFOLD_EXTRA_ARGS='' \
    RAUTHY_IMAGE='' \
    GKE_MOCK_CONFIG_PATH_FILE="${config_path_file}" GHCR_USERNAME=ci-user GHCR_TOKEN="${registry_token}" \
    bash "${fixture_script}" >"${success_log}" 2>&1

  grep -q 'docker .* login .* --password-stdin' "${mock_log}" \
    || fail "GKE Skaffold did not authenticate to GHCR through password stdin"
  pull_secret_line=$(grep -n 'kubectl .* create secret generic rauthy-launcher-pull .*--type=kubernetes.io/dockerconfigjson' "${mock_log}" \
    | head -n 1 | cut -d: -f1)
  skaffold_line=$(grep -n '^skaffold .*run' "${mock_log}" | head -n 1 | cut -d: -f1)
  if ! { [ -n "${pull_secret_line}" ] && [ -n "${skaffold_line}" ] \
    && [ "${pull_secret_line}" -lt "${skaffold_line}" ]; }; then
    sed -n '1,40p' "${mock_log}" >&2
    fail "GKE Skaffold did not create the private launcher pull Secret before deployment"
  fi
  if grep -Fq "${registry_token}" "${mock_log}" "${success_log}"; then
    fail "GKE Skaffold exposed the private registry token in argv or output"
  fi
  [ -s "${config_path_file}" ] \
    || fail "GKE Skaffold success mock did not record the temporary registry config"
  registry_config_dir=$(sed -n '1p' "${config_path_file}")
  [ ! -e "${registry_config_dir}" ] \
    || fail "GKE Skaffold did not clean the temporary registry config after success"

  : >"${mock_log}"
  rm -f "${config_path_file}"
  set +e
  PATH="${mock_bin}:${PATH}" GKE_MOCK_LOG="${mock_log}" GKE_EXPECTED_TOKEN="${registry_token}" \
    NAMESPACE=grant-chaos KUBE_CONTEXT=gke_ied-cluster \
    SKAFFOLD_PROFILE=ied SKAFFOLD_MODULE=grant SKAFFOLD_EXTRA_ARGS='' \
    RAUTHY_IMAGE='' \
    GKE_MOCK_CONFIG_PATH_FILE="${config_path_file}" GKE_MOCK_SKAFFOLD_STATUS=47 \
    GHCR_USERNAME=ci-user GHCR_TOKEN="${registry_token}" \
    bash "${fixture_script}" >"${success_log}" 2>&1
  failure_status=$?
  set -e
  [ "${failure_status}" -eq 47 ] \
    || fail "GKE Skaffold normal failure status was not preserved: ${failure_status}"
  registry_config_dir=$(sed -n '1p' "${config_path_file}")
  [ -n "${registry_config_dir}" ] && [ ! -e "${registry_config_dir}" ] \
    || fail "GKE Skaffold did not clean the temporary registry config after normal failure"

  for signal_contract in HUP:129:0 INT:130:0 TERM:143:0 TERM:143:1; do
    signal_name=${signal_contract%%:*}
    signal_details=${signal_contract#*:}
    expected_status=${signal_details%%:*}
    ignore_signals=${signal_details#*:}
    contract_label=${signal_name}
    if [ "${ignore_signals}" = 1 ]; then contract_label=${signal_name}-ignored; fi
    parent_pid_file="${TMP_DIR}/gke-runner-${contract_label}.pid"
    child_pid_file="${TMP_DIR}/gke-skaffold-${contract_label}.pid"
    child_signal_file="${TMP_DIR}/gke-skaffold-${contract_label}.signal"
    watchdog_result="${TMP_DIR}/gke-watchdog-${contract_label}.result"
    : >"${mock_log}"
    rm -f "${config_path_file}" "${parent_pid_file}" "${child_pid_file}" \
      "${child_signal_file}" "${watchdog_result}"

    (
      set +e
      terminate_watchdog_processes() {
        runner_to_stop=${1:-}
        child_to_stop=${2:-}
        [ -z "${runner_to_stop}" ] || kill -TERM "${runner_to_stop}" 2>/dev/null
        [ -z "${child_to_stop}" ] || kill -TERM "${child_to_stop}" 2>/dev/null
        cleanup_attempts=0
        while [ "${cleanup_attempts}" -lt 10 ]; do
          runner_alive=0
          child_alive=0
          [ -z "${runner_to_stop}" ] || kill -0 "${runner_to_stop}" 2>/dev/null || runner_alive=$?
          [ -z "${child_to_stop}" ] || kill -0 "${child_to_stop}" 2>/dev/null || child_alive=$?
          if { [ -z "${runner_to_stop}" ] || [ "${runner_alive}" -ne 0 ]; } \
            && { [ -z "${child_to_stop}" ] || [ "${child_alive}" -ne 0 ]; }; then
            return 0
          fi
          cleanup_attempts=$((cleanup_attempts + 1))
          sleep 0.1
        done
        [ -z "${runner_to_stop}" ] || kill -KILL "${runner_to_stop}" 2>/dev/null
        [ -z "${child_to_stop}" ] || kill -KILL "${child_to_stop}" 2>/dev/null
      }

      watchdog_attempts=0
      while { [ ! -s "${parent_pid_file}" ] || [ ! -s "${child_pid_file}" ]; }; do
        watchdog_attempts=$((watchdog_attempts + 1))
        if [ "${watchdog_attempts}" -ge 100 ]; then
          runner_pid=$(sed -n '1p' "${parent_pid_file}" 2>/dev/null)
          skaffold_pid=$(sed -n '1p' "${child_pid_file}" 2>/dev/null)
          terminate_watchdog_processes "${runner_pid}" "${skaffold_pid}"
          printf '%s\n' start-timeout >"${watchdog_result}"
          exit 1
        fi
        sleep 0.1
      done
      runner_pid=$(sed -n '1p' "${parent_pid_file}")
      skaffold_pid=$(sed -n '1p' "${child_pid_file}")
      registry_config_dir=$(sed -n '1p' "${config_path_file}")
      if [ -z "${registry_config_dir}" ] || [ -e "${registry_config_dir}" ]; then
        terminate_watchdog_processes "${runner_pid}" "${skaffold_pid}"
        printf '%s\n' registry-config-retained >"${watchdog_result}"
        exit 1
      fi

      # Only the tracked GKE runner receives the test signal. Its handler must
      # forward the same signal to the long-running Skaffold child.
      kill -s "${signal_name}" "${runner_pid}"
      watchdog_attempts=0
      while kill -0 "${runner_pid}" 2>/dev/null; do
        watchdog_attempts=$((watchdog_attempts + 1))
        if [ "${watchdog_attempts}" -ge 100 ]; then
          terminate_watchdog_processes "${runner_pid}" "${skaffold_pid}"
          printf '%s\n' exit-timeout >"${watchdog_result}"
          exit 1
        fi
        sleep 0.1
      done
      printf '%s\n' exited >"${watchdog_result}"
    ) &
    watchdog_pid=$!

    set +e
    PATH="${mock_bin}:${PATH}" GKE_MOCK_LOG="${mock_log}" GKE_EXPECTED_TOKEN="${registry_token}" \
      NAMESPACE=grant-chaos KUBE_CONTEXT=gke_ied-cluster \
      SKAFFOLD_PROFILE=ied SKAFFOLD_MODULE=grant SKAFFOLD_EXTRA_ARGS='' \
      RAUTHY_IMAGE='' \
      GKE_MOCK_CONFIG_PATH_FILE="${config_path_file}" GKE_MOCK_LONG_RUNNING=1 \
      GKE_MOCK_CHILD_PID_FILE="${child_pid_file}" GKE_MOCK_CHILD_SIGNAL_FILE="${child_signal_file}" \
      GKE_MOCK_IGNORE_SIGNALS="${ignore_signals}" \
      GHCR_USERNAME=ci-user GHCR_TOKEN="${registry_token}" \
      bash -c 'set -m; bash "$2" & runner_pid=$!; printf "%s\n" "${runner_pid}" >"$1"; set +m; wait "${runner_pid}"; exit "$?"' \
        gke-signal-runner "${parent_pid_file}" "${fixture_script}" >"${success_log}" 2>&1
    signal_status=$?
    wait "${watchdog_pid}"
    watchdog_status=$?
    set -e
    if [ "${watchdog_status}" -ne 0 ] || ! grep -qx exited "${watchdog_result}"; then
      fail "GKE Skaffold ${signal_name} watchdog failed: $(sed -n '1p' "${watchdog_result}" 2>/dev/null)"
    fi
    [ "${signal_status}" -eq "${expected_status}" ] \
      || fail "GKE Skaffold ${contract_label} status was ${signal_status}, expected ${expected_status}"
    skaffold_pid=$(sed -n '1p' "${child_pid_file}")
    if [ "${ignore_signals}" = 0 ]; then
      grep -qx "${signal_name}" "${child_signal_file}" \
        || fail "GKE Skaffold did not forward ${signal_name} to its tracked child"
    elif [ -s "${child_signal_file}" ]; then
      fail "GKE Skaffold non-cooperating child unexpectedly handled ${signal_name}"
    fi
    if kill -0 "${skaffold_pid}" 2>/dev/null; then
      fail "GKE Skaffold ${contract_label} handler left child ${skaffold_pid} running"
    fi
  done
}

verify_static() {
  need bash
  need kubectl
  need grep
  need jq
  need openssl
  TMP_DIR=$(mktemp -d "${TMPDIR:-/tmp}/grant-kustomize.XXXXXX")

  kubectl kustomize "${ROOT_DIR}/deploy/k8s" >"${TMP_DIR}/base.yaml"
  kubectl kustomize "${ROOT_DIR}/deploy/overlays/ephemeral" >"${TMP_DIR}/ephemeral.yaml"
  kubectl kustomize "${ROOT_DIR}/deploy/overlays/ied" >"${TMP_DIR}/ied.yaml"
  kubectl kustomize "${ROOT_DIR}/deploy/overlays/production" >"${TMP_DIR}/production.yaml"

  assert_repository_rauthy_launcher_pin "${ROOT_DIR}/deploy/k8s/rauthy-statefulset.yaml" \
    || fail "repository Rauthy launcher image does not match the verified production pin"
  placeholder_pin_fixture=${TMP_DIR}/rauthy-placeholder-pin.yaml
  printf '%s\n' \
    'image: ghcr.io/conalog/grant-rauthy-launcher:rauthy-0.36.0@sha256:RAUTHY_LAUNCHER_IMAGE_DIGEST' \
    >"${placeholder_pin_fixture}"
  if (ALLOW_UNPUBLISHED_RAUTHY_LAUNCHER=1; export ALLOW_UNPUBLISHED_RAUTHY_LAUNCHER; \
      assert_repository_rauthy_launcher_pin "${placeholder_pin_fixture}"); then
    fail "legacy unpublished-launcher environment bypassed the repository production pin"
  fi
  parser_empty_fixture=${TMP_DIR}/rauthy-parser-empty.yaml
  printf '%s\n' 'image: docker.io/library/busybox:1.37' >"${parser_empty_fixture}"
  if read_rauthy_launcher_image "${parser_empty_fixture}" >/dev/null 2>&1; then
    fail "Rauthy launcher image parser accepted a manifest without the launcher repository"
  fi

  grep -q 'RAUTHY_BASE_IMAGE=ghcr.io/sebadob/rauthy:0.36.0@sha256:e2a670c79e04ddf5947fbd6cd5dc2ec7115bb9f21132ddf03129d7b66d26f11d' \
    "${ROOT_DIR}/deploy/rauthy-launcher/Dockerfile" \
    || fail "Rauthy launcher does not derive from the pinned official 0.36.0 image"
  grep -q 'ENTRYPOINT \["/usr/local/bin/rauthy-launcher"\]' "${ROOT_DIR}/deploy/rauthy-launcher/Dockerfile" \
    || fail "Rauthy launcher image entrypoint contract is missing"
  grep -q 'CMD \["/app/rauthy", "serve"\]' "${ROOT_DIR}/deploy/rauthy-launcher/Dockerfile" \
    || fail "Rauthy launcher image does not preserve the official serve command"
  if ! grep -q 'provenance: mode=max,version=v1' "${ROOT_DIR}/.github/workflows/rauthy-launcher-image.yml" \
    || ! grep -q 'sbom: generator=docker/buildkit-syft-scanner:stable-1@sha256:79e7b013cbec16bbb436f312819a49a4a57752b2270c1a9332ae1a10fcc82a68' "${ROOT_DIR}/.github/workflows/rauthy-launcher-image.yml" \
    || [ "$(grep -c 'driver-opts: image=moby/buildkit:v0.30.0@sha256:0168606be2315b7c807a03b3d8aa79beefdb31c98740cebdffdfeebf31190c9f' "${ROOT_DIR}/.github/workflows/rauthy-launcher-image.yml" || :)" -ne 2 ] \
    || ! grep -q 'EXPECTED_SOURCE_SHA:' "${ROOT_DIR}/.github/workflows/rauthy-launcher-image.yml" \
    || ! grep -q 'EXPECTED_BUILDER_ID:' "${ROOT_DIR}/.github/workflows/rauthy-launcher-image.yml" \
    || ! grep -q "EXPECTED_BUILDER_MODE:.*prior-attempt.*current" "${ROOT_DIR}/.github/workflows/rauthy-launcher-image.yml" \
    || ! grep -Fq "EXPECTED_DIGEST: \${{ steps.digest.outputs.digest }}" "${ROOT_DIR}/.github/workflows/rauthy-launcher-image.yml" \
    || ! grep -q "if: steps.preflight.outputs.tag_exists == 'false'" "${ROOT_DIR}/.github/workflows/rauthy-launcher-image.yml" \
    || ! grep -q "if: steps.preflight.outputs.tag_exists == 'true'" "${ROOT_DIR}/.github/workflows/rauthy-launcher-image.yml" \
    || ! grep -Fq "test \"\${GITHUB_RUN_ATTEMPT}\" -gt 1" "${ROOT_DIR}/.github/workflows/rauthy-launcher-image.yml" \
    || ! grep -q 'run: sh scripts/verify-oci-attestations.sh' "${ROOT_DIR}/.github/workflows/rauthy-launcher-image.yml" \
    || ! grep -q 'fetch_blob' "${ROOT_DIR}/scripts/verify-oci-attestations.sh" \
    || ! grep -q 'https://in-toto.io/Statement/v0.1' "${ROOT_DIR}/scripts/verify-oci-attestations.sh"; then
    fail "Rauthy launcher publish does not verify portable OCI provenance and SBOM attestations"
  fi
  if grep -Eq 'attest-build-provenance|attestations:[[:space:]]*write|id-token:[[:space:]]*write' \
    "${ROOT_DIR}/.github/workflows/rauthy-launcher-image.yml"; then
    fail "Rauthy launcher publish still depends on unsupported GitHub repository attestations"
  fi
  grep -q 'ca-certificates curl libcurl4 libssl3' "${ROOT_DIR}/Dockerfile" \
    || fail "Grant runtime image is missing the restore readiness HTTP probe"
  grep -q 'REQUIRED_BASE64_DOCKER_CONFIG_JSON' "${ROOT_DIR}/deploy/overlays/production/rauthy-launcher-pull-secret.example.yaml" \
    || fail "production private registry pull Secret example is missing"
  grep -q -- '--password-stdin' "${ROOT_DIR}/scripts/k8s-secret-helpers.sh" \
    || fail "private registry login must use password stdin"
  grep -Fq "RAUTHY_DOCKER_CONTEXT=\$(docker context show)" "${ROOT_DIR}/scripts/k8s-secret-helpers.sh" \
    || fail "private registry auth does not preserve the active Docker context"
  grep -Fq -- "docker --context \"\${RAUTHY_DOCKER_CONTEXT}\" --config \"\${RAUTHY_REGISTRY_CONFIG_DIR}\"" \
    "${ROOT_DIR}/scripts/k8s-secret-helpers.sh" \
    || fail "private registry login does not bind the captured Docker context"
  if grep -q 'export DOCKER_CONFIG' "${ROOT_DIR}/scripts/k8s-secret-helpers.sh"; then
    fail "private registry auth globally overrides DOCKER_CONFIG"
  fi
  if grep -Eq -- '--password([ =]|$)|--from-literal=.*GHCR' "${ROOT_DIR}/scripts/k8s-secret-helpers.sh"; then
    fail "private registry credential is exposed through child argv"
  fi
  if grep -Fq 'grant\.cypark\.dev/' "${ROOT_DIR}/scripts/restore-full-cluster.sh" \
    || grep -Fq 'grant\.cypark\.dev/' "${ROOT_DIR}/scripts/create-recovery-pair.sh"; then
    fail "recovery scripts use escaped-dot JSONPath for slash-containing metadata keys"
  fi
  if grep -q 'command: \["/bin/sh", "/gate/restore-once.sh"\]' "${ROOT_DIR}/deploy/k8s/rauthy-statefulset.yaml"; then
    fail "Rauthy cannot use the shell restore wrapper from its distroless image"
  fi
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
  grep -Fq "image: ${EXPECTED_RAUTHY_LAUNCHER_IMAGE}" "${TMP_DIR}/production.yaml" \
    || fail "production Rauthy launcher image is not digest-pinned"
  if grep -Eq 'sha256:0{64}|grant\.example\.com|identity\.example\.com|kind: Ingress' "${TMP_DIR}/production.yaml"; then
    fail "production render contains an unresolved digest, example host, or bundled public ingress"
  fi
  grep -q 'name: GRANT_APP_AUTH_KEY' "${TMP_DIR}/production.yaml" \
    || fail "production Grant app-auth key wiring is missing"
  grep -q 'key: GRANT_APP_AUTH_KEY' "${TMP_DIR}/production.yaml" \
    || fail "production Grant app-auth Secret key wiring is missing"
  for restore_setting in RESTORE_ONCE_HEALTH_URL RESTORE_ONCE_READY_URL RESTORE_ONCE_READY_TIMEOUT_SECONDS \
    RESTORE_ONCE_READY_INTERVAL_SECONDS RESTORE_ONCE_READY_SUCCESSES RESTORE_ONCE_PROBE_TIMEOUT_SECONDS \
    RESTORE_ONCE_SIGNAL_GRACE_SECONDS; do
    grep -q "name: ${restore_setting}" "${TMP_DIR}/production.yaml" \
      || fail "production Grant restore supervisor is missing ${restore_setting}"
  done
  awk '
    $0 ~ /name: RESTORE_ONCE_READY_SUCCESSES$/ {
      getline
      if ($0 ~ /value: "3"$/) found = 1
    }
    END { exit found ? 0 : 1 }
  ' "${TMP_DIR}/production.yaml" \
    || fail "production Grant restore supervisor must require three consecutive readiness successes"
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
  assert_ci_local_placeholder_guard
  assert_ci_local_registry_auth_contract
  "${ROOT_DIR}/scripts/verify-vind-bootstrap.sh"
  assert_gke_skaffold_registry_secret_contract
  assert_recovery_role_resolution_policy
  "${ROOT_DIR}/scripts/verify-oci-attestations-tests.sh"
  "${ROOT_DIR}/scripts/verify-lifecycle-values.sh"
  "${ROOT_DIR}/scripts/verify-aws-principal-helpers.sh"
  "${ROOT_DIR}/scripts/verify-s3-object-history.sh"
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
