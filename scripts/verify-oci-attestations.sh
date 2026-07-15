#!/bin/sh
set -eu
set +a

IMAGE_REPOSITORY=${IMAGE_REPOSITORY:?IMAGE_REPOSITORY is required}
IMAGE_REF=${IMAGE_REF:?IMAGE_REF is required}
EXPECTED_DIGEST=${EXPECTED_DIGEST:?EXPECTED_DIGEST is required}
EXPECTED_SOURCE_SHA=${EXPECTED_SOURCE_SHA:?EXPECTED_SOURCE_SHA is required}
EXPECTED_BUILDER_ID=${EXPECTED_BUILDER_ID:?EXPECTED_BUILDER_ID is required}
EXPECTED_BUILDER_MODE=${EXPECTED_BUILDER_MODE:?EXPECTED_BUILDER_MODE is required}
GHCR_USERNAME=${GHCR_USERNAME:?GHCR_USERNAME is required}
GHCR_TOKEN=${GHCR_TOKEN:?GHCR_TOKEN is required}
unset raw_username raw_token escaped_username escaped_token registry_token registry_token_with_sentinel escaped_registry_token
raw_username=${GHCR_USERNAME}
raw_token=${GHCR_TOKEN}
unset GHCR_USERNAME GHCR_TOKEN GITHUB_TOKEN
EXPECTED_IMAGE_REPOSITORY=ghcr.io/conalog/grant-rauthy-launcher
EXPECTED_SOURCE_REPOSITORY=https://github.com/Conalog/grant

valid_digest() {
  case "$1" in
    sha256:????????????????????????????????????????????????????????????????) ;;
    *) return 1 ;;
  esac
  case "${1#sha256:}" in *[!0-9a-f]*) return 1 ;; esac
}

escape_curl_config_value() {
  printf '%s' "$1" | sed 's/\\/\\\\/g; s/"/\\"/g'
}

[ "${IMAGE_REPOSITORY}" = "${EXPECTED_IMAGE_REPOSITORY}" ] \
  || { echo "image repository must be ${EXPECTED_IMAGE_REPOSITORY}" >&2; exit 2; }
valid_digest "${EXPECTED_DIGEST}" \
  || { echo "expected image digest must be sha256 plus 64 lowercase hex characters" >&2; exit 2; }
case "${EXPECTED_SOURCE_SHA}" in
  ????????????????????????????????????????) ;;
  *) echo "expected source SHA must contain exactly 40 lowercase hex characters" >&2; exit 2 ;;
esac
case "${EXPECTED_SOURCE_SHA}" in *[!0-9a-f]*) echo "expected source SHA is not lowercase hexadecimal" >&2; exit 2 ;; esac
config_newline='
'
config_carriage_return=$(printf '\r')
case "${raw_username}${raw_token}" in
  *"${config_newline}"*|*"${config_carriage_return}"*)
    echo "GHCR credentials must not contain carriage return or newline characters" >&2
    exit 2
    ;;
esac
for command_name in curl docker jq mktemp openssl; do
  command -v "${command_name}" >/dev/null 2>&1 || { echo "missing command: ${command_name}" >&2; exit 1; }
done
jq -en --arg builderId "${EXPECTED_BUILDER_ID}" '
  $builderId | test("^https://github\\.com/Conalog/grant/actions/runs/[1-9][0-9]*/attempts/[1-9][0-9]{0,8}$")
' >/dev/null || { echo "expected builder ID is not a Grant GitHub Actions run attempt" >&2; exit 2; }
case "${EXPECTED_BUILDER_MODE}" in
  current|prior-attempt) ;;
  *) echo "expected builder mode must be current or prior-attempt" >&2; exit 2 ;;
esac
expected_builder_prefix=${EXPECTED_BUILDER_ID%/*}
expected_builder_attempt=${EXPECTED_BUILDER_ID##*/}

umask 077
TMP_DIR=$(mktemp -d "${TMPDIR:-/tmp}/oci-attestations.XXXXXX")
cleanup() {
  status=$?
  trap - 0 HUP INT TERM
  unset GHCR_USERNAME GHCR_TOKEN GITHUB_TOKEN raw_username raw_token escaped_username escaped_token registry_token registry_token_with_sentinel escaped_registry_token
  rm -rf "${TMP_DIR}"
  exit "${status}"
}
trap cleanup 0
trap 'exit 129' HUP
trap 'exit 130' INT
trap 'exit 143' TERM

