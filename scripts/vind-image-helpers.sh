#!/usr/bin/env bash

# Shared vCluster image helpers. Callers provide CLUSTER_NAME and RAUTHY_IMAGE.

cluster_container_name() {
  printf 'vcluster.cp.%s' "${CLUSTER_NAME}"
}

image_refs_to_match() {
  local image="$1"

  printf '%s\n' "${image}"
  if [[ "${image%%/*}" == "${image}" ]]; then
    printf 'docker.io/library/%s\n' "${image}"
  fi
}

redact_image_import_log() {
  local log_file="$1"

  sed -E \
    -e 's#(https?://)[^/@[:space:]]+@#\1[redacted]@#g' \
    -e 's#([Aa]uthorization:[[:space:]]*(Basic|Bearer)[[:space:]]+)[^[:space:]]+#\1[redacted]#g' \
    -e 's#([?&](access_token|password|token)=)[^&[:space:]]+#\1[redacted]#g' \
    "${log_file}" >&2
}

report_image_import_failure() {
  local image="$1"
  local step="$2"
  local log_file="$3"

  echo "failed to import image into vcluster runtime during ${step}: ${image}" >&2
  if [[ -s "${log_file}" ]]; then
    redact_image_import_log "${log_file}"
  fi
}

import_image_with_ctr_vcluster() (
  local image="$1"
  local container_name
  local import_log=""
  local remote_tar=""
  local tar_file=""

  # shellcheck disable=SC2317,SC2329 # Invoked by the EXIT trap.
  cleanup_image_import() {
    if [[ -n "${tar_file}" ]]; then
      rm -f "${tar_file}"
    fi
    if [[ -n "${import_log}" ]]; then
      rm -f "${import_log}"
    fi
    if [[ -n "${remote_tar}" ]] && [[ -n "${container_name}" ]]; then
      docker exec "${container_name}" rm -f "${remote_tar}" >/dev/null 2>&1 || true
    fi
  }
  trap cleanup_image_import EXIT
  trap 'exit 129' HUP
  trap 'exit 130' INT
  trap 'exit 143' TERM

  container_name="$(cluster_container_name)"
  docker inspect "${container_name}" >/dev/null 2>&1 || {
    echo "missing vcluster container ${container_name}" >&2
    exit 1
  }

  tar_file="$(mktemp "${TMPDIR:-/tmp}/${CLUSTER_NAME}.image.XXXXXX")"
  import_log="$(mktemp "${TMPDIR:-/tmp}/${CLUSTER_NAME}.image-import.XXXXXX")"
  # The vCluster docker driver mounts /tmp as an in-VM tmpfs. `docker cp`
  # writes the container rootfs underneath that mount, so a copied /tmp file
  # is invisible to `docker exec`. /home is part of the same visible rootfs in
  # which ctr runs.
  remote_tar="/home/${CLUSTER_NAME}.image-import.$$.tar"

  if ! docker save "${image}" -o "${tar_file}" >"${import_log}" 2>&1; then
    report_image_import_failure "${image}" "Docker archive creation" "${import_log}"
    return 1
  fi

  if ! docker cp "${tar_file}" "${container_name}:${remote_tar}" >"${import_log}" 2>&1; then
    report_image_import_failure "${image}" "containerd archive transfer" "${import_log}"
    return 1
  fi

  if ! docker exec "${container_name}" ctr -n k8s.io images import "${remote_tar}" >"${import_log}" 2>&1; then
    report_image_import_failure "${image}" "containerd archive import" "${import_log}"
    return 1
  fi
)

is_digest_qualified_image() {
  [[ "$1" =~ @sha256:[0-9a-f]{64}$ ]]
}

ensure_runtime_image() {
  local image="$1"
  local container_name
  local refs_file

  container_name="$(cluster_container_name)"
  docker inspect "${container_name}" >/dev/null 2>&1 || {
    echo "missing vind runtime for ${CLUSTER_NAME}; expected vcluster" >&2
    exit 1
  }

  refs_file="$(mktemp "${TMPDIR:-/tmp}/${CLUSTER_NAME}.refs.XXXXXX")"
  image_refs_to_match "${image}" >"${refs_file}"

  if docker exec "${container_name}" ctr -n k8s.io images ls -q \
    | grep -F -x -f "${refs_file}" >/dev/null 2>&1; then
    echo "[vind] image already present in runtime: ${image}"
    rm -f "${refs_file}"
    return 0
  fi

  echo "[vind] importing image into runtime: ${image}"
  import_image_with_ctr_vcluster "${image}"
  docker exec "${container_name}" ctr -n k8s.io images ls -q \
    | grep -F -x -f "${refs_file}" >/dev/null 2>&1 || {
    rm -f "${refs_file}"
    echo "image archive import completed but the runtime reference is missing: ${image}" >&2
    exit 1
  }

  rm -f "${refs_file}"
}

ensure_rauthy_runtime_image() {
  if is_digest_qualified_image "${RAUTHY_IMAGE}"; then
    echo "[vind] leaving digest-qualified Rauthy image for authenticated kubelet pull: ${RAUTHY_IMAGE}"
    return 0
  fi

  ensure_runtime_image "${RAUTHY_IMAGE}"
}
