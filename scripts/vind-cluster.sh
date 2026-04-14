#!/usr/bin/env bash
set -euo pipefail

CLUSTER_NAME="${CLUSTER_NAME:-certd-ci}"
RESET_CLUSTER="${RESET_CLUSTER:-0}"
KEEP_CLUSTER="${KEEP_CLUSTER:-0}"
VIND_COLIMA_AUTOFIX="${VIND_COLIMA_AUTOFIX:-1}"
VIND_UNTAINT_CONTROL_PLANE_NODES="${VIND_UNTAINT_CONTROL_PLANE_NODES:-1}"
VIND_ALLOW_K3D_FALLBACK="${VIND_ALLOW_K3D_FALLBACK:-0}"

need() {
  command -v "$1" >/dev/null 2>&1 || {
    echo "missing command: $1" >&2
    exit 1
  }
}

need docker
need kubectl
need vcluster

usage() {
  cat <<'EOF' >&2
usage: scripts/vind-cluster.sh <ensure-context|delete|preflight>
EOF
  exit 1
}

cluster_container_name() {
  printf 'vcluster.cp.%s' "${CLUSTER_NAME}"
}

k3d_context_name() {
  printf 'k3d-%s' "${CLUSTER_NAME}"
}

is_colima_runtime() {
  command -v colima >/dev/null 2>&1 || return 1

  local docker_host=""
  docker_host="$(docker context inspect "$(docker context show)" -f '{{ (index .Endpoints "docker").Host }}' 2>/dev/null || true)"
  [[ "${DOCKER_HOST:-${docker_host}}" == *".colima"* ]] || return 1

  colima status >/dev/null 2>&1
}

ensure_colima_kernel_modules() {
  if ! is_colima_runtime; then
    return 0
  fi

  if colima ssh -- sudo sh -lc 'lsmod | grep -q "^br_netfilter"' >/dev/null 2>&1; then
    return 0
  fi

  if [[ "${VIND_COLIMA_AUTOFIX}" == "1" ]]; then
    echo "[vind] loading br_netfilter inside Colima" >&2
    colima ssh -- sudo modprobe br_netfilter >/dev/null
    colima ssh -- sudo sh -lc \
      'sysctl -w net.bridge.bridge-nf-call-iptables=1 >/dev/null && sysctl -w net.bridge.bridge-nf-call-ip6tables=1 >/dev/null' \
      >/dev/null 2>&1 || true
  fi

  if ! colima ssh -- sudo sh -lc 'lsmod | grep -q "^br_netfilter"' >/dev/null 2>&1; then
    cat >&2 <<'EOF'
vind preflight failed: Colima is active but br_netfilter is not loaded.
Run:
  colima ssh -- sudo modprobe br_netfilter
and retry, or set VIND_COLIMA_AUTOFIX=1 to let the script attempt it automatically.
EOF
    exit 1
  fi
}

docker_server_architecture() {
  docker version -f '{{.Server.Arch}}' 2>/dev/null || true
}

probe_image_architecture() {
  docker image inspect alpine:latest --format '{{.Architecture}}' 2>/dev/null || true
}

ensure_arm64_probe_image() {
  if ! is_colima_runtime; then
    return 0
  fi

  if [[ "$(docker_server_architecture)" != "arm64" ]]; then
    return 0
  fi

  if [[ "$(probe_image_architecture)" == "arm64" ]]; then
    return 0
  fi

  if [[ "${VIND_COLIMA_AUTOFIX}" == "1" ]]; then
    echo "[vind] refreshing alpine:latest for arm64 probe containers" >&2
    docker pull --platform linux/arm64 alpine:latest >/dev/null
  fi

  if [[ "$(probe_image_architecture)" != "arm64" ]]; then
    cat >&2 <<'EOF'
vind preflight failed: alpine:latest is not pinned to arm64 on this Colima runtime.
Run:
  docker pull --platform linux/arm64 alpine:latest
and retry, or set VIND_COLIMA_AUTOFIX=1 to let the script attempt it automatically.
EOF
    exit 1
  fi
}

use_driver() {
  vcluster use driver docker >/dev/null
}

vcluster_cluster_exists() {
  docker inspect "$(cluster_container_name)" >/dev/null 2>&1
}

have_k3d() {
  command -v k3d >/dev/null 2>&1
}

k3d_cluster_exists() {
  have_k3d && k3d cluster get "${CLUSTER_NAME}" >/dev/null 2>&1
}

cluster_exists() {
  vcluster_cluster_exists || k3d_cluster_exists
}

