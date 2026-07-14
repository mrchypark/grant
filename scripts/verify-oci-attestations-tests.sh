#!/bin/sh
set -eu

ROOT_DIR=$(CDPATH='' cd -P "$(dirname "$0")/.." && pwd)
TMP_DIR=$(mktemp -d "${TMPDIR:-/tmp}/oci-attestation-test.XXXXXX")
REAL_CURL=$(command -v curl)
REDIRECT_FIXTURE_PID=
cleanup() {
  status=$?
  trap - 0 HUP INT TERM
  if [ -n "${REDIRECT_FIXTURE_PID}" ]; then
    kill -TERM "${REDIRECT_FIXTURE_PID}" 2>/dev/null || :
    wait "${REDIRECT_FIXTURE_PID}" 2>/dev/null || :
  fi
  rm -rf "${TMP_DIR}"
  exit "${status}"
}
trap cleanup 0
trap 'exit 129' HUP
trap 'exit 130' INT
trap 'exit 143' TERM

EXPECTED_REPOSITORY=ghcr.io/conalog/grant-rauthy-launcher
EXPECTED=sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa
AMD=sha256:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb
ARM=sha256:cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc
AMD_ATTEST=sha256:dddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddd
ARM_ATTEST=sha256:eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee
SOURCE_SHA=1111111111111111111111111111111111111111
SOURCE_REPOSITORY=https://github.com/Conalog/grant
export EXPECTED AMD ARM AMD_ATTEST ARM_ATTEST SOURCE_SHA SOURCE_REPOSITORY
mkdir -p "${TMP_DIR}/bin" "${TMP_DIR}/fixtures" "${TMP_DIR}/verifier-tmp"

digest_file() {
  file=$1
  printf 'sha256:%s\n' "$(openssl dgst -sha256 "${file}" | awk '{print $NF}')"
}

size_file() {
  wc -c <"$1" | tr -d ' '
}

write_provenance_fixture() {
  mode=$1
  platform=$2
  platform_digest=$3
  output=$4
  case "${mode}:${platform}" in
    empty-provenance:linux/amd64) printf '%s\n' null >"${output}"; return ;;
    malformed-provenance:linux/amd64|malformed-statement:linux/amd64) printf '%s\n' '{' >"${output}"; return ;;
  esac
  statement_type=https://in-toto.io/Statement/v0.1
  predicate_type=https://slsa.dev/provenance/v0.2
  subject_sha=${platform_digest#sha256:}
  subject_name="pkg:docker/${EXPECTED_REPOSITORY}@${EXPECTED_DIGEST:-${EXPECTED}}?platform=${platform}"
  builder_id=https://github.com/Conalog/grant/actions/runs/29366035201/attempts/1
  build_type=https://mobyproject.org/buildkit@v1
  label_sha=${SOURCE_SHA}
  label_source=${SOURCE_REPOSITORY}
  vcs_revision=${SOURCE_SHA}
  vcs_source=${SOURCE_REPOSITORY}
  materials='[{"uri":"pkg:docker/alpine@latest","digest":{"sha256":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"}}]'
  subject_mode=present
  case "${mode}:${platform}" in
    missing-subject:linux/amd64) subject_mode=missing ;;
    wrong-subject:linux/amd64) subject_sha=ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff ;;
    wrong-type:linux/amd64) statement_type=https://example.invalid/Statement ;;
    wrong-predicate-type:linux/amd64) predicate_type=https://example.invalid/Predicate ;;
    wrong-builder:linux/amd64|empty-builder:linux/amd64) builder_id=https://example.invalid/builder ;;
    wrong-source-sha:linux/amd64) label_sha=2222222222222222222222222222222222222222; vcs_revision=${label_sha} ;;
    wrong-source-repository:linux/amd64) label_source=https://github.com/example/wrong; vcs_source=${label_source} ;;
    wrong-vcs-revision:linux/amd64) vcs_revision=3333333333333333333333333333333333333333 ;;
    wrong-vcs-source:linux/amd64) vcs_source=https://github.com/example/wrong-vcs ;;
    wrong-platform:linux/amd64) platform=linux/s390x ;;
    wrong-build-type:linux/amd64) build_type=https://example.invalid/build ;;
    empty-materials:linux/amd64) materials='[]' ;;
  esac
  jq -n \
    --arg statementType "${statement_type}" \
    --arg predicateType "${predicate_type}" \
    --arg subjectMode "${subject_mode}" \
    --arg subjectName "${subject_name}" \
    --arg subjectSha "${subject_sha}" \
    --arg builderId "${builder_id}" \
    --arg buildType "${build_type}" \
    --argjson materials "${materials}" \
    --arg platform "${platform}" \
    --arg labelSha "${label_sha}" \
    --arg labelSource "${label_source}" \
    --arg vcsRevision "${vcs_revision}" \
    --arg vcsSource "${vcs_source}" '
    {
      _type: $statementType,
      predicateType: $predicateType,
      subject: (if $subjectMode == "missing" then [] else [{name: $subjectName, digest: {sha256: $subjectSha}}] end),
      predicate: {
        builder: {id: $builderId},
        buildType: $buildType,
        materials: $materials,
        invocation: {
          parameters: {
            args: {
              "label:org.opencontainers.image.revision": $labelSha,
              "label:org.opencontainers.image.source": $labelSource
            },
            root: {request: {args: {
              "vcs:revision": $vcsRevision,
              "vcs:source": $vcsSource
            }}}
          },
          environment: {platform: $platform}
        }
      }
    }' >"${output}"
}

