#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
CLUSTER_NAME="${CLUSTER_NAME:-certd-k3d}"
NAMESPACE="${NAMESPACE:-certd}"
IMAGE_NAME="${IMAGE_NAME:-certd:local}"
TRAILBASE_IMAGE="${TRAILBASE_IMAGE:-trailbase-auth-bridge:local}"
LITESTREAM_IMAGE="${LITESTREAM_IMAGE:-litestream/litestream:0.3.14}"
AGENTS="${AGENTS:-2}"
KUSTOMIZE_DIR="${KUSTOMIZE_DIR:-$ROOT_DIR/deploy/k8s}"
K3D_NO_LB="${K3D_NO_LB:-0}"
K3D_SKIP_MINIO="${K3D_SKIP_MINIO:-0}"

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
TRAILBASE_CERTD_INTERNAL_SECRET="${TRAILBASE_CERTD_INTERNAL_SECRET:-dev-trailbase-internal-secret}"
ENC_KEYS="${ENC_KEYS:-devkey01/MDEyMzQ1Njc4OWFiY2RlZjAxMjM0NTY3ODlhYmNkZWY=}"
ENC_KEY_ACTIVE="${ENC_KEY_ACTIVE:-devkey01}"
MINIO_ROOT_USER="${MINIO_ROOT_USER:-minioadmin}"
MINIO_ROOT_PASSWORD="${MINIO_ROOT_PASSWORD:-minioadmin}"

if ! k3d cluster get "${CLUSTER_NAME}" >/dev/null 2>&1; then
  if [[ "${K3D_NO_LB}" == "1" ]]; then
    k3d cluster create "${CLUSTER_NAME}" \
      --servers 1 \
      --agents "${AGENTS}" \
      --no-lb \
      --wait
  else
    k3d cluster create "${CLUSTER_NAME}" \
      --servers 1 \
      --agents "${AGENTS}" \
      --wait
  fi
fi

kubectl config use-context "k3d-${CLUSTER_NAME}" >/dev/null

docker image inspect "${IMAGE_NAME}" >/dev/null 2>&1 || {
  echo "missing image ${IMAGE_NAME}. run: docker build -t ${IMAGE_NAME} ." >&2
  exit 1
}

docker image inspect "${TRAILBASE_IMAGE}" >/dev/null 2>&1 || {
  echo "missing image ${TRAILBASE_IMAGE}. run: docker build -f Dockerfile.trailbase-auth-bridge -t ${TRAILBASE_IMAGE} ." >&2
  exit 1
}

docker image inspect "${LITESTREAM_IMAGE}" >/dev/null 2>&1 || {
  echo "missing image ${LITESTREAM_IMAGE}. run: docker pull --platform linux/arm64 ${LITESTREAM_IMAGE}" >&2
  exit 1
}

import_image_with_ctr() {
  local image="$1"
  local tar_file
  tar_file="$(mktemp "/tmp/${CLUSTER_NAME}.image.XXXXXX")"
  docker save "${image}" -o "${tar_file}"

  local nodes
  nodes="$(
    docker ps --format '{{.Names}}' \
      | grep -E "^k3d-${CLUSTER_NAME}-(server|agent)-" || true
  )"
  if [[ -z "${nodes}" ]]; then
    echo "failed to find k3d node containers for ${CLUSTER_NAME}" >&2
    rm -f "${tar_file}"
    exit 1
  fi

  for node in ${nodes}; do
    docker cp "${tar_file}" "${node}:/tmp/k3d-image-import.tar"
    docker exec "${node}" ctr -n k8s.io images import /tmp/k3d-image-import.tar >/dev/null
    docker exec "${node}" rm -f /tmp/k3d-image-import.tar >/dev/null 2>&1 || true
  done

  rm -f "${tar_file}"
}

k3d image import "${IMAGE_NAME}" -c "${CLUSTER_NAME}"
k3d image import "${LITESTREAM_IMAGE}" -c "${CLUSTER_NAME}"
import_image_with_ctr "${TRAILBASE_IMAGE}"

kubectl create namespace "${NAMESPACE}" --dry-run=client -o yaml | kubectl apply -f -

kubectl -n "${NAMESPACE}" create secret generic certd-secrets \
  --from-literal=CERTD_MASTER_KEY="${CERTD_MASTER_KEY}" \
  --from-literal=HQL_SECRET_RAFT="${HQL_SECRET_RAFT}" \
  --from-literal=HQL_SECRET_API="${HQL_SECRET_API}" \
  --from-literal=TRAILBASE_CERTD_INTERNAL_SECRET="${TRAILBASE_CERTD_INTERNAL_SECRET}" \
  --from-literal=ENC_KEYS="${ENC_KEYS}" \
  --from-literal=ENC_KEY_ACTIVE="${ENC_KEY_ACTIVE}" \
  --from-literal=HQL_S3_KEY="${MINIO_ROOT_USER}" \
  --from-literal=HQL_S3_SECRET="${MINIO_ROOT_PASSWORD}" \
  --dry-run=client -o yaml | kubectl apply -f -

