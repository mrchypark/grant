#!/usr/bin/env bash
set -euo pipefail

CLUSTER_NAME="${CLUSTER_NAME:-grant-ci}"
RESET_CLUSTER="${RESET_CLUSTER:-0}"
KEEP_CLUSTER="${KEEP_CLUSTER:-0}"
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

use_driver() {
  vcluster use driver docker >/dev/null
}

vcluster_cluster_exists() {
  docker inspect "$(cluster_container_name)" >/dev/null 2>&1
}

delete_cluster() {
  if vcluster_cluster_exists; then
    use_driver
    vcluster delete "${CLUSTER_NAME}" >/dev/null 2>&1 || true
  fi
  docker rm -f "$(cluster_container_name)" >/dev/null 2>&1 || true
  docker network rm "vcluster.${CLUSTER_NAME}" >/dev/null 2>&1 || true
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
    cat >&2 <<'EOF'
vind create failed due to the known docker-driver platform warning bug.
Fix the local probe image first:
  docker pull --platform linux/arm64 alpine:latest
EOF
    exit 1
  fi

  rm -f "${create_log}"
  return 1
}

ensure_context() {
  if [[ "${RESET_CLUSTER}" == "1" ]]; then
    delete_cluster
  fi

  if vcluster_cluster_exists; then
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
      docker version >/dev/null
      vcluster version >/dev/null
      ;;
    *)
      usage
      ;;
  esac
}

main "$@"