write_spdx_fixture() {
  mode=$1
  platform=$2
  platform_digest=$3
  output=$4
  case "${mode}:${platform}" in
    empty-spdx:linux/amd64) printf '%s\n' null >"${output}"; return ;;
    malformed-spdx:linux/amd64) printf '%s\n' '{' >"${output}"; return ;;
  esac
  subject_sha=${platform_digest#sha256:}
  spdx_version=SPDX-2.3
  document_id=SPDXRef-DOCUMENT
  document_namespace=https://example.invalid/spdx/mock
  packages='[{"name":"rauthy","SPDXID":"SPDXRef-Package-rauthy"}]'
  if [ "${mode}:${platform}" = invalid-spdx:linux/amd64 ]; then
    spdx_version=invalid
    document_id=invalid
    document_namespace=
  fi
  [ "${mode}:${platform}" != empty-packages:linux/amd64 ] || packages='[]'
  jq -n \
    --arg subjectName "pkg:docker/${EXPECTED_REPOSITORY}@${EXPECTED}?platform=${platform}" \
    --arg subjectSha "${subject_sha}" \
    --arg spdxVersion "${spdx_version}" \
    --arg documentId "${document_id}" \
    --arg documentNamespace "${document_namespace}" \
    --argjson packages "${packages}" '
    {
      _type: "https://in-toto.io/Statement/v0.1",
      predicateType: "https://spdx.dev/Document",
      subject: [{name: $subjectName, digest: {sha256: $subjectSha}}],
      predicate: {
        spdxVersion: $spdxVersion,
        SPDXID: $documentId,
        documentNamespace: $documentNamespace,
        packages: $packages
      }
    }' >"${output}"
}

prepare_fixtures() {
  mode=$1
  fixture_dir=${TMP_DIR}/fixtures/${mode}
  rm -rf "${fixture_dir}"
  mkdir -p "${fixture_dir}"
  AMD_PROVENANCE_FILE=${fixture_dir}/amd-provenance.json
  AMD_SPDX_FILE=${fixture_dir}/amd-spdx.json
  ARM_PROVENANCE_FILE=${fixture_dir}/arm-provenance.json
  ARM_SPDX_FILE=${fixture_dir}/arm-spdx.json
  write_provenance_fixture "${mode}" linux/amd64 "${AMD}" "${AMD_PROVENANCE_FILE}"
  write_spdx_fixture "${mode}" linux/amd64 "${AMD}" "${AMD_SPDX_FILE}"
  write_provenance_fixture success linux/arm64 "${ARM}" "${ARM_PROVENANCE_FILE}"
  write_spdx_fixture success linux/arm64 "${ARM}" "${ARM_SPDX_FILE}"
  AMD_PROVENANCE_DIGEST=$(digest_file "${AMD_PROVENANCE_FILE}")
  AMD_SPDX_DIGEST=$(digest_file "${AMD_SPDX_FILE}")
  ARM_PROVENANCE_DIGEST=$(digest_file "${ARM_PROVENANCE_FILE}")
  ARM_SPDX_DIGEST=$(digest_file "${ARM_SPDX_FILE}")
  AMD_PROVENANCE_SIZE=$(size_file "${AMD_PROVENANCE_FILE}")
  AMD_SPDX_SIZE=$(size_file "${AMD_SPDX_FILE}")
  ARM_PROVENANCE_SIZE=$(size_file "${ARM_PROVENANCE_FILE}")
  ARM_SPDX_SIZE=$(size_file "${ARM_SPDX_FILE}")
  export AMD_PROVENANCE_FILE AMD_SPDX_FILE ARM_PROVENANCE_FILE ARM_SPDX_FILE
  export AMD_PROVENANCE_DIGEST AMD_SPDX_DIGEST ARM_PROVENANCE_DIGEST ARM_SPDX_DIGEST
  export AMD_PROVENANCE_SIZE AMD_SPDX_SIZE ARM_PROVENANCE_SIZE ARM_SPDX_SIZE
}

