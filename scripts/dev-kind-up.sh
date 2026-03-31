#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
CLUSTER_NAME="${CLUSTER_NAME:-certd-dev}"

kind create cluster --name "${CLUSTER_NAME}" --wait 60s || true
kubectl apply -k "${ROOT_DIR}/deploy/k8s"

cat <<EOF
kind cluster '${CLUSTER_NAME}' prepared.

Notes:
- The MinIO Tenant manifest requires MinIO Operator CRDs in the cluster.
- Replace the example secrets before applying to a real environment.
EOF
