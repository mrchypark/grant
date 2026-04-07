#!/usr/bin/env bash
set -euo pipefail

CLUSTER_NAME="${CLUSTER_NAME:-certd-ci}"
VIND_DRIVER="${VIND_DRIVER:-docker}"
RESET_CLUSTER="${RESET_CLUSTER:-0}"
KEEP_CLUSTER="${KEEP_CLUSTER:-0}"
VIND_COLIMA_AUTOFIX="${VIND_COLIMA_AUTOFIX:-1}"

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

use_driver() {
  vcluster use driver "${VIND_DRIVER}" >/dev/null
}

cluster_exists() {
  docker inspect "$(cluster_container_name)" >/dev/null 2>&1
}

delete_cluster() {
  use_driver

  if cluster_exists; then
    vcluster delete "${CLUSTER_NAME}" >/dev/null
  fi
}

connect_cluster() {
  use_driver
  vcluster connect "${CLUSTER_NAME}" --driver "${VIND_DRIVER}" >/dev/null
}

create_cluster() {
  use_driver

  vcluster create "${CLUSTER_NAME}" --driver "${VIND_DRIVER}" >/dev/null
}

ensure_context() {
  ensure_colima_kernel_modules

  if [[ "${RESET_CLUSTER}" == "1" ]]; then
    delete_cluster
  fi

  if cluster_exists; then
    connect_cluster
  else
    create_cluster
  fi

  kubectl wait --for=condition=Ready nodes --all --timeout=120s >/dev/null
  kubectl config current-context
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
      ;;
    *)
      usage
      ;;
  esac
}

main "$@"