start_redirect_fixture() {
  redirect_ready=${TMP_DIR}/redirect.ready
  redirect_log=${TMP_DIR}/redirect.requests.log
  redirect_body=${TMP_DIR}/redirect.body
  printf '%s\n' 'digest-bound-blob-body' >"${redirect_body}"
  CODEX_GO_SCRIPT_CACHE=${TMP_DIR}/go-cache \
    "${ROOT_DIR}/scripts/oci-redirect-fixture" \
      --ready "${redirect_ready}" --log "${redirect_log}" --body "${redirect_body}" &
  REDIRECT_FIXTURE_PID=$!
  attempts=0
  while [ ! -s "${redirect_ready}" ]; do
    if ! kill -0 "${REDIRECT_FIXTURE_PID}" 2>/dev/null; then
      wait "${REDIRECT_FIXTURE_PID}" || :
      echo "HTTPS redirect fixture exited before becoming ready" >&2
      exit 1
    fi
    attempts=$((attempts + 1))
    [ "${attempts}" -lt 200 ] || { echo "timed out waiting for HTTPS redirect fixture" >&2; exit 1; }
    sleep 0.05
  done
  # shellcheck disable=SC1090 # generated by the local fixture with quoted values
  . "${redirect_ready}"
}

redirect_request_count() {
  request_origin=$1
  request_path=$2
  awk -v origin="${request_origin}" -v path="path=${request_path}" \
    '$1 == origin && $2 == path { count++ } END { print count + 0 }' "${redirect_log}"
}

verify_real_curl_redirect_contract() {
  redirect_config=${TMP_DIR}/redirect.curlrc
  redirect_output=${TMP_DIR}/redirect.output
  {
    printf 'silent\n'
    printf 'show-error\n'
    printf 'fail\n'
    printf 'cacert = "%s"\n' "${CA_CERT}"
    printf 'location\n'
    printf 'proto-redir = "=https"\n'
    printf 'max-redirs = 3\n'
    printf 'header = "Authorization: Bearer redirect-contract-secret"\n'
  } >"${redirect_config}"
  if grep -q '^location-trusted' "${redirect_config}"; then
    echo "redirect contract would forward credentials across origins" >&2
    exit 1
  fi

  "${REAL_CURL}" --config "${redirect_config}" --url "${ORIGIN_URL}/success" >"${redirect_output}"
  cmp "${redirect_body}" "${redirect_output}" >/dev/null \
    || { echo "real curl did not return the HTTPS redirect target body" >&2; exit 1; }
  grep -q '^origin path=/success authorization=true$' "${redirect_log}" \
    || { echo "redirect origin did not receive the registry bearer" >&2; exit 1; }
  grep -q '^target path=/blob authorization=false$' "${redirect_log}" \
    || { echo "real curl forwarded Authorization to the redirect target" >&2; exit 1; }

  case "${HTTP_TARGET_URL}" in http://*) ;; *) echo "fixture HTTP target is not plain HTTP" >&2; exit 1 ;; esac
  http_target_before=$(redirect_request_count http-target /blob)
  if "${REAL_CURL}" --config "${redirect_config}" --url "${ORIGIN_URL}/http" \
    >"${redirect_output}" 2>"${TMP_DIR}/redirect-http.log"; then
    echo "real curl followed a non-HTTPS blob redirect" >&2
    exit 1
  fi
  http_target_after=$(redirect_request_count http-target /blob)
  [ "${http_target_before}" -eq "${http_target_after}" ] \
    || { echo "HTTPS-only curlrc allowed a request to the HTTP target" >&2; exit 1; }

  redirect_negative_config=${TMP_DIR}/redirect-negative-control.curlrc
  grep -v '^proto-redir = ' "${redirect_config}" >"${redirect_negative_config}"
  "${REAL_CURL}" --config "${redirect_negative_config}" --url "${ORIGIN_URL}/http" >"${redirect_output}"
  cmp "${redirect_body}" "${redirect_output}" >/dev/null \
    || { echo "HTTP redirect negative control did not return the target body" >&2; exit 1; }
  http_target_control=$(redirect_request_count http-target /blob)
  [ $((http_target_control - http_target_after)) -eq 1 ] \
    || { echo "HTTP redirect negative control did not reach the target exactly once" >&2; exit 1; }
  grep -q '^http-target path=/blob authorization=false$' "${redirect_log}" \
    || { echo "HTTP redirect target unexpectedly received Authorization" >&2; exit 1; }

  loop_before=$(redirect_request_count origin /loop)
  if "${REAL_CURL}" --config "${redirect_config}" --url "${ORIGIN_URL}/loop" \
    >"${redirect_output}" 2>"${TMP_DIR}/redirect-loop.log"; then
    echo "real curl accepted an unbounded redirect loop" >&2
    exit 1
  fi
  loop_after=$(redirect_request_count origin /loop)
  [ $((loop_after - loop_before)) -eq 4 ] \
    || { echo "real curl did not enforce max-redirs=3" >&2; exit 1; }

  if "${REAL_CURL}" --config "${redirect_config}" --url "${ORIGIN_URL}/failure" \
    >"${redirect_output}" 2>"${TMP_DIR}/redirect-failure.log"; then
    echo "real curl accepted an unreachable HTTPS redirect target" >&2
    exit 1
  fi
  if grep -q 'redirect-contract-secret' "${redirect_log}"; then
    echo "HTTPS fixture logged the redirect credential" >&2
    exit 1
  fi
}