auth_config=${TMP_DIR}/ghcr-auth.curlrc
token_response=${TMP_DIR}/ghcr-token.json
registry_config=${TMP_DIR}/ghcr-registry.curlrc
escaped_username=$(escape_curl_config_value "${raw_username}")
escaped_token=$(escape_curl_config_value "${raw_token}")
{
  printf 'silent\n'
  printf 'show-error\n'
  printf 'fail\n'
  printf 'user = "%s:%s"\n' "${escaped_username}" "${escaped_token}"
  printf 'url = "https://ghcr.io/token?service=ghcr.io&scope=repository:conalog/grant-rauthy-launcher:pull"\n'
} >"${auth_config}"
unset raw_username raw_token escaped_username escaped_token
curl -q --config "${auth_config}" >"${token_response}"
if ! registry_token_with_sentinel=$(
  jq -ej '(.token // .access_token) | select(type == "string" and length > 0)' "${token_response}" \
    && printf x
); then
  echo "GHCR token response did not contain a non-empty token" >&2
  exit 1
fi
registry_token=${registry_token_with_sentinel%?}
unset registry_token_with_sentinel
case "${registry_token}" in
  *"${config_newline}"*|*"${config_carriage_return}"*)
    echo "GHCR token response contained an invalid token" >&2
    exit 1
    ;;
esac
escaped_registry_token=$(escape_curl_config_value "${registry_token}")
{
  printf 'silent\n'
  printf 'show-error\n'
  printf 'fail\n'
  printf 'location\n'
  printf 'proto-redir = "=https"\n'
  printf 'max-redirs = 3\n'
  printf 'header = "Authorization: Bearer %s"\n' "${escaped_registry_token}"
} >"${registry_config}"
unset registry_token escaped_registry_token config_newline config_carriage_return
rm -f "${auth_config}" "${token_response}"

fetch_blob() {
  blob_digest=$1
  blob_output=$2
  curl -q --config "${registry_config}" \
    --url "https://ghcr.io/v2/conalog/grant-rauthy-launcher/blobs/${blob_digest}" >"${blob_output}"
}

inspection=${TMP_DIR}/tag-inspection.txt
docker buildx imagetools inspect "${IMAGE_REF}" >"${inspection}"
resolved_digest=$(sed -n 's/^Digest:[[:space:]]*\(sha256:[0-9a-f][0-9a-f]*\)[[:space:]]*$/\1/p' "${inspection}")
[ "${resolved_digest}" = "${EXPECTED_DIGEST}" ] \
  || { echo "published tag does not resolve to the build output digest" >&2; exit 1; }

index=${TMP_DIR}/index.json
docker buildx imagetools inspect --raw "${IMAGE_REPOSITORY}@${EXPECTED_DIGEST}" >"${index}"
jq -e '
  .mediaType == "application/vnd.oci.image.index.v1+json" and
  (.manifests | type == "array" and length == 4) and
  ([.manifests[] |
    select(.mediaType == "application/vnd.oci.image.manifest.v1+json" and
      .platform.os == "linux" and
      (.platform.architecture == "amd64" or .platform.architecture == "arm64") and
      (.digest | type == "string" and test("^sha256:[0-9a-f]{64}$")))] | length) == 2 and
  ([.manifests[] | select(.platform.os == "linux" and .platform.architecture == "amd64")] | length) == 1 and
  ([.manifests[] | select(.platform.os == "linux" and .platform.architecture == "arm64")] | length) == 1 and
  ([.manifests[] |
    select(.mediaType == "application/vnd.oci.image.manifest.v1+json" and
      .platform.os == "unknown" and .platform.architecture == "unknown" and
      (.digest | type == "string" and test("^sha256:[0-9a-f]{64}$")) and
      .annotations["vnd.docker.reference.type"] == "attestation-manifest" and
      (.annotations["vnd.docker.reference.digest"] |
        type == "string" and test("^sha256:[0-9a-f]{64}$")))] | length) == 2 and
  ([.manifests[] | select(.platform.os == "linux" and
      (.platform.architecture == "amd64" or .platform.architecture == "arm64"))] as $images |
   [.manifests[] | select(.annotations["vnd.docker.reference.type"] == "attestation-manifest")] as $attestations |
   all($images[]; .digest as $imageDigest |
     any($attestations[]; .annotations["vnd.docker.reference.digest"] == $imageDigest)) and
   all($attestations[]; .annotations["vnd.docker.reference.digest"] as $imageDigest |
     any($images[]; .digest == $imageDigest)))
' "${index}" >/dev/null || { echo "published OCI index lacks the required platform-attestation links" >&2; exit 1; }

