#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
KUBE_CONTEXT="${KUBE_CONTEXT:-gke_ied-cluster}"
NAMESPACE="${NAMESPACE:-certd-chaos}"
LOCAL_PORT="${LOCAL_PORT:-18080}"
TRAILBASE_USER_EMAIL="${TRAILBASE_USER_EMAIL:-trailbase-user@example.com}"
TRAILBASE_USER_PASSWORD="${TRAILBASE_USER_PASSWORD:-secret}"
BOOTSTRAP_ORGANIZATION_ID="${BOOTSTRAP_ORGANIZATION_ID:-conalog-account}"
EXPECTED_STRONG_ERROR="${EXPECTED_STRONG_ERROR:-unavailable: strong consistency requested but quorum-backed identity sync is unavailable}"
EXPECTED_STRONG_STATUS="${EXPECTED_STRONG_STATUS:-503}"
PARTITION_MANIFEST="${PARTITION_MANIFEST:-$ROOT_DIR/deploy/chaos-mesh/network/learner-1-to-voters-partition.yaml}"
PARTITION_RESOURCE_NAME="${PARTITION_RESOURCE_NAME:-learner-1-to-voters-partition}"
LEARNER_SELECTOR="${LEARNER_SELECTOR:-app=certd,role=learner,instance=learner-1}"
UNAFFECTED_LEARNER_SELECTOR="${UNAFFECTED_LEARNER_SELECTOR:-app=certd,role=learner,instance=learner-2}"
LEARNER_DEPLOYMENT="${LEARNER_DEPLOYMENT:-certd-learner-1}"
HTTP_CURL_TIMEOUT_SECONDS="${HTTP_CURL_TIMEOUT_SECONDS:-10}"

need() {
  command -v "$1" >/dev/null 2>&1 || {
    echo "missing command: $1" >&2
    exit 1
  }
}

need kubectl
need curl
need jq

if [[ "${KUBE_CONTEXT}" != "$(kubectl config current-context 2>/dev/null || true)" ]]; then
  kubectl config use-context "${KUBE_CONTEXT}" >/dev/null
fi

wait_for_http() {
  local url="$1"
  local attempts="${2:-30}"
  local delay="${3:-1}"
  for _ in $(seq 1 "${attempts}"); do
    if curl -fsS --max-time "${HTTP_CURL_TIMEOUT_SECONDS}" "${url}" >/dev/null 2>&1; then
      return 0
    fi
    sleep "${delay}"
  done
  echo "timed out waiting for ${url}" >&2
  exit 1
}

json_field_equals() {
  local json="$1"
  local jq_expr="$2"
  local expected="$3"
  local actual
  actual="$(jq -r "${jq_expr}" <<<"${json}")"
  [[ "${actual}" == "${expected}" ]]
}

learner_pod() {
  kubectl --context "${KUBE_CONTEXT}" -n "${NAMESPACE}" get pod -l "${LEARNER_SELECTOR}" -o jsonpath='{.items[0].metadata.name}'
}

unaffected_learner_pod() {
  kubectl --context "${KUBE_CONTEXT}" -n "${NAMESPACE}" get pod -l "${UNAFFECTED_LEARNER_SELECTOR}" -o jsonpath='{.items[0].metadata.name}'
}

port_forward_log="$(mktemp)"
pf_pid=""

cleanup() {
  kubectl --context "${KUBE_CONTEXT}" -n "${NAMESPACE}" delete networkchaos "${PARTITION_RESOURCE_NAME}" --ignore-not-found >/dev/null 2>&1 || true
  if [[ -n "${pf_pid}" ]]; then
    kill "${pf_pid}" >/dev/null 2>&1 || true
  fi
  rm -f "${port_forward_log}"
}
trap cleanup EXIT

kubectl --context "${KUBE_CONTEXT}" -n "${NAMESPACE}" port-forward svc/certd "${LOCAL_PORT}:80" >"${port_forward_log}" 2>&1 &
pf_pid=$!
wait_for_http "http://127.0.0.1:${LOCAL_PORT}/healthz"

auth_token="$(
  curl -fsS --max-time "${HTTP_CURL_TIMEOUT_SECONDS}" -X POST \
    "http://127.0.0.1:${LOCAL_PORT}/api/auth/v1/login" \
    -H 'content-type: application/json' \
    --data-binary "$(jq -nc --arg email "${TRAILBASE_USER_EMAIL}" --arg password "${TRAILBASE_USER_PASSWORD}" '{email: $email, password: $password}')" \
    | jq -r '.auth_token // .authToken // empty'
)"
if [[ -z "${auth_token}" || "${auth_token}" == "null" ]]; then
  echo "failed to obtain TrailBase auth token" >&2
  exit 1
