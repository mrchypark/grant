#!/bin/sh
set -eu

ROOT_DIR=$(CDPATH='' cd -P "$(dirname "$0")/.." && pwd)
TMP_DIR=$(mktemp -d "${TMPDIR:-/tmp}/oci-attestation-test.XXXXXX")
REAL_CURL=$(command -v curl)
REAL_MKTEMP=$(command -v mktemp)
CREDENTIAL_SCAN_MATCHES=${TMP_DIR}.credential-scan.matches
CREDENTIAL_SCAN_ERROR=${TMP_DIR}.credential-scan.error
REDIRECT_FIXTURE_PID=
cleanup() {
  status=$?
  trap - 0 HUP INT TERM
  if [ -n "${REDIRECT_FIXTURE_PID}" ]; then
    kill -TERM "${REDIRECT_FIXTURE_PID}" 2>/dev/null || :
    wait "${REDIRECT_FIXTURE_PID}" 2>/dev/null || :
  fi
  rm -rf "${TMP_DIR}"
  rm -f "${CREDENTIAL_SCAN_MATCHES}" "${CREDENTIAL_SCAN_ERROR}"
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
CURRENT_BUILDER_ID=https://github.com/Conalog/grant/actions/runs/29366035201/attempts/1
TEST_GHCR_USERNAME='username-must-not-leak"back\slash'
TEST_GHCR_TOKEN='credential-must-not-leak"back\slash'
TEST_AUTH_SHA=$(printf '%s:%s' "${TEST_GHCR_USERNAME}" "${TEST_GHCR_TOKEN}" | openssl dgst -sha256 | awk '{print $NF}')
TEST_REGISTRY_TOKEN='registry-token-must-not-leak"back\slash'
TEST_REGISTRY_TOKEN_SHA=$(printf '%s' "${TEST_REGISTRY_TOKEN}" | openssl dgst -sha256 | awk '{print $NF}')
export EXPECTED AMD ARM AMD_ATTEST ARM_ATTEST SOURCE_SHA SOURCE_REPOSITORY CURRENT_BUILDER_ID
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
  predicate_type=https://slsa.dev/provenance/v1
  subject_sha=${platform_digest#sha256:}
  subject_name="pkg:docker/${EXPECTED_REPOSITORY}@${EXPECTED_DIGEST:-${EXPECTED}}?platform=${platform}"
  builder_id=${CURRENT_BUILDER_ID}
  build_type=https://github.com/moby/buildkit/blob/master/docs/attestations/slsa-definitions.md
  label_sha=${SOURCE_SHA}
  label_source=${SOURCE_REPOSITORY}
  config_source_path=Dockerfile
  vcs_revision=${SOURCE_SHA}
  vcs_source=${SOURCE_REPOSITORY}
  vcs_context_dir=deploy/rauthy-launcher
  vcs_dockerfile_dir=deploy/rauthy-launcher
  resolved_dependencies='[{"uri":"pkg:docker/docker/buildkit-syft-scanner@stable-1","digest":{"sha256":"79e7b013cbec16bbb436f312819a49a4a57752b2270c1a9332ae1a10fcc82a68"}},{"uri":"pkg:docker/docker/dockerfile@1.7","digest":{"sha256":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"}}]'
  builder_platform=linux/amd64
  llb_definition='[{"id":"step0","op":{"Op":{}}}]'
  request_complete=true
  subject_mode=present
  case "${mode}:${platform}" in
    missing-subject:linux/amd64) subject_mode=missing ;;
    wrong-subject:linux/amd64) subject_sha=ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff ;;
    wrong-type:linux/amd64) statement_type=https://example.invalid/Statement ;;
    wrong-predicate-type:linux/amd64) predicate_type=https://example.invalid/Predicate ;;
    wrong-builder:linux/amd64|empty-builder:linux/amd64) builder_id=https://example.invalid/builder ;;
    prior-wrong-run:linux/amd64) builder_id=https://github.com/Conalog/grant/actions/runs/999/attempts/1 ;;
    prior-current-attempt:linux/amd64) builder_id=https://github.com/Conalog/grant/actions/runs/29366035201/attempts/2 ;;
    prior-future-attempt:linux/amd64) builder_id=https://github.com/Conalog/grant/actions/runs/29366035201/attempts/3 ;;
    prior-platform-disagreement:linux/amd64) builder_id=https://github.com/Conalog/grant/actions/runs/29366035201/attempts/2 ;;
    wrong-source-sha:linux/amd64) label_sha=2222222222222222222222222222222222222222; vcs_revision=${label_sha} ;;
    wrong-source-repository:linux/amd64) label_source=https://github.com/example/wrong; vcs_source=${label_source} ;;
    wrong-config-source:linux/amd64) config_source_path=Wrongfile ;;
    wrong-vcs-revision:linux/amd64) vcs_revision=3333333333333333333333333333333333333333 ;;
    wrong-vcs-source:linux/amd64) vcs_source=https://github.com/example/wrong-vcs ;;
    wrong-vcs-context-dir:linux/amd64) vcs_context_dir=. ;;
    wrong-vcs-dockerfile-dir:linux/amd64) vcs_dockerfile_dir=. ;;
    wrong-build-type:linux/amd64) build_type=https://example.invalid/build ;;
    empty-resolved-dependencies:linux/amd64) resolved_dependencies='[]' ;;
    wrong-sbom-generator-dependency:linux/amd64) resolved_dependencies='[{"uri":"pkg:docker/docker/dockerfile@1.7","digest":{"sha256":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"}}]' ;;
    wrong-builder-platform:linux/amd64) builder_platform=linux/arm64 ;;
    empty-llb-definition:linux/amd64) llb_definition='[]' ;;
    incomplete-request:linux/amd64) request_complete=false ;;
    provenance-v0.2:linux/amd64) predicate_type=https://slsa.dev/provenance/v0.2 ;;
  esac
  jq -n \
    --arg statementType "${statement_type}" \
    --arg predicateType "${predicate_type}" \
    --arg subjectMode "${subject_mode}" \
    --arg subjectName "${subject_name}" \
    --arg subjectSha "${subject_sha}" \
    --arg builderId "${builder_id}" \
    --arg builderPlatform "${builder_platform}" \
    --arg buildType "${build_type}" \
    --arg configSourcePath "${config_source_path}" \
    --argjson llbDefinition "${llb_definition}" \
    --argjson requestComplete "${request_complete}" \
    --argjson resolvedDependencies "${resolved_dependencies}" \
    --arg labelSha "${label_sha}" \
    --arg labelSource "${label_source}" \
    --arg vcsRevision "${vcs_revision}" \
    --arg vcsContextDir "${vcs_context_dir}" \
    --arg vcsDockerfileDir "${vcs_dockerfile_dir}" \
    --arg vcsSource "${vcs_source}" '
    {
      _type: $statementType,
      predicateType: $predicateType,
      subject: (if $subjectMode == "missing" then [] else [{name: $subjectName, digest: {sha256: $subjectSha}}] end),
      predicate: {
        buildDefinition: {
          buildType: $buildType,
          resolvedDependencies: $resolvedDependencies,
          internalParameters: {
            builderPlatform: $builderPlatform,
            buildConfig: {llbDefinition: $llbDefinition}
          },
          externalParameters: {
            configSource: {path: $configSourcePath},
            request: {
              args: {
                "label:org.opencontainers.image.revision": $labelSha,
                "label:org.opencontainers.image.source": $labelSource
              },
              root: {
                configSource: {path: $configSourcePath},
                request: {args: {
                  "vcs:localdir:context": $vcsContextDir,
                  "vcs:localdir:dockerfile": $vcsDockerfileDir,
                  "vcs:revision": $vcsRevision,
                  "vcs:source": $vcsSource
                }}
              }
            }
          }
        },
        runDetails: {
          builder: {id: $builderId},
          metadata: {
            invocationId: "fixture-invocation",
            startedOn: "2026-07-15T09:34:29Z",
            finishedOn: "2026-07-15T09:35:17Z",
            buildkit_completeness: {request: $requestComplete}
          }
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
  redirect_cache=${TMP_DIR}/go-cache
  printf '%s\n' 'digest-bound-blob-body' >"${redirect_body}"
  # Finish toolchain selection, compilation, and the atomic cache move before
  # starting the server. This keeps cold-cache build time out of readiness and
  # prevents cleanup from racing the launcher cache write.
  CODEX_GO_SCRIPT_CACHE=${redirect_cache} \
    "${ROOT_DIR}/scripts/oci-redirect-fixture" --help >/dev/null 2>&1
  CODEX_GO_SCRIPT_CACHE=${redirect_cache} \
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

  malicious_home=${TMP_DIR}/malicious-curl-home
  malicious_xdg=${TMP_DIR}/malicious-curl-xdg
  mkdir -p "${malicious_home}" "${malicious_xdg}"
  printf 'proxy = "http://127.0.0.1:1"\n' >"${malicious_home}/.curlrc"
  printf 'proxy = "http://127.0.0.1:1"\n' >"${malicious_xdg}/curlrc"
  if HOME=${malicious_home} XDG_CONFIG_HOME=${malicious_xdg} CURL_HOME=${malicious_home} NO_PROXY='' no_proxy='' \
    "${REAL_CURL}" --config "${redirect_config}" --url "${ORIGIN_URL}/success" \
      >"${redirect_output}" 2>"${TMP_DIR}/malicious-curlrc.log"; then
    echo "malicious default curlrc negative control unexpectedly succeeded" >&2
    exit 1
  fi
  HOME=${malicious_home} XDG_CONFIG_HOME=${malicious_xdg} CURL_HOME=${malicious_home} NO_PROXY='' no_proxy='' \
    "${REAL_CURL}" -q --config "${redirect_config}" --url "${ORIGIN_URL}/success" >"${redirect_output}"
  cmp "${redirect_body}" "${redirect_output}" >/dev/null \
    || { echo "curl -q did not ignore the malicious default curlrc" >&2; exit 1; }
}

scan_for_credentials() {
  scan_root=$1
  : >"${CREDENTIAL_SCAN_MATCHES}"
  : >"${CREDENTIAL_SCAN_ERROR}"
  find_status=0
  find "${scan_root}" -type f -exec sh -c '
    matches=$1
    error_marker=$2
    shift 2
    for scan_file do
      if grep -F -l \
        -e "username-must-not-leak" \
        -e "credential-must-not-leak" \
        -e "registry-token-must-not-leak" \
        -e "github-credential-must-not-leak" \
        "${scan_file}" >>"${matches}" 2>/dev/null; then
        :
      else
        grep_status=$?
        if [ "${grep_status}" -ne 1 ]; then
          printf "%s\n" grep-error >"${error_marker}"
          exit "${grep_status}"
        fi
      fi
    done
  ' credential-scan "${CREDENTIAL_SCAN_MATCHES}" "${CREDENTIAL_SCAN_ERROR}" {} + \
    || find_status=$?
  if [ "${find_status}" -ne 0 ] || [ -s "${CREDENTIAL_SCAN_ERROR}" ]; then
    return 2
  fi
  [ ! -s "${CREDENTIAL_SCAN_MATCHES}" ] || return 1
}

cat >"${TMP_DIR}/bin/mktemp" <<'EOF'
#!/bin/sh
set -eu
[ "${raw_username+x}" != x ] || { echo 'mktemp inherited raw_username' >&2; exit 110; }
[ "${raw_token+x}" != x ] || { echo 'mktemp inherited raw_token' >&2; exit 111; }
[ "${escaped_username+x}" != x ] || { echo 'mktemp inherited escaped_username' >&2; exit 112; }
[ "${escaped_token+x}" != x ] || { echo 'mktemp inherited escaped_token' >&2; exit 113; }
[ "${registry_token+x}" != x ] || { echo 'mktemp inherited registry_token' >&2; exit 114; }
[ "${registry_token_with_sentinel+x}" != x ] || { echo 'mktemp inherited registry_token_with_sentinel' >&2; exit 123; }
[ "${escaped_registry_token+x}" != x ] || { echo 'mktemp inherited escaped_registry_token' >&2; exit 115; }
exec "${REAL_MKTEMP}" "$@"
EOF

cat >"${TMP_DIR}/bin/docker" <<'EOF'
#!/bin/sh
set -eu
[ "${GHCR_TOKEN+x}" != x ] || { echo 'docker inherited GHCR_TOKEN' >&2; exit 92; }
[ "${GHCR_USERNAME+x}" != x ] || { echo 'docker inherited GHCR_USERNAME' >&2; exit 94; }
[ "${GITHUB_TOKEN+x}" != x ] || { echo 'docker inherited GITHUB_TOKEN' >&2; exit 93; }
[ "${raw_username+x}" != x ] || { echo 'docker inherited raw_username' >&2; exit 116; }
[ "${raw_token+x}" != x ] || { echo 'docker inherited raw_token' >&2; exit 117; }
[ "${escaped_username+x}" != x ] || { echo 'docker inherited escaped_username' >&2; exit 118; }
[ "${escaped_token+x}" != x ] || { echo 'docker inherited escaped_token' >&2; exit 119; }
[ "${registry_token+x}" != x ] || { echo 'docker inherited registry_token' >&2; exit 120; }
[ "${registry_token_with_sentinel+x}" != x ] || { echo 'docker inherited registry_token_with_sentinel' >&2; exit 123; }
[ "${escaped_registry_token+x}" != x ] || { echo 'docker inherited escaped_registry_token' >&2; exit 121; }
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
    if [ "${MOCK_MODE}" = extra-index-manifest ]; then
      platforms=${platforms},'{"mediaType":"application/vnd.oci.image.manifest.v1+json","digest":"sha256:ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff","platform":{"os":"linux","architecture":"s390x"}}'
    fi
    amd_reference=${amd_digest}
    [ "${MOCK_MODE}" != missing-reference ] || amd_reference=sha256:ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff
    printf '{"schemaVersion":2,"mediaType":"application/vnd.oci.image.index.v1+json","manifests":[%s,{"mediaType":"application/vnd.oci.image.manifest.v1+json","digest":"%s","platform":{"os":"unknown","architecture":"unknown"},"annotations":{"vnd.docker.reference.type":"attestation-manifest","vnd.docker.reference.digest":"%s"}},{"mediaType":"application/vnd.oci.image.manifest.v1+json","digest":"%s","platform":{"os":"unknown","architecture":"unknown"},"annotations":{"vnd.docker.reference.type":"attestation-manifest","vnd.docker.reference.digest":"%s"}}]}\n' \
      "${platforms}" "${amd_attestation_digest}" "${amd_reference}" "${ARM_ATTEST}" "${ARM}"
    ;;
  *@"${AMD_ATTEST}")
    provenance_digest=${AMD_PROVENANCE_DIGEST}
    provenance_size=${AMD_PROVENANCE_SIZE}
    provenance_predicate_type=https://slsa.dev/provenance/v1
    [ "${MOCK_MODE}" != provenance-v0.2 ] \
      || provenance_predicate_type=https://slsa.dev/provenance/v0.2
    [ "${MOCK_MODE}" != uppercase-layer-digest ] \
      || provenance_digest=sha256:$(printf '%s' "${provenance_digest#sha256:}" | tr '[:lower:]' '[:upper:]')
    [ "${MOCK_MODE}" != wrong-blob-digest ] \
      || provenance_digest=sha256:ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff
    [ "${MOCK_MODE}" != wrong-layer-size ] || provenance_size=$((provenance_size + 1))
    if [ "${MOCK_MODE}" = missing-sbom ]; then
      printf '{"schemaVersion":2,"mediaType":"application/vnd.oci.image.manifest.v1+json","layers":[{"mediaType":"application/vnd.in-toto+json","digest":"%s","size":%s,"annotations":{"in-toto.io/predicate-type":"%s"}}]}\n' "${provenance_digest}" "${provenance_size}" "${provenance_predicate_type}"
    elif [ "${MOCK_MODE}" = missing-provenance ]; then
      printf '{"schemaVersion":2,"mediaType":"application/vnd.oci.image.manifest.v1+json","layers":[{"mediaType":"application/vnd.in-toto+json","digest":"%s","size":%s,"annotations":{"in-toto.io/predicate-type":"https://spdx.dev/Document"}}]}\n' "${AMD_SPDX_DIGEST}" "${AMD_SPDX_SIZE}"
    elif [ "${MOCK_MODE}" = extra-attestation-layer ]; then
      printf '{"schemaVersion":2,"mediaType":"application/vnd.oci.image.manifest.v1+json","layers":[{"mediaType":"application/vnd.in-toto+json","digest":"%s","size":%s,"annotations":{"in-toto.io/predicate-type":"%s"}},{"mediaType":"application/vnd.in-toto+json","digest":"%s","size":%s,"annotations":{"in-toto.io/predicate-type":"https://spdx.dev/Document"}},{"mediaType":"application/vnd.in-toto+json","digest":"%s","size":%s,"annotations":{"in-toto.io/predicate-type":"https://spdx.dev/Document"}}]}\n' \
        "${provenance_digest}" "${provenance_size}" "${provenance_predicate_type}" \
        "${AMD_SPDX_DIGEST}" "${AMD_SPDX_SIZE}" "${AMD_SPDX_DIGEST}" "${AMD_SPDX_SIZE}"
    else
      printf '{"schemaVersion":2,"mediaType":"application/vnd.oci.image.manifest.v1+json","layers":[{"mediaType":"application/vnd.in-toto+json","digest":"%s","size":%s,"annotations":{"in-toto.io/predicate-type":"%s"}},{"mediaType":"application/vnd.in-toto+json","digest":"%s","size":%s,"annotations":{"in-toto.io/predicate-type":"https://spdx.dev/Document"}}]}\n' \
        "${provenance_digest}" "${provenance_size}" "${provenance_predicate_type}" "${AMD_SPDX_DIGEST}" "${AMD_SPDX_SIZE}"
    fi
    ;;
  *@"${ARM_ATTEST}")
    printf '{"schemaVersion":2,"mediaType":"application/vnd.oci.image.manifest.v1+json","layers":[{"mediaType":"application/vnd.in-toto+json","digest":"%s","size":%s,"annotations":{"in-toto.io/predicate-type":"https://slsa.dev/provenance/v1"}},{"mediaType":"application/vnd.in-toto+json","digest":"%s","size":%s,"annotations":{"in-toto.io/predicate-type":"https://spdx.dev/Document"}}]}\n' \
      "${ARM_PROVENANCE_DIGEST}" "${ARM_PROVENANCE_SIZE}" "${ARM_SPDX_DIGEST}" "${ARM_SPDX_SIZE}"
    ;;
  *) exit 91 ;;