cat >"${TMP_DIR}/bin/docker" <<'EOF'
#!/bin/sh
set -eu
[ "${GHCR_TOKEN+x}" != x ] || { echo 'docker inherited GHCR_TOKEN' >&2; exit 92; }
[ "${GITHUB_TOKEN+x}" != x ] || { echo 'docker inherited GITHUB_TOKEN' >&2; exit 93; }
printf '%s\n' "$*" >>"${MOCK_DOCKER_LOG}"
[ "$1 $2 $3" = "buildx imagetools inspect" ] || exit 90
case "${MOCK_MODE}" in
  signal-hup) kill -HUP "${PPID}"; exit 0 ;;
  signal-int) kill -INT "${PPID}"; exit 0 ;;
  signal-term) kill -TERM "${PPID}"; exit 0 ;;
esac
if [ "${4-}" != --raw ]; then
  case "${MOCK_MODE}" in wrong-digest) printf 'Digest: sha256:ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff\n' ;;
    *) printf 'Digest: %s\n' "${EXPECTED}" ;;
  esac
  exit 0
fi
case "$5" in
  *@"${EXPECTED}")
    amd_digest=${AMD}
    amd_attestation_digest=${AMD_ATTEST}
    [ "${MOCK_MODE}" != uppercase-platform-digest ] \
      || amd_digest=sha256:$(printf '%s' "${AMD#sha256:}" | tr '[:lower:]' '[:upper:]')
    [ "${MOCK_MODE}" != uppercase-attestation-digest ] \
      || amd_attestation_digest=sha256:$(printf '%s' "${AMD_ATTEST#sha256:}" | tr '[:lower:]' '[:upper:]')
    if [ "${MOCK_MODE}" = missing-platform ]; then
      platforms=$(printf '{"mediaType":"application/vnd.oci.image.manifest.v1+json","digest":"%s","platform":{"os":"linux","architecture":"amd64"}}' "${amd_digest}")
    else
      platforms=$(printf '{"mediaType":"application/vnd.oci.image.manifest.v1+json","digest":"%s","platform":{"os":"linux","architecture":"amd64"}},{"mediaType":"application/vnd.oci.image.manifest.v1+json","digest":"%s","platform":{"os":"linux","architecture":"arm64"}}' "${amd_digest}" "${ARM}")
    fi
    amd_reference=${amd_digest}
    [ "${MOCK_MODE}" != missing-reference ] || amd_reference=sha256:ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff
    printf '{"schemaVersion":2,"mediaType":"application/vnd.oci.image.index.v1+json","manifests":[%s,{"mediaType":"application/vnd.oci.image.manifest.v1+json","digest":"%s","platform":{"os":"unknown","architecture":"unknown"},"annotations":{"vnd.docker.reference.type":"attestation-manifest","vnd.docker.reference.digest":"%s"}},{"mediaType":"application/vnd.oci.image.manifest.v1+json","digest":"%s","platform":{"os":"unknown","architecture":"unknown"},"annotations":{"vnd.docker.reference.type":"attestation-manifest","vnd.docker.reference.digest":"%s"}}]}\n' \
      "${platforms}" "${amd_attestation_digest}" "${amd_reference}" "${ARM_ATTEST}" "${ARM}"
    ;;
  *@"${AMD_ATTEST}")
    provenance_digest=${AMD_PROVENANCE_DIGEST}
    provenance_size=${AMD_PROVENANCE_SIZE}
    [ "${MOCK_MODE}" != uppercase-layer-digest ] \
      || provenance_digest=sha256:$(printf '%s' "${provenance_digest#sha256:}" | tr '[:lower:]' '[:upper:]')
    [ "${MOCK_MODE}" != wrong-blob-digest ] \
      || provenance_digest=sha256:ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff
    [ "${MOCK_MODE}" != wrong-layer-size ] || provenance_size=$((provenance_size + 1))
    if [ "${MOCK_MODE}" = missing-sbom ]; then
      printf '{"schemaVersion":2,"mediaType":"application/vnd.oci.image.manifest.v1+json","layers":[{"mediaType":"application/vnd.in-toto+json","digest":"%s","size":%s,"annotations":{"in-toto.io/predicate-type":"https://slsa.dev/provenance/v0.2"}}]}\n' "${provenance_digest}" "${provenance_size}"
    elif [ "${MOCK_MODE}" = missing-provenance ]; then
      printf '{"schemaVersion":2,"mediaType":"application/vnd.oci.image.manifest.v1+json","layers":[{"mediaType":"application/vnd.in-toto+json","digest":"%s","size":%s,"annotations":{"in-toto.io/predicate-type":"https://spdx.dev/Document"}}]}\n' "${AMD_SPDX_DIGEST}" "${AMD_SPDX_SIZE}"
    else
      printf '{"schemaVersion":2,"mediaType":"application/vnd.oci.image.manifest.v1+json","layers":[{"mediaType":"application/vnd.in-toto+json","digest":"%s","size":%s,"annotations":{"in-toto.io/predicate-type":"https://slsa.dev/provenance/v0.2"}},{"mediaType":"application/vnd.in-toto+json","digest":"%s","size":%s,"annotations":{"in-toto.io/predicate-type":"https://spdx.dev/Document"}}]}\n' \
        "${provenance_digest}" "${provenance_size}" "${AMD_SPDX_DIGEST}" "${AMD_SPDX_SIZE}"
    fi
    ;;
  *@"${ARM_ATTEST}")
    printf '{"schemaVersion":2,"mediaType":"application/vnd.oci.image.manifest.v1+json","layers":[{"mediaType":"application/vnd.in-toto+json","digest":"%s","size":%s,"annotations":{"in-toto.io/predicate-type":"https://slsa.dev/provenance/v0.2"}},{"mediaType":"application/vnd.in-toto+json","digest":"%s","size":%s,"annotations":{"in-toto.io/predicate-type":"https://spdx.dev/Document"}}]}\n' \
      "${ARM_PROVENANCE_DIGEST}" "${ARM_PROVENANCE_SIZE}" "${ARM_SPDX_DIGEST}" "${ARM_SPDX_SIZE}"
    ;;
  *) exit 91 ;;