fi

pod="$(learner_pod)"
unaffected_pod="$(unaffected_learner_pod)"
refresh_learner_pod() {
  pod="$(learner_pod)"
}

restart_learner_under_partition() {
  kubectl --context "${KUBE_CONTEXT}" -n "${NAMESPACE}" delete pod "${pod}" >/dev/null
  kubectl --context "${KUBE_CONTEXT}" -n "${NAMESPACE}" rollout status "deployment/${LEARNER_DEPLOYMENT}" --timeout=5m >/dev/null
  refresh_learner_pod
}

api_call() {
  local path="$1"
  kubectl --context "${KUBE_CONTEXT}" -n "${NAMESPACE}" exec "${pod}" -- sh -lc \
    "curl -sS --max-time ${HTTP_CURL_TIMEOUT_SECONDS} -w '\nHTTP %{http_code}\n' \
      -H 'authorization: Bearer ${auth_token}' \
      -H 'x-organization-id: ${BOOTSTRAP_ORGANIZATION_ID}' \
      'http://127.0.0.1:8080${path}'"
}

healthz_json() {
  kubectl --context "${KUBE_CONTEXT}" -n "${NAMESPACE}" exec "${pod}" -- sh -lc \
    "curl -sS --max-time ${HTTP_CURL_TIMEOUT_SECONDS} http://127.0.0.1:8080/healthz"
}

healthz_json_for_pod() {
  local target_pod="$1"
  kubectl --context "${KUBE_CONTEXT}" -n "${NAMESPACE}" exec "${target_pod}" -- sh -lc \
    "curl -sS --max-time ${HTTP_CURL_TIMEOUT_SECONDS} http://127.0.0.1:8080/healthz"
}

split_http_body() {
  sed '$d'
}

extract_http_code() {
  tail -n1 | awk '{print $2}'
}

echo "partition probe: learner pod=${pod}"
echo "partition probe: unaffected learner pod=${unaffected_pod}"

before_healthz="$(healthz_json)"
echo "before healthz: ${before_healthz}"
if ! json_field_equals "${before_healthz}" '.writeReady' 'true'; then
  echo "expected learner writeReady=true before partition" >&2
  exit 1
fi

for _ in $(seq 1 20); do
  if before_local="$(api_call '/api/v1/me' 2>/dev/null)" \
    && [[ "$(printf '%s\n' "${before_local}" | extract_http_code)" == "200" ]]; then
    break
  fi
  sleep 1
done
if [[ "$(printf '%s\n' "${before_local}" | extract_http_code)" != "200" ]]; then
  echo "expected local read to succeed before partition" >&2
  printf '%s\n' "${before_local}" >&2
  exit 1
fi

for _ in $(seq 1 20); do
  if before_strong="$(api_call '/api/v1/me?consistency=strong' 2>/dev/null)" \
    && [[ "$(printf '%s\n' "${before_strong}" | extract_http_code)" == "200" ]]; then
    break
  fi
  sleep 1
done
if [[ "$(printf '%s\n' "${before_strong}" | extract_http_code)" != "200" ]]; then
  echo "expected strong read to succeed before partition" >&2
  printf '%s\n' "${before_strong}" >&2
  exit 1
fi

kubectl --context "${KUBE_CONTEXT}" -n "${NAMESPACE}" apply -f "${PARTITION_MANIFEST}" >/dev/null
for _ in $(seq 1 20); do
  injected="$(
    kubectl --context "${KUBE_CONTEXT}" -n "${NAMESPACE}" get networkchaos "${PARTITION_RESOURCE_NAME}" \
      -o jsonpath='{.status.conditions[?(@.type=="AllInjected")].status}' 2>/dev/null || true
  )"
  [[ "${injected}" == "True" ]] && break
  sleep 1
done
if during_healthz="$(healthz_json 2>/dev/null)"; then
  echo "during healthz: ${during_healthz}"
else
  during_healthz=""
  echo "during healthz: timeout" >&2
fi

for _ in $(seq 1 20); do
  if during_local="$(api_call '/api/v1/me' 2>/dev/null)" \
    && [[ "$(printf '%s\n' "${during_local}" | extract_http_code)" == "200" ]]; then
    break
  fi
  sleep 1
done
if [[ "$(printf '%s\n' "${during_local}" | extract_http_code)" != "200" ]]; then
  echo "expected local read to succeed during partition" >&2
  printf '%s\n' "${during_local}" >&2
  exit 1
fi