esac
EOF

cat >"${TMP_DIR}/bin/curl" <<'EOF'
#!/bin/sh
set -eu
[ "${GHCR_TOKEN+x}" != x ] || { echo 'curl inherited GHCR_TOKEN' >&2; exit 96; }
[ "${GHCR_USERNAME+x}" != x ] || { echo 'curl inherited GHCR_USERNAME' >&2; exit 95; }
[ "${GITHUB_TOKEN+x}" != x ] || { echo 'curl inherited GITHUB_TOKEN' >&2; exit 97; }
[ "${raw_username+x}" != x ] || { echo 'curl inherited raw_username' >&2; exit 116; }
[ "${raw_token+x}" != x ] || { echo 'curl inherited raw_token' >&2; exit 117; }
[ "${escaped_username+x}" != x ] || { echo 'curl inherited escaped_username' >&2; exit 118; }
[ "${escaped_token+x}" != x ] || { echo 'curl inherited escaped_token' >&2; exit 119; }
[ "${registry_token+x}" != x ] || { echo 'curl inherited registry_token' >&2; exit 120; }
[ "${registry_token_with_sentinel+x}" != x ] || { echo 'curl inherited registry_token_with_sentinel' >&2; exit 123; }
[ "${escaped_registry_token+x}" != x ] || { echo 'curl inherited escaped_registry_token' >&2; exit 121; }

