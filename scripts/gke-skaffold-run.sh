#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
KUBE_CONTEXT="${KUBE_CONTEXT:-gke_ied-cluster}"
NAMESPACE="${NAMESPACE:-grant-chaos}"
SKAFFOLD_PROFILE="${SKAFFOLD_PROFILE:-ied}"
SKAFFOLD_MODULE="${SKAFFOLD_MODULE:-grant}"
SKAFFOLD_EXTRA_ARGS="${SKAFFOLD_EXTRA_ARGS:-}"
DEFAULT_RAUTHY_IMAGE="$(sed -n 's/^[[:space:]]*image:[[:space:]]*\(ghcr.io\/conalog\/grant-rauthy-launcher[^[:space:]]*\).*/\1/p' "${ROOT_DIR}/deploy/k8s/rauthy-statefulset.yaml")"

if [[ -n "${RAUTHY_IMAGE:-}" && "${RAUTHY_IMAGE}" != "${DEFAULT_RAUTHY_IMAGE}" ]]; then
  echo "the GKE Skaffold deployment uses the Rauthy launcher image pinned in deploy/k8s/rauthy-statefulset.yaml; RAUTHY_IMAGE cannot override it" >&2
  exit 1
fi
RAUTHY_IMAGE="${DEFAULT_RAUTHY_IMAGE}"
if [[ "${RAUTHY_IMAGE}" == *RAUTHY_LAUNCHER_IMAGE_DIGEST* ]]; then
  echo "Rauthy launcher digest is unpublished; refusing GKE deployment before Docker or cluster access" >&2
  exit 1
fi
if ! [[ "${RAUTHY_IMAGE}" =~ ^ghcr\.io/conalog/grant-rauthy-launcher:[^@[:space:]]+@sha256:[a-f0-9]{64}$ ]]; then
  echo "cannot determine an immutable private Rauthy launcher image from deploy/k8s/rauthy-statefulset.yaml" >&2
  exit 1
fi
export RAUTHY_IMAGE

GRANT_MASTER_KEY="${GRANT_MASTER_KEY:-local-dev-master-key}"
GRANT_APP_AUTH_KEY="${GRANT_APP_AUTH_KEY:-local-dev-app-auth-key}"
HQL_SECRET_RAFT="${HQL_SECRET_RAFT:-SuperSecureSecret1337}"
HQL_SECRET_API="${HQL_SECRET_API:-AnotherSecureSecret42}"
GRANT_RAUTHY_CLIENT_ID="${GRANT_RAUTHY_CLIENT_ID:-grant}"
GRANT_RAUTHY_CLIENT_SECRET="${GRANT_RAUTHY_CLIENT_SECRET:-devRauthyClientSecret0000000000000000000000000000000000000000000}"
GRANT_RAUTHY_CLIENT_READ_API_KEY="${GRANT_RAUTHY_CLIENT_READ_API_KEY:-grant-client-read\$devRauthyClientReadApiKeySecret00000000000000000000000000000000000}"
ENC_KEYS="${ENC_KEYS:-devkey01/MDEyMzQ1Njc4OWFiY2RlZjAxMjM0NTY3ODlhYmNkZWY=}"
ENC_KEY_ACTIVE="${ENC_KEY_ACTIVE:-devkey01}"

# shellcheck source=scripts/k8s-secret-helpers.sh
. "${ROOT_DIR}/scripts/k8s-secret-helpers.sh"

need() {
  command -v "$1" >/dev/null 2>&1 || {
    echo "missing command: $1" >&2
    exit 1
  }
}

need kubectl
need skaffold
need docker

if [[ "${SKAFFOLD_PROFILE}" == "ied" && "${NAMESPACE}" != "grant-chaos" ]]; then
  echo "the ied Skaffold profile deploys the grant-chaos overlay; set NAMESPACE=grant-chaos or add a dedicated overlay" >&2
  exit 1
fi

SKAFFOLD_PID=

cleanup_registry_config() {
  unset GHCR_TOKEN GHCR_USERNAME
  if [[ -n "${RAUTHY_REGISTRY_CONFIG_DIR:-}" ]]; then
    rm -rf "${RAUTHY_REGISTRY_CONFIG_DIR}"
    unset RAUTHY_REGISTRY_CONFIG_DIR
  fi
}

# shellcheck disable=SC2317,SC2329 # invoked by EXIT trap
exit_with_cleanup() {
  local exit_code="$1"
  trap - EXIT HUP INT TERM
  cleanup_registry_config
  exit "${exit_code}"
}

# shellcheck disable=SC2317,SC2329 # invoked by signal traps
forward_signal_and_exit() {
  local signal_name="$1"
  local exit_code="$2"
  local child_pid="${SKAFFOLD_PID}"
  local grace_attempts=0
  trap - EXIT HUP INT TERM
  cleanup_registry_config
  if [[ -n "${child_pid}" ]] && kill -0 "${child_pid}" 2>/dev/null; then
    kill -s "${signal_name}" "${child_pid}" 2>/dev/null || true
    while kill -0 "${child_pid}" 2>/dev/null && [[ "${grace_attempts}" -lt 20 ]]; do
      sleep 0.1
      grace_attempts=$((grace_attempts + 1))
    done
    if kill -0 "${child_pid}" 2>/dev/null; then
      kill -KILL "${child_pid}" 2>/dev/null || true
    fi
    wait "${child_pid}" 2>/dev/null || true
  fi
  SKAFFOLD_PID=
  exit "${exit_code}"
}

trap 'exit_with_cleanup "$?"' EXIT
trap 'forward_signal_and_exit HUP 129' HUP
trap 'forward_signal_and_exit INT 130' INT
trap 'forward_signal_and_exit TERM 143' TERM

# Authenticate before mutating the cluster so a clean deployment cannot begin
# without credentials for the private launcher image required by the manifest.
prepare_rauthy_registry_auth
unset GHCR_TOKEN GHCR_USERNAME

kubectl config use-context "${KUBE_CONTEXT}" >/dev/null
kubectl --context "${KUBE_CONTEXT}" create namespace "${NAMESPACE}" --dry-run=client -o yaml \
  | kubectl --context "${KUBE_CONTEXT}" apply -f -
apply_rauthy_registry_pull_secret
cleanup_registry_config
apply_grant_secret

extra_args=()
if [[ -n "${SKAFFOLD_EXTRA_ARGS}" ]]; then
  read -r -a extra_args <<<"${SKAFFOLD_EXTRA_ARGS}"
fi

# Bash otherwise starts asynchronous commands with SIGINT ignored when job
# control is disabled. Enable it only for the launch so forwarded signals retain
# their normal disposition in the tracked Skaffold process.
set -m
if [[ -n "${SKAFFOLD_EXTRA_ARGS}" ]]; then
  skaffold run \
    -f "${ROOT_DIR}/skaffold.yaml" \
    --module "${SKAFFOLD_MODULE}" \
    --profile "${SKAFFOLD_PROFILE}" \
    --kube-context "${KUBE_CONTEXT}" \
    "${extra_args[@]}" &
else
  skaffold run \
    -f "${ROOT_DIR}/skaffold.yaml" \
    --module "${SKAFFOLD_MODULE}" \
    --profile "${SKAFFOLD_PROFILE}" \
    --kube-context "${KUBE_CONTEXT}" &
fi
SKAFFOLD_PID=$!
set +m

if wait "${SKAFFOLD_PID}"; then
  skaffold_status=0
else
  skaffold_status=$?
fi
SKAFFOLD_PID=
exit "${skaffold_status}"
