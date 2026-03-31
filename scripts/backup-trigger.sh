#!/usr/bin/env bash
set -euo pipefail

NAMESPACE="${NAMESPACE:-certd}"

cat <<EOF
hiqlite backup is driven by HQL_BACKUP_CRON inside the Rust service.

Namespace: ${NAMESPACE}

To verify backup configuration:
  kubectl -n ${NAMESPACE} get configmap certd-config -o yaml
  kubectl -n ${NAMESPACE} get secret certd-secrets -o yaml

To inspect recent logs:
  kubectl -n ${NAMESPACE} logs statefulset/certd --tail=200

To list backup objects in MinIO/S3:
  aws s3 ls s3://\${HQL_S3_BUCKET}/
EOF

