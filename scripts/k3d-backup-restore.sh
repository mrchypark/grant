#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
NAMESPACE="${NAMESPACE:-certd}"
LOCAL_PORT="${LOCAL_PORT:-18080}"
FACILITY_CODE="${FACILITY_CODE:-PV15-R-090795}"
CERTD_STABILIZE_SECONDS="${CERTD_STABILIZE_SECONDS:-60}"

need() {
  command -v "$1" >/dev/null 2>&1 || {
    echo "missing command: $1" >&2
    exit 1
  }
}

need kubectl
need curl
need jq

pf_pid=""
cleanup() {
  if [[ -n "${pf_pid}" ]]; then
    kill "${pf_pid}" >/dev/null 2>&1 || true
  fi
}
trap cleanup EXIT

stop_port_forward() {
  if [[ -n "${pf_pid}" ]]; then
    kill "${pf_pid}" >/dev/null 2>&1 || true
    pf_pid=""
  fi
}

start_port_forward() {
  stop_port_forward
  kubectl -n "${NAMESPACE}" port-forward svc/certd "${LOCAL_PORT}:80" >/tmp/certd-port-forward.log 2>&1 &
  pf_pid=$!
  for _ in $(seq 1 90); do
    if curl -fsS "http://127.0.0.1:${LOCAL_PORT}/healthz" >/dev/null 2>&1; then
      return 0
    fi
    sleep 2
  done
  echo "certd service did not become healthy on localhost:${LOCAL_PORT}" >&2
  return 1
}

ensure_backup_bucket() {
  kubectl -n "${NAMESPACE}" delete job minio-bootstrap-backups --ignore-not-found >/dev/null 2>&1 || true
  cat <<EOF | kubectl apply -f - >/dev/null
apiVersion: batch/v1
kind: Job
metadata:
  name: minio-bootstrap-backups
  namespace: ${NAMESPACE}
spec:
  backoffLimit: 6
  template:
    spec:
      restartPolicy: OnFailure
      containers:
        - name: mc
          image: minio/mc:latest
          env:
            - name: MINIO_ROOT_USER
              valueFrom:
                secretKeyRef:
                  name: minio-secrets
                  key: MINIO_ROOT_USER
            - name: MINIO_ROOT_PASSWORD
              valueFrom:
                secretKeyRef:
                  name: minio-secrets
                  key: MINIO_ROOT_PASSWORD
          command: ["/bin/sh", "-lc"]
          args:
            - |
              set -e
              until mc alias set local http://minio.${NAMESPACE}.svc.cluster.local:9000 "\$MINIO_ROOT_USER" "\$MINIO_ROOT_PASSWORD"; do
                sleep 2
              done
              mc mb -p local/cert-backups || true
EOF
  kubectl -n "${NAMESPACE}" wait --for=condition=complete job/minio-bootstrap-backups --timeout=10m >/dev/null
}

ensure_backup_bucket
start_port_forward

curl -sS -X POST "http://127.0.0.1:${LOCAL_PORT}/api/v1/admin/backups"

ORIGINAL_REPLICAS="$(
  kubectl -n "${NAMESPACE}" get statefulset certd -o jsonpath='{.spec.replicas}'
)"
if [[ -z "${ORIGINAL_REPLICAS}" ]]; then
  echo "failed to determine current certd replica count" >&2
  exit 1
fi

BACKUP_NAME=""
for _ in $(seq 1 30); do
  BACKUP_NAME="$(
    curl -fsS "http://127.0.0.1:${LOCAL_PORT}/api/v1/admin/backups" \
      | jq -r '.items | sort_by(.lastModified) | last | .name // empty'
  )"
  if [[ -n "${BACKUP_NAME}" ]]; then
    break
  fi
  sleep 2
done

if [[ -z "${BACKUP_NAME}" ]]; then
  echo "backup object not found in cert-backups bucket" >&2
  exit 1
fi

stop_port_forward

kubectl -n "${NAMESPACE}" scale statefulset/certd --replicas=0
kubectl -n "${NAMESPACE}" wait --for=delete pod -l app=certd --timeout=10m

for ordinal in $(seq 0 $((ORIGINAL_REPLICAS - 1))); do
  kubectl -n "${NAMESPACE}" delete pvc "data-certd-${ordinal}" --ignore-not-found=true
done

kubectl -n "${NAMESPACE}" set env statefulset/certd HQL_BACKUP_RESTORE="s3:${BACKUP_NAME}"
kubectl -n "${NAMESPACE}" scale statefulset/certd --replicas="${ORIGINAL_REPLICAS}"
kubectl -n "${NAMESPACE}" rollout status statefulset/certd --timeout=15m
kubectl -n "${NAMESPACE}" set env statefulset/certd HQL_BACKUP_RESTORE-

sleep "${CERTD_STABILIZE_SECONDS}"
start_port_forward

curl -sS "http://127.0.0.1:${LOCAL_PORT}/api/v1/plants/${FACILITY_CODE}" | jq
DOC_ID="$(curl -sS "http://127.0.0.1:${LOCAL_PORT}/api/v1/plants/${FACILITY_CODE}/documents" | jq -r '.[0].id')"
curl -sSI "http://127.0.0.1:${LOCAL_PORT}/api/v1/documents/${DOC_ID}/download"
echo "keploy replay after restore..."
stop_port_forward
time "${ROOT_DIR}/scripts/keploy-k8s-test.sh"

echo "backup/restore completed with ${BACKUP_NAME}"