parse_curl_config_value() {
  prefix=$1
  config_path=$2
  awk -v prefix="${prefix}" '
    index($0, prefix) == 1 {
      if (found) exit 2
      value = substr($0, length(prefix) + 1)
      if (substr(value, length(value), 1) != "\"") exit 3
      value = substr(value, 1, length(value) - 1)
      decoded = ""
      escaped = 0
      for (i = 1; i <= length(value); i++) {
        character = substr(value, i, 1)
        if (escaped) {
          if (character != "\"" && character != "\\") exit 4
          decoded = decoded character
          escaped = 0
        } else if (character == "\\") {
          escaped = 1
        } else if (character == "\"") {
          exit 5
        } else {
          decoded = decoded character
        }
      }
      if (escaped) exit 6
      found = 1
    }
    END {
      if (found != 1) exit 7
      printf "%s", decoded
    }
  ' "${config_path}"
}

printf '%s\n' "$*" >>"${MOCK_CURL_LOG}"
case "${MOCK_MODE}" in
  signal-hup-curl) kill -HUP "${PPID}"; exit 0 ;;
  signal-int-curl) kill -INT "${PPID}"; exit 0 ;;
  signal-term-curl) kill -TERM "${PPID}"; exit 0 ;;
esac
[ "${1-}" = -q ] || exit 98
[ "${2-}" = --config ] || exit 109
config=$3
if grep -q 'ghcr.io/token?' "${config}"; then
  auth_value=$(parse_curl_config_value 'user = "' "${config}") || exit 99
  auth_sha=$(printf '%s' "${auth_value}" | openssl dgst -sha256 | awk '{print $NF}')
  unset auth_value
  [ "${auth_sha}" = "${MOCK_EXPECTED_AUTH_SHA}" ] || exit 108
  unset auth_sha
  mock_registry_token=${MOCK_REGISTRY_TOKEN}
  case "${MOCK_MODE}" in
    registry-token-empty) mock_registry_token= ;;
    registry-token-lf) mock_registry_token="${mock_registry_token}
