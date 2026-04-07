#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
NAMESPACE="${NAMESPACE:-certd}"
IMAGE_NAME="${IMAGE_NAME:-certd:local}"
TRAILBASE_IMAGE="${TRAILBASE_IMAGE:-trailbase-auth-bridge:local}"
KUSTOMIZE_DIR="${KUSTOMIZE_DIR:-$ROOT_DIR/deploy/k8s}"
VIND_SKIP_MINIO="${VIND_SKIP_MINIO:-0}"
KUBECONTEXT="${KUBECONTEXT:-}"

need() {
  command -v "$1" >/dev/null 2>&1 || {
    echo "missing command: $1" >&2
    exit 1
  }
}

need docker
need kubectl

if [[ -z "${KUBECONTEXT}" ]]; then
  KUBECONTEXT="$(kubectl config current-context)"
fi

CERTD_MASTER_KEY="${CERTD_MASTER_KEY:-local-dev-master-key}"
HQL_SECRET_RAFT="${HQL_SECRET_RAFT:-SuperSecureSecret1337}"
HQL_SECRET_API="${HQL_SECRET_API:-AnotherSecureSecret42}"
TRAILBASE_CERTD_INTERNAL_SECRET="${TRAILBASE_CERTD_INTERNAL_SECRET:-dev-trailbase-internal-secret}"
ENC_KEYS="${ENC_KEYS:-devkey01/MDEyMzQ1Njc4OWFiY2RlZjAxMjM0NTY3ODlhYmNkZWY=}"
ENC_KEY_ACTIVE="${ENC_KEY_ACTIVE:-devkey01}"
MINIO_ROOT_USER="${MINIO_ROOT_USER:-minioadmin}"
MINIO_ROOT_PASSWORD="${MINIO_ROOT_PASSWORD:-minioadmin}"

kubectl config use-context "${KUBECONTEXT}" >/dev/null

docker image inspect "${IMAGE_NAME}" >/dev/null 2>&1 || {
  echo "missing image ${IMAGE_NAME}. run: docker build -t ${IMAGE_NAME} ." >&2
  exit 1
}

docker image inspect "${TRAILBASE_IMAGE}" >/dev/null 2>&1 || {
  echo "missing image ${TRAILBASE_IMAGE}. run: docker build -f Dockerfile.trailbase-auth-bridge -t ${TRAILBASE_IMAGE} ." >&2
  exit 1
}

apply_resource() {
  kubectl -n "${NAMESPACE}" apply -f "$1"
}

wait_for_resource() {
  local kind_name="$1"
  local name="$2"
  local attempts="${3:-60}"
  local delay="${4:-2}"
  for _ in $(seq 1 "${attempts}"); do
    if kubectl -n "${NAMESPACE}" get "${kind_name}" "${name}" >/dev/null 2>&1; then
      return 0
    fi
    sleep "${delay}"
  done
  echo "timed out waiting for ${kind_name}/${name} to exist" >&2
  exit 1
}

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

apply_resource "${KUSTOMIZE_DIR}/namespace.yaml"

if [[ "${VIND_SKIP_MINIO}" != "1" ]]; then
  apply_resource "${KUSTOMIZE_DIR}/minio-headless-service.yaml"
  apply_resource "${KUSTOMIZE_DIR}/minio-service.yaml"
  apply_resource "${KUSTOMIZE_DIR}/minio-pdb.yaml"
  apply_resource "${KUSTOMIZE_DIR}/minio-statefulset.yaml"
fi

if [[ "${VIND_SKIP_MINIO}" != "1" ]]; then
  wait_for_resource statefulset minio
  kubectl -n "${NAMESPACE}" rollout status statefulset/minio --timeout=15m

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
kubectl -n "${NAMESPACE}" rollout status deployment/trailbase --timeout=15m

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

wait_for_resource statefulset certd-voter
kubectl -n "${NAMESPACE}" rollout status statefulset/certd-voter --timeout=15m
for deployment in certd-learner-1 certd-learner-2 certd-learner-3; do
  wait_for_resource deployment "${deployment}"
  kubectl -n "${NAMESPACE}" rollout status "deployment/${deployment}" --timeout=15m
done

cat <<EOF
vind bootstrap completed.

Context: ${KUBECONTEXT}
Namespace: ${NAMESPACE}
Image: ${IMAGE_NAME}
TrailBase Image: ${TRAILBASE_IMAGE}

Next:
  export CERTD_MASTER_KEY=${CERTD_MASTER_KEY}
  ${ROOT_DIR}/scripts/k3d-e2e.sh
  ${ROOT_DIR}/scripts/vind-chaos.sh
EOF