pairs=${TMP_DIR}/attestation-pairs.tsv
jq -r '.manifests[] |
  select(.annotations["vnd.docker.reference.type"] == "attestation-manifest") |
  [.annotations["vnd.docker.reference.digest"], .digest] | @tsv' "${index}" >"${pairs}"
[ "$(wc -l <"${pairs}" | tr -d ' ')" -eq 2 ] \
  || { echo "expected exactly two platform attestation manifests" >&2; exit 1; }

TAB=$(printf '\t')
resolved_prior_builder_id=
while IFS="${TAB}" read -r platform_digest attestation_digest; do
  if ! valid_digest "${platform_digest}" || ! valid_digest "${attestation_digest}"; then
    echo "invalid lowercase platform-attestation digest pair" >&2
    exit 1
  fi
  attestation=${TMP_DIR}/${attestation_digest#sha256:}.json
  docker buildx imagetools inspect --raw "${IMAGE_REPOSITORY}@${attestation_digest}" >"${attestation}"
  jq -e '
    .mediaType == "application/vnd.oci.image.manifest.v1+json" and
    ([.layers[]? | select(.annotations["in-toto.io/predicate-type"] == "https://slsa.dev/provenance/v1")] | length) == 1 and
    ([.layers[]? | select(.annotations["in-toto.io/predicate-type"] == "https://spdx.dev/Document")] | length) == 1 and
    (.layers | length) == 2 and
    all(.layers[];
      .mediaType == "application/vnd.in-toto+json" and
      (.digest | type == "string" and test("^sha256:[0-9a-f]{64}$")) and
      (.size | type == "number" and . > 0 and . == floor) and
      (.annotations["in-toto.io/predicate-type"] == "https://slsa.dev/provenance/v1" or
       .annotations["in-toto.io/predicate-type"] == "https://spdx.dev/Document"))
  ' "${attestation}" >/dev/null \
    || { echo "attestation ${attestation_digest} has invalid provenance or SPDX layer descriptors" >&2; exit 1; }

  layers=${TMP_DIR}/${attestation_digest#sha256:}.layers.tsv
  jq -r '.layers[] | [.annotations["in-toto.io/predicate-type"], .digest, (.size | tostring)] | @tsv' \
    "${attestation}" >"${layers}"
  while IFS="${TAB}" read -r predicate_type layer_digest layer_size; do
    valid_digest "${layer_digest}" \
      || { echo "invalid lowercase attestation layer digest" >&2; exit 1; }
    blob=${TMP_DIR}/${layer_digest#sha256:}.blob
    fetch_blob "${layer_digest}" "${blob}"
    actual_size=$(wc -c <"${blob}" | tr -d ' ')
    [ "${actual_size}" = "${layer_size}" ] \
      || { echo "attestation blob size does not match its descriptor" >&2; exit 1; }
    actual_sha=$(openssl dgst -sha256 "${blob}" | awk '{print $NF}')
    [ "sha256:${actual_sha}" = "${layer_digest}" ] \
      || { echo "attestation blob digest does not match its descriptor" >&2; exit 1; }

    platform_sha=${platform_digest#sha256:}
    jq -e --arg predicateType "${predicate_type}" --arg platformSha "${platform_sha}" '
      ._type == "https://in-toto.io/Statement/v0.1" and
      .predicateType == $predicateType and
      (.subject | type == "array" and length > 0) and
      any(.subject[];
        (.name | type == "string" and length > 0) and
        .digest.sha256 == $platformSha)
    ' "${blob}" >/dev/null \
      || { echo "attestation envelope does not bind the expected platform manifest" >&2; exit 1; }

    case "${predicate_type}" in
      https://slsa.dev/provenance/v1)
        observed_builder_id=$(jq -er '.predicate.runDetails.builder.id | select(type == "string")' "${blob}") \
          || { echo "platform provenance predicate lacks a builder ID" >&2; exit 1; }
        case "${EXPECTED_BUILDER_MODE}" in
          current)
            [ "${observed_builder_id}" = "${EXPECTED_BUILDER_ID}" ] \
              || { echo "platform provenance builder is not the current workflow run attempt" >&2; exit 1; }
            ;;
          prior-attempt)
            [ "${observed_builder_id%/*}" = "${expected_builder_prefix}" ] \
              || { echo "platform provenance builder is not from the current workflow run" >&2; exit 1; }
            observed_builder_attempt=${observed_builder_id##*/}
            case "${observed_builder_attempt}" in
              [1-9]|[1-9][0-9]|[1-9][0-9][0-9]|[1-9][0-9][0-9][0-9]|\
              [1-9][0-9][0-9][0-9][0-9]|[1-9][0-9][0-9][0-9][0-9][0-9]|\
              [1-9][0-9][0-9][0-9][0-9][0-9][0-9]|\
              [1-9][0-9][0-9][0-9][0-9][0-9][0-9][0-9]|\
              [1-9][0-9][0-9][0-9][0-9][0-9][0-9][0-9][0-9]) ;;
              *) echo "platform provenance builder attempt is invalid" >&2; exit 1 ;;
            esac
            [ "${observed_builder_attempt}" -lt "${expected_builder_attempt}" ] \
              || { echo "platform provenance builder is not from a prior run attempt" >&2; exit 1; }
            if [ -n "${resolved_prior_builder_id}" ] && [ "${resolved_prior_builder_id}" != "${observed_builder_id}" ]; then
              echo "platform provenance builders disagree across architectures" >&2
              exit 1
            fi
            resolved_prior_builder_id=${observed_builder_id}
            ;;
        esac
        jq -e \
          --arg expectedBuilderId "${observed_builder_id}" \
          --arg sourceSha "${EXPECTED_SOURCE_SHA}" \
          --arg sourceRepository "${EXPECTED_SOURCE_REPOSITORY}" '
          (.predicate | type == "object") and
          (.predicate.buildDefinition | type == "object") and
          (.predicate.runDetails | type == "object") and
          .predicate.buildDefinition.buildType ==
            "https://github.com/moby/buildkit/blob/master/docs/attestations/slsa-definitions.md" and
          .predicate.buildDefinition.externalParameters.configSource.path == "Dockerfile" and
          .predicate.buildDefinition.externalParameters.request.root.configSource.path == "Dockerfile" and
          .predicate.buildDefinition.internalParameters.builderPlatform == "linux/amd64" and
          (.predicate.buildDefinition.internalParameters.buildConfig.llbDefinition |
            type == "array" and length > 0) and
          .predicate.runDetails.builder.id == $expectedBuilderId and
          .predicate.runDetails.metadata.buildkit_completeness.request == true and
          (.predicate.buildDefinition.resolvedDependencies | type == "array" and length > 0) and
          any(.predicate.buildDefinition.resolvedDependencies[];
            .uri == "pkg:docker/docker/buildkit-syft-scanner@stable-1" and
            .digest.sha256 == "79e7b013cbec16bbb436f312819a49a4a57752b2270c1a9332ae1a10fcc82a68") and
          .predicate.buildDefinition.externalParameters.request.args[
            "label:org.opencontainers.image.revision"] == $sourceSha and
          .predicate.buildDefinition.externalParameters.request.args[
            "label:org.opencontainers.image.source"] == $sourceRepository and
          .predicate.buildDefinition.externalParameters.request.root.request.args[
            "vcs:revision"] == $sourceSha and
          .predicate.buildDefinition.externalParameters.request.root.request.args[
            "vcs:localdir:context"] == "deploy/rauthy-launcher" and
          .predicate.buildDefinition.externalParameters.request.root.request.args[
            "vcs:localdir:dockerfile"] == "deploy/rauthy-launcher" and
          (.predicate.buildDefinition.externalParameters.request.root.request.args[
              "vcs:source"] as $vcsSource |
            $vcsSource == $sourceRepository or $vcsSource == ($sourceRepository + ".git"))
        ' "${blob}" >/dev/null \
          || { echo "platform provenance predicate does not match the BuildKit source contract" >&2; exit 1; }
        ;;
      https://spdx.dev/Document)
        jq -e '
          (.predicate | type == "object") and
          (.predicate.spdxVersion == "SPDX-2.2" or .predicate.spdxVersion == "SPDX-2.3") and
          .predicate.SPDXID == "SPDXRef-DOCUMENT" and
          (.predicate.documentNamespace | type == "string" and length > 0) and
          (.predicate.packages | type == "array" and length > 0)
        ' "${blob}" >/dev/null \
          || { echo "platform SBOM predicate is not a populated SPDX document" >&2; exit 1; }
        ;;
      *) echo "unexpected attestation predicate type" >&2; exit 1 ;;
    esac
  done <"${layers}"
done <"${pairs}"

printf 'verified digest-bound in-toto provenance and SPDX SBOM envelopes for %s@%s\n' "${IMAGE_REPOSITORY}" "${EXPECTED_DIGEST}"