" ;;
    registry-token-cr) mock_registry_token="${mock_registry_token}$(printf '\r')" ;;
  esac
  printf '%s' "${mock_registry_token}" | jq -Rs '{token: .}'
  unset mock_registry_token
  exit 0
fi
registry_auth_value=$(parse_curl_config_value 'header = "Authorization: Bearer ' "${config}") || exit 100
registry_auth_sha=$(printf '%s' "${registry_auth_value}" | openssl dgst -sha256 | awk '{print $NF}')
unset registry_auth_value
[ "${registry_auth_sha}" = "${MOCK_EXPECTED_REGISTRY_AUTH_SHA}" ] || exit 122
unset registry_auth_sha
grep -q '^location$' "${config}" || exit 104
grep -q '^proto-redir = "=https"$' "${config}" || exit 105
grep -q '^max-redirs = 3$' "${config}" || exit 106
if grep -q '^location-trusted' "${config}"; then exit 107; fi
[ "${4-}" = --url ] || exit 101
digest=${5##*/}
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
chmod 0700 "${TMP_DIR}/bin/mktemp" "${TMP_DIR}/bin/docker" "${TMP_DIR}/bin/curl"

run_verifier() {
  requested_mode=$1
  verifier_username=${2-${TEST_GHCR_USERNAME}}
  verifier_token=${3-${TEST_GHCR_TOKEN}}
  prepare_fixtures "${requested_mode}"
  repository=${EXPECTED_REPOSITORY}
  expected_builder_id=${CURRENT_BUILDER_ID}
  expected_builder_mode=current
  [ "${requested_mode}" != wrong-repository ] || repository=ghcr.io/conalog/wrong
  case "${requested_mode}" in
    prior-success|prior-wrong-run|prior-current-attempt|prior-future-attempt)
      expected_builder_id=https://github.com/Conalog/grant/actions/runs/29366035201/attempts/2
      expected_builder_mode=prior-attempt
      ;;
    prior-platform-disagreement)
      expected_builder_id=https://github.com/Conalog/grant/actions/runs/29366035201/attempts/3
      expected_builder_mode=prior-attempt
      ;;
    invalid-expected-builder-id) expected_builder_id=https://example.invalid/builder ;;
    invalid-builder-mode) expected_builder_mode=unexpected ;;
  esac
  set -- sh "${ROOT_DIR}/scripts/verify-oci-attestations.sh"
  [ "${requested_mode}" != allexport ] || set -- sh -a "${ROOT_DIR}/scripts/verify-oci-attestations.sh"
  MOCK_MODE=${requested_mode} \
    MOCK_DOCKER_LOG="${TMP_DIR}/${requested_mode}.docker.log" \
    MOCK_CURL_LOG="${TMP_DIR}/${requested_mode}.curl.log" \
    MOCK_EXPECTED_AUTH_SHA=${TEST_AUTH_SHA} \
    MOCK_REGISTRY_TOKEN=${TEST_REGISTRY_TOKEN} \
    MOCK_EXPECTED_REGISTRY_AUTH_SHA=${TEST_REGISTRY_TOKEN_SHA} \
    REAL_MKTEMP=${REAL_MKTEMP} \
    GHCR_USERNAME=${verifier_username} GHCR_TOKEN=${verifier_token} \
    GITHUB_TOKEN=github-credential-must-not-leak \
    HOME="${TMP_DIR}/malicious-curl-home" XDG_CONFIG_HOME="${TMP_DIR}/malicious-curl-xdg" \
    CURL_HOME="${TMP_DIR}/malicious-curl-home" TMPDIR="${TMP_DIR}/verifier-tmp" PATH="${TMP_DIR}/bin:${PATH}" \
    IMAGE_REPOSITORY=${repository} IMAGE_REF=${repository}:source-tag \
    EXPECTED_DIGEST=${EXPECTED} EXPECTED_SOURCE_SHA=${SOURCE_SHA} \
    EXPECTED_BUILDER_ID=${expected_builder_id} EXPECTED_BUILDER_MODE=${expected_builder_mode} \
    "$@"
}