kubectl -n "${NAMESPACE}" create secret generic trailbase-backup-secret \
  --from-literal=TRAILBASE_CERTD_INTERNAL_SECRET="${TRAILBASE_CERTD_INTERNAL_SECRET}" \
  --from-literal=LITESTREAM_ACCESS_KEY_ID="${MINIO_ROOT_USER}" \
  --from-literal=LITESTREAM_SECRET_ACCESS_KEY="${MINIO_ROOT_PASSWORD}" \
  --dry-run=client -o yaml | kubectl apply -f -

kubectl -n "${NAMESPACE}" create secret generic minio-secrets \
  --from-literal=MINIO_ROOT_USER="${MINIO_ROOT_USER}" \
  --from-literal=MINIO_ROOT_PASSWORD="${MINIO_ROOT_PASSWORD}" \
  --dry-run=client -o yaml | kubectl apply -f -

apply_resource() {
  kubectl -n "${NAMESPACE}" apply -f "$1"
}

apply_resource "${KUSTOMIZE_DIR}/namespace.yaml"
if [[ "${K3D_SKIP_MINIO}" != "1" ]]; then
  apply_resource "${KUSTOMIZE_DIR}/minio-headless-service.yaml"
  apply_resource "${KUSTOMIZE_DIR}/minio-service.yaml"
  apply_resource "${KUSTOMIZE_DIR}/minio-pdb.yaml"
  apply_resource "${KUSTOMIZE_DIR}/minio-statefulset.yaml"
fi

kubernetes_wait() {
  kubectl -n "${NAMESPACE}" "$@"
}

wait_for_resource() {
  local kind="$1"
  local name="$2"
  local attempts="${3:-60}"
  local delay="${4:-2}"
  for _ in $(seq 1 "${attempts}"); do
    if kubectl -n "${NAMESPACE}" get "${kind}" "${name}" >/dev/null 2>&1; then
      return 0
    fi
    sleep "${delay}"
  done
  echo "timed out waiting for ${kind}/${name} to exist" >&2
  exit 1
}

if [[ "${K3D_SKIP_MINIO}" != "1" ]]; then
  wait_for_resource statefulset minio
  kubernetes_wait rollout status statefulset/minio --timeout=15m

  kubectl -n "${NAMESPACE}" delete job minio-bootstrap --ignore-not-found
  for _ in $(seq 1 60); do
    if ! kubectl -n "${NAMESPACE}" get job minio-bootstrap >/dev/null 2>&1; then
      break
    fi
    sleep 1
  done
  if kubectl -n "${NAMESPACE}" get job minio-bootstrap >/dev/null 2>&1; then
    echo "timed out waiting for minio-bootstrap job deletion" >&2
    exit 1
  fi
  kubectl -n "${NAMESPACE}" apply -f - <<EOF
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
              mc mb -p local/trailbase-backups || true
EOF

  kubectl -n "${NAMESPACE}" wait --for=condition=complete job/minio-bootstrap --timeout=10m
else
  echo "[skip] minio deployment"
fi

apply_resource "${KUSTOMIZE_DIR}/trailbase-configmap.yaml"
apply_resource "${KUSTOMIZE_DIR}/trailbase-service.yaml"
apply_resource "${KUSTOMIZE_DIR}/trailbase-deployment.yaml"

wait_for_resource deployment trailbase
kubernetes_wait rollout status deployment/trailbase --timeout=15m

apply_resource "${KUSTOMIZE_DIR}/app-configmap.yaml"
apply_resource "${KUSTOMIZE_DIR}/app-headless-service.yaml"
apply_resource "${KUSTOMIZE_DIR}/app-service.yaml"
apply_resource "${KUSTOMIZE_DIR}/app-pdb.yaml"
apply_resource "${KUSTOMIZE_DIR}/app-statefulset.yaml"
apply_resource "${KUSTOMIZE_DIR}/app-learner-1-service.yaml"
apply_resource "${KUSTOMIZE_DIR}/app-learner-2-service.yaml"
apply_resource "${KUSTOMIZE_DIR}/app-learner-3-service.yaml"
apply_resource "${KUSTOMIZE_DIR}/app-learner-1-deployment.yaml"
apply_resource "${KUSTOMIZE_DIR}/app-learner-2-deployment.yaml"
apply_resource "${KUSTOMIZE_DIR}/app-learner-3-deployment.yaml"
apply_resource "${KUSTOMIZE_DIR}/ingress.yaml"

wait_for_resource statefulset certd-voter
kubernetes_wait rollout status statefulset/certd-voter --timeout=15m
wait_for_resource deployment certd-learner-1
kubernetes_wait rollout status deployment/certd-learner-1 --timeout=15m
wait_for_resource deployment certd-learner-2
kubernetes_wait rollout status deployment/certd-learner-2 --timeout=15m
wait_for_resource deployment certd-learner-3
kubernetes_wait rollout status deployment/certd-learner-3 --timeout=15m

cat <<EOF
k3d cluster '${CLUSTER_NAME}' is ready.

Namespace: ${NAMESPACE}
Image: ${IMAGE_NAME}
TrailBase Image: ${TRAILBASE_IMAGE}
Litestream Image: ${LITESTREAM_IMAGE}

Next:
  export CERTD_MASTER_KEY=${CERTD_MASTER_KEY}
  ${ROOT_DIR}/scripts/k3d-e2e.sh
  ${ROOT_DIR}/scripts/k3d-chaos.sh
  ${ROOT_DIR}/scripts/k3d-backup-restore.sh
EOF
