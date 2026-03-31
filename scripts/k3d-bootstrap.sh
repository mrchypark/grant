#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
CLUSTER_NAME="${CLUSTER_NAME:-certd-k3d}"
NAMESPACE="${NAMESPACE:-certd}"
IMAGE_NAME="${IMAGE_NAME:-certd:local}"
AGENTS="${AGENTS:-5}"
KUSTOMIZE_DIR="${KUSTOMIZE_DIR:-$ROOT_DIR/deploy/k8s}"

need() {
  command -v "$1" >/dev/null 2>&1 || {
    echo "missing command: $1" >&2
    exit 1
  }
}

need docker
need k3d
need kubectl
need kustomize

CERTD_MASTER_KEY="${CERTD_MASTER_KEY:-local-dev-master-key}"
HQL_SECRET_RAFT="${HQL_SECRET_RAFT:-SuperSecureSecret1337}"
HQL_SECRET_API="${HQL_SECRET_API:-AnotherSecureSecret42}"
ENC_KEYS="${ENC_KEYS:-devkey01/MDEyMzQ1Njc4OWFiY2RlZjAxMjM0NTY3ODlhYmNkZWY=}"
ENC_KEY_ACTIVE="${ENC_KEY_ACTIVE:-devkey01}"
MINIO_ROOT_USER="${MINIO_ROOT_USER:-minioadmin}"
MINIO_ROOT_PASSWORD="${MINIO_ROOT_PASSWORD:-minioadmin}"

if ! k3d cluster get "${CLUSTER_NAME}" >/dev/null 2>&1; then
  k3d cluster create "${CLUSTER_NAME}" \
    --servers 1 \
    --agents "${AGENTS}" \
    --wait
fi

docker image inspect "${IMAGE_NAME}" >/dev/null 2>&1 || {
  echo "missing image ${IMAGE_NAME}. run: docker build -t ${IMAGE_NAME} ." >&2
  exit 1
}

k3d image import "${IMAGE_NAME}" -c "${CLUSTER_NAME}"

kubectl create namespace "${NAMESPACE}" --dry-run=client -o yaml | kubectl apply -f -

kubectl -n "${NAMESPACE}" create secret generic certd-secrets \
  --from-literal=CERTD_MASTER_KEY="${CERTD_MASTER_KEY}" \
  --from-literal=HQL_SECRET_RAFT="${HQL_SECRET_RAFT}" \
  --from-literal=HQL_SECRET_API="${HQL_SECRET_API}" \
  --from-literal=ENC_KEYS="${ENC_KEYS}" \
  --from-literal=ENC_KEY_ACTIVE="${ENC_KEY_ACTIVE}" \
  --from-literal=HQL_S3_KEY="${MINIO_ROOT_USER}" \
  --from-literal=HQL_S3_SECRET="${MINIO_ROOT_PASSWORD}" \
  --dry-run=client -o yaml | kubectl apply -f -

kubectl -n "${NAMESPACE}" create secret generic minio-secrets \
  --from-literal=MINIO_ROOT_USER="${MINIO_ROOT_USER}" \
  --from-literal=MINIO_ROOT_PASSWORD="${MINIO_ROOT_PASSWORD}" \
  --dry-run=client -o yaml | kubectl apply -f -

kustomize build "${KUSTOMIZE_DIR}" | kubectl apply -f -

kubectl -n "${NAMESPACE}" rollout status statefulset/minio --timeout=15m
kubectl -n "${NAMESPACE}" rollout status statefulset/certd --timeout=15m

kubectl -n "${NAMESPACE}" delete job minio-bootstrap --ignore-not-found
cat <<EOF | kubectl apply -f -
apiVersion: batch/v1
kind: Job
metadata:
  name: minio-bootstrap
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
              mc mb -p local/cert-artifacts || true
              mc mb -p local/cert-backups || true
EOF

kubectl -n "${NAMESPACE}" wait --for=condition=complete job/minio-bootstrap --timeout=10m

cat <<EOF
k3d cluster '${CLUSTER_NAME}' is ready.

Namespace: ${NAMESPACE}
Image: ${IMAGE_NAME}

Next:
  ${ROOT_DIR}/scripts/k3d-e2e.sh
  ${ROOT_DIR}/scripts/k3d-chaos.sh
  ${ROOT_DIR}/scripts/k3d-backup-restore.sh
EOF