start_redirect_fixture
verify_real_curl_redirect_contract
run_verifier success >"${TMP_DIR}/success.log"
run_verifier allexport >"${TMP_DIR}/allexport.log"
run_verifier prior-success >"${TMP_DIR}/prior-success.log"
for rejected_mode in \
  wrong-repository wrong-digest missing-platform missing-reference extra-index-manifest \
  missing-sbom missing-provenance extra-attestation-layer \
  uppercase-platform-digest uppercase-attestation-digest uppercase-layer-digest wrong-layer-size \
  wrong-blob-digest missing-subject wrong-subject wrong-type wrong-predicate-type malformed-statement \
  empty-provenance malformed-provenance wrong-builder wrong-source-sha wrong-source-repository \
  invalid-expected-builder-id invalid-builder-mode prior-wrong-run prior-current-attempt \
  prior-future-attempt prior-platform-disagreement \
  wrong-config-source wrong-vcs-revision wrong-vcs-source wrong-vcs-context-dir \
  wrong-vcs-dockerfile-dir wrong-build-type empty-resolved-dependencies \
  wrong-sbom-generator-dependency \
  wrong-builder-platform empty-llb-definition incomplete-request provenance-v0.2 \
  empty-spdx malformed-spdx invalid-spdx empty-packages \
  registry-token-empty registry-token-lf registry-token-cr; do
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
has_stale_files=0
for stale_file in "${TMP_DIR}/verifier-tmp"/oci-attestations.*; do
  if [ -e "${stale_file}" ]; then
    has_stale_files=1
    break
  fi