esac
EOF

cat >"${TMP_DIR}/bin/curl" <<'EOF'
#!/bin/sh
set -eu
[ "${GHCR_TOKEN+x}" != x ] || { echo 'curl inherited GHCR_TOKEN' >&2; exit 96; }
[ "${GITHUB_TOKEN+x}" != x ] || { echo 'curl inherited GITHUB_TOKEN' >&2; exit 97; }
printf '%s\n' "$*" >>"${MOCK_CURL_LOG}"
case "${MOCK_MODE}" in
  signal-hup-curl) kill -HUP "${PPID}"; exit 0 ;;
  signal-int-curl) kill -INT "${PPID}"; exit 0 ;;
  signal-term-curl) kill -TERM "${PPID}"; exit 0 ;;
esac
[ "${1-}" = --config ] || exit 98
config=$2
if grep -q 'ghcr.io/token?' "${config}"; then
  grep -q '^user = ' "${config}" || exit 99
  printf '%s\n' '{"token":"mock-bearer-token"}'
  exit 0
fi
grep -q '^header = "Authorization: Bearer mock-bearer-token"$' "${config}" || exit 100
grep -q '^location$' "${config}" || exit 104
grep -q '^proto-redir = "=https"$' "${config}" || exit 105
grep -q '^max-redirs = 3$' "${config}" || exit 106
if grep -q '^location-trusted' "${config}"; then exit 107; fi
[ "${3-}" = --url ] || exit 101
digest=${4##*/}
case "${digest}" in
  "${AMD_PROVENANCE_DIGEST}") file=${AMD_PROVENANCE_FILE} ;;
  "${AMD_SPDX_DIGEST}") file=${AMD_SPDX_FILE} ;;
  "${ARM_PROVENANCE_DIGEST}") file=${ARM_PROVENANCE_FILE} ;;
  "${ARM_SPDX_DIGEST}") file=${ARM_SPDX_FILE} ;;
  sha256:ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff)
    [ "${MOCK_MODE}" = wrong-blob-digest ] || exit 102
    file=${AMD_PROVENANCE_FILE}
    ;;
  *) exit 103 ;;