for _ in $(seq 1 20); do
  if during_strong="$(api_call '/api/v1/me?consistency=strong' 2>/dev/null)"; then
    during_strong_code="$(printf '%s\n' "${during_strong}" | extract_http_code)"
    during_strong_body="$(printf '%s\n' "${during_strong}" | split_http_body)"
  else
    during_strong_code=""
    during_strong_body=""
  fi
  if [[ "${during_strong_code}" == "${EXPECTED_STRONG_STATUS}" ]]; then
    break
  fi
  sleep 1
done
if [[ "${during_strong_code}" != "${EXPECTED_STRONG_STATUS}" ]]; then
  echo "during strong stayed available; restarting learner under active partition" >&2
  restart_learner_under_partition
  if during_healthz="$(healthz_json 2>/dev/null)"; then
    echo "post-restart during healthz: ${during_healthz}"
  else
    echo "post-restart during healthz: timeout" >&2
  fi
  for _ in $(seq 1 20); do
    if during_local="$(api_call '/api/v1/me' 2>/dev/null)" \
      && [[ "$(printf '%s\n' "${during_local}" | extract_http_code)" == "200" ]]; then
      break
    fi
    sleep 1
  done
  for _ in $(seq 1 20); do
    if during_strong="$(api_call '/api/v1/me?consistency=strong' 2>/dev/null)"; then
      during_strong_code="$(printf '%s\n' "${during_strong}" | extract_http_code)"
      during_strong_body="$(printf '%s\n' "${during_strong}" | split_http_body)"
    else
      during_strong_code=""
      during_strong_body=""
    fi
    if [[ "${during_strong_code}" == "${EXPECTED_STRONG_STATUS}" ]]; then
      break
    fi
    sleep 1
  done
fi
echo "during strong response: ${during_strong_body}"
if [[ "${during_strong_code}" != "${EXPECTED_STRONG_STATUS}" ]]; then
  echo "expected strong read to fail with ${EXPECTED_STRONG_STATUS} during partition" >&2
  printf '%s\n' "${during_strong}" >&2
  exit 1
fi
if ! jq -e --arg expected "${EXPECTED_STRONG_ERROR}" '.error == $expected' >/dev/null <<<"${during_strong_body}"; then
  echo "unexpected strong read error body during partition" >&2
  printf '%s\n' "${during_strong}" >&2
  exit 1
fi

for _ in $(seq 1 20); do
  if during_plants_strong="$(api_call '/api/v1/plants?consistency=strong' 2>/dev/null)"; then
    during_plants_code="$(printf '%s\n' "${during_plants_strong}" | extract_http_code)"
    during_plants_body="$(printf '%s\n' "${during_plants_strong}" | split_http_body)"
  else
    during_plants_code=""
    during_plants_body=""
  fi
  if [[ "${during_plants_code}" == "${EXPECTED_STRONG_STATUS}" ]]; then
    break
  fi
  sleep 1
done
echo "during plants strong response: ${during_plants_body}"
if [[ "${during_plants_code}" != "${EXPECTED_STRONG_STATUS}" ]]; then
  echo "expected strong plant list to fail with ${EXPECTED_STRONG_STATUS} during partition" >&2
  printf '%s\n' "${during_plants_strong}" >&2
  exit 1
fi
if ! jq -e --arg expected "${EXPECTED_STRONG_ERROR}" '.error == $expected' >/dev/null <<<"${during_plants_body}"; then
  echo "unexpected strong plant list error body during partition" >&2
  printf '%s\n' "${during_plants_strong}" >&2
  exit 1
fi

unaffected_healthz="$(healthz_json_for_pod "${unaffected_pod}")"
echo "unaffected learner healthz: ${unaffected_healthz}"
if ! json_field_equals "${unaffected_healthz}" '.writeReady' 'true'; then
  echo "expected unaffected learner writeReady=true during partition" >&2
  exit 1
fi

kubectl --context "${KUBE_CONTEXT}" -n "${NAMESPACE}" delete networkchaos "${PARTITION_RESOURCE_NAME}" >/dev/null
for _ in $(seq 1 30); do
  if after_healthz="$(healthz_json 2>/dev/null)" \
    && json_field_equals "${after_healthz}" '.writeReady' 'true'; then
    break
  fi
  sleep 1
done
echo "after healthz: ${after_healthz}"
if ! json_field_equals "${after_healthz}" '.writeReady' 'true'; then
  echo "expected learner writeReady=true after partition recovery" >&2
  exit 1
fi

if ! after_strong="$(api_call '/api/v1/me?consistency=strong' 2>/dev/null)"; then
  echo "expected strong read to recover after partition removal" >&2
  exit 1
fi
if [[ "$(printf '%s\n' "${after_strong}" | extract_http_code)" != "200" ]]; then
  echo "expected strong read to recover after partition removal" >&2
  printf '%s\n' "${after_strong}" >&2
  exit 1
fi

echo "gke learner-voter partition probe completed"