done
if [ "${has_stale_files}" -eq 1 ]; then
  echo "registry verifier did not clean its temporary directory after a signal" >&2
  exit 1
fi

newline='
'
carriage_return=$(printf '\r')
for injection_contract in username-lf:username username-cr:username token-lf:token token-cr:token; do
  injection_mode=${injection_contract%:*}
  injection_field=${injection_contract#*:}
  injection_username=${TEST_GHCR_USERNAME}
  injection_token=${TEST_GHCR_TOKEN}
  case "${injection_mode}" in
    *-lf) injection_character=${newline} ;;
    *-cr) injection_character=${carriage_return} ;;
  esac
  case "${injection_field}" in
    username) injection_username="${TEST_GHCR_USERNAME}${injection_character}injected" ;;
    token) injection_token="${TEST_GHCR_TOKEN}${injection_character}injected" ;;
  esac
  set +e
  run_verifier "${injection_mode}" "${injection_username}" "${injection_token}" \
    >"${TMP_DIR}/${injection_mode}.log" 2>&1
  injection_status=$?
  set -e
  [ "${injection_status}" -eq 2 ] \
    || { echo "${injection_mode} credential exited ${injection_status}, expected 2" >&2; exit 1; }
  grep -F 'GHCR credentials must not contain carriage return or newline characters' \
    "${TMP_DIR}/${injection_mode}.log" >/dev/null \
    || { echo "${injection_mode} credential did not fail closed" >&2; exit 1; }
  if [ -s "${TMP_DIR}/${injection_mode}.curl.log" ]; then
    echo "${injection_mode} credential reached curl" >&2
    exit 1
  fi