esac
cat "${file}"
EOF
chmod 0700 "${TMP_DIR}/bin/docker" "${TMP_DIR}/bin/curl"

run_verifier() {
  requested_mode=$1
  prepare_fixtures "${requested_mode}"
  repository=${EXPECTED_REPOSITORY}
  [ "${requested_mode}" != wrong-repository ] || repository=ghcr.io/conalog/wrong
  MOCK_MODE=${requested_mode} \
    MOCK_DOCKER_LOG="${TMP_DIR}/${requested_mode}.docker.log" \
    MOCK_CURL_LOG="${TMP_DIR}/${requested_mode}.curl.log" \
    GHCR_USERNAME=ci-user GHCR_TOKEN=credential-must-not-leak \
    GITHUB_TOKEN=github-credential-must-not-leak \
    TMPDIR="${TMP_DIR}/verifier-tmp" PATH="${TMP_DIR}/bin:${PATH}" \
    IMAGE_REPOSITORY=${repository} IMAGE_REF=${repository}:source-tag \
    EXPECTED_DIGEST=${EXPECTED} EXPECTED_SOURCE_SHA=${SOURCE_SHA} \
    sh "${ROOT_DIR}/scripts/verify-oci-attestations.sh"
}

start_redirect_fixture
verify_real_curl_redirect_contract
run_verifier success >"${TMP_DIR}/success.log"
for rejected_mode in \
  wrong-repository wrong-digest missing-platform missing-reference missing-sbom missing-provenance \
  uppercase-platform-digest uppercase-attestation-digest uppercase-layer-digest wrong-layer-size \
  wrong-blob-digest missing-subject wrong-subject wrong-type wrong-predicate-type malformed-statement \
  empty-provenance malformed-provenance wrong-builder wrong-source-sha wrong-source-repository \
  wrong-vcs-revision wrong-vcs-source wrong-platform wrong-build-type empty-materials \
  empty-spdx malformed-spdx invalid-spdx empty-packages; do
  if run_verifier "${rejected_mode}" >"${TMP_DIR}/${rejected_mode}.log" 2>&1; then
    echo "invalid OCI attestation fixture was accepted: ${rejected_mode}" >&2
    exit 1
  fi
done
for signal_contract in \
  signal-hup:129 signal-int:130 signal-term:143 \
  signal-hup-curl:129 signal-int-curl:130 signal-term-curl:143; do
  signal_mode=${signal_contract%:*}
  expected_status=${signal_contract#*:}
  set +e
  run_verifier "${signal_mode}" >"${TMP_DIR}/${signal_mode}.log" 2>&1
  signal_status=$?
  set -e
  if [ "${signal_status}" -ne "${expected_status}" ]; then
    echo "verifier ${signal_mode} exit status ${signal_status}, expected ${expected_status}" >&2
    exit 1
  fi
done
if find "${TMP_DIR}/verifier-tmp" -mindepth 1 -maxdepth 1 -name 'oci-attestations.*' | grep -q .; then
  echo "registry verifier did not clean its temporary directory after a signal" >&2
  exit 1
fi
if grep -R 'credential-must-not-leak\|github-credential-must-not-leak' "${TMP_DIR}" >/dev/null 2>&1; then
  echo "registry verifier exposed a credential" >&2
  exit 1
fi

echo 'OCI digest-bound in-toto provenance and SPDX envelope verification contract passed'