current_driver() {
  if vcluster_cluster_exists; then
    printf 'vcluster'
  elif k3d_cluster_exists; then
    printf 'k3d'
  else
    return 1
  fi
}

delete_cluster() {
  if vcluster_cluster_exists; then
    use_driver
    vcluster delete "${CLUSTER_NAME}" >/dev/null
  elif k3d_cluster_exists; then
    k3d cluster delete "${CLUSTER_NAME}" >/dev/null
  fi
}

connect_cluster() {
  use_driver
  CLUSTER_NAME="${CLUSTER_NAME}" \
  VIND_UNTAINT_CONTROL_PLANE_NODES="${VIND_UNTAINT_CONTROL_PLANE_NODES}" \
    bash "$(dirname "${BASH_SOURCE[0]}")/vind-exec.sh" bash -lc '
      kubectl wait --for=condition=Ready nodes --all --timeout=120s >/dev/null
      echo "[vind] node taints before normalization:" >&2
      kubectl get nodes -o jsonpath='"'"'{range .items[*]}{.metadata.name}{"\t"}{range .spec.taints[*]}{.key}{"="}{.effect}{","}{end}{"\n"}{end}'"'"' 2>/dev/null >&2 || true
      if [[ "${VIND_UNTAINT_CONTROL_PLANE_NODES:-1}" == "1" ]]; then
        kubectl taint nodes --all node-role.kubernetes.io/control-plane:NoSchedule- >/dev/null 2>&1 || true
        kubectl taint nodes --all node-role.kubernetes.io/master:NoSchedule- >/dev/null 2>&1 || true
        if kubectl get nodes -o jsonpath='"'"'{range .items[*]}{range .spec.taints[*]}{.key}{"="}{.effect}{"\n"}{end}{end}'"'"' \
          | grep -Eq "^node-role\\.kubernetes\\.io/(control-plane|master)=NoSchedule$"; then
          echo "vind cluster nodes remain NoSchedule-tainted after normalization" >&2
          kubectl get nodes -o jsonpath='"'"'{range .items[*]}{.metadata.name}{"\t"}{range .spec.taints[*]}{.key}{"="}{.effect}{","}{end}{"\n"}{end}'"'"' >&2 || true
          exit 1
        fi
      fi
      echo "[vind] node taints after normalization:" >&2
      kubectl get nodes -o jsonpath='"'"'{range .items[*]}{.metadata.name}{"\t"}{range .spec.taints[*]}{.key}{"="}{.effect}{","}{end}{"\n"}{end}'"'"' 2>/dev/null >&2 || true
      kubectl config current-context
    '
}

create_cluster() {
  use_driver

  local create_log
  create_log="$(mktemp "${TMPDIR:-/tmp}/${CLUSTER_NAME}.vcluster-create.XXXXXX")"

  if vcluster create "${CLUSTER_NAME}" --driver docker --connect=false >"${create_log}" 2>&1; then
    rm -f "${create_log}"
    return 0
  fi

  cat "${create_log}" >&2
  if grep -Fq 'failed to inspect test network container' "${create_log}" \
    && grep -Fq "requested image's platform" "${create_log}"; then
    rm -f "${create_log}"
    if [[ "${VIND_ALLOW_K3D_FALLBACK}" != "1" ]]; then
      cat >&2 <<'EOF'
vind create failed due to the known docker-driver platform warning bug.
Fix the local probe image first:
  docker pull --platform linux/arm64 alpine:latest
If you still want the legacy fallback behavior for this run, re-run with:
  VIND_ALLOW_K3D_FALLBACK=1
EOF
      exit 1
    fi
    have_k3d || {
      echo "vcluster create hit the known docker-driver network bug and k3d is not installed for fallback" >&2
      exit 1
    }
    echo "[vind] vcluster docker driver failed; using opt-in k3d fallback" >&2
    k3d cluster create "${CLUSTER_NAME}" --wait >/dev/null
    return 0
  fi

  rm -f "${create_log}"
  return 1
}

ensure_context() {
  ensure_colima_kernel_modules
  ensure_arm64_probe_image

  if [[ "${RESET_CLUSTER}" == "1" ]]; then
    delete_cluster
  fi

  if cluster_exists; then
    connect_cluster
  else
    create_cluster
    connect_cluster
  fi
}

main() {
  local command="${1:-}"

  case "${command}" in
    ensure-context)
      ensure_context
      ;;
    delete)
      if [[ "${KEEP_CLUSTER}" != "1" ]]; then
        delete_cluster
      fi
      ;;
    preflight)
      ensure_colima_kernel_modules
      ensure_arm64_probe_image
      ;;
    *)
      usage
      ;;
  esac
}

main "$@"
