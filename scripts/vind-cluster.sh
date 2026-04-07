#!/usr/bin/env bash
set -euo pipefail

CLUSTER_NAME="${CLUSTER_NAME:-certd-ci}"
RESET_CLUSTER="${RESET_CLUSTER:-0}"
KEEP_CLUSTER="${KEEP_CLUSTER:-0}"
VIND_COLIMA_AUTOFIX="${VIND_COLIMA_AUTOFIX:-1}"
VIND_UNTAINT_CONTROL_PLANE_NODES="${VIND_UNTAINT_CONTROL_PLANE_NODES:-1}"

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
  vcluster use driver docker >/dev/null
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

  vcluster create "${CLUSTER_NAME}" --driver docker --connect=false >/dev/null
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
      ;;
    *)
      usage
      ;;
  esac
}

main "$@"
