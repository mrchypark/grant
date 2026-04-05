#!/usr/bin/env bash
set -euo pipefail

NAMESPACE="${NAMESPACE:-certd}"
TRAILBASE_SELECTOR="${TRAILBASE_SELECTOR:-app=trailbase}"
TRAILBASE_CONTAINER="${TRAILBASE_CONTAINER:-trailbase}"
TRAILBASE_DATA_DIR="${TRAILBASE_DATA_DIR:-/app/traildepot}"
TRAILBASE_USER_EMAIL="${TRAILBASE_USER_EMAIL:-trailbase-user@example.com}"
TRAILBASE_USER_PASSWORD="${TRAILBASE_USER_PASSWORD:-secret}"

need() {
  command -v "$1" >/dev/null 2>&1 || {
    echo "missing command: $1" >&2
    exit 1
  }
}

need kubectl

kubectl -n "${NAMESPACE}" wait --for=condition=Ready pod -l "${TRAILBASE_SELECTOR}" --timeout=180s >/dev/null

TRAILBASE_POD="$(
  kubectl -n "${NAMESPACE}" get pods -l "${TRAILBASE_SELECTOR}" \
    -o jsonpath='{.items[?(@.status.phase=="Running")].metadata.name}' \
    | awk '{print $1}'
)"

if [[ -z "${TRAILBASE_POD}" ]]; then
  echo "no running trailbase pod found for selector ${TRAILBASE_SELECTOR}" >&2
  exit 1
fi

set +e
seed_output="$(
  kubectl -n "${NAMESPACE}" exec "${TRAILBASE_POD}" -c "${TRAILBASE_CONTAINER}" -- \
    trailbase-auth-bridge seed-user \
    --data-dir "${TRAILBASE_DATA_DIR}" \
    --email "${TRAILBASE_USER_EMAIL}" \
    --password "${TRAILBASE_USER_PASSWORD}" 2>&1
)"
seed_status=$?
set -e

if [[ "${seed_status}" -ne 0 ]]; then
  if grep -qi "unique" <<<"${seed_output}"; then
    echo "TrailBase user ${TRAILBASE_USER_EMAIL} already exists in pod ${TRAILBASE_POD}" >&2
    exit 0
  fi
  printf '%s\n' "${seed_output}" >&2
  exit "${seed_status}"
fi

echo "seeded TrailBase user ${TRAILBASE_USER_EMAIL} in pod ${TRAILBASE_POD}"