done
unset newline carriage_return injection_character injection_username injection_token

mkdir -p "${TMP_DIR}/credential-scan-error" "${TMP_DIR}/scan-bin"
printf '%s\n' harmless >"${TMP_DIR}/credential-scan-error/unreadable"
cat >"${TMP_DIR}/scan-bin/grep" <<'EOF'
#!/bin/sh
exit 2
EOF
chmod 0700 "${TMP_DIR}/scan-bin/grep"
original_path=${PATH}
PATH="${TMP_DIR}/scan-bin:${PATH}"
export PATH
set +e
scan_for_credentials "${TMP_DIR}/credential-scan-error"
forced_scan_status=$?
set -e
PATH=${original_path}
export PATH
[ "${forced_scan_status}" -eq 2 ] \
  || { echo "credential scan ignored a forced grep read error" >&2; exit 1; }
rm -rf "${TMP_DIR}/credential-scan-error" "${TMP_DIR}/scan-bin"

set +e
scan_for_credentials "${TMP_DIR}"
credential_scan_status=$?
set -e
case "${credential_scan_status}" in
  0) ;;
  1) echo "registry verifier exposed a credential" >&2; exit 1 ;;
  *) echo "registry verifier credential scan failed" >&2; exit 1 ;;
esac

echo 'OCI digest-bound in-toto provenance and SPDX envelope verification contract passed'
