#!/usr/bin/env bash
set -euo pipefail

NAMESPACE="${NAMESPACE:-certd}"
BACKUP_NAME="${1:-}"

if [[ -z "${BACKUP_NAME}" ]]; then
  echo "usage: $0 <backup-name>" >&2
  exit 1
fi

cat <<EOF
1. Scale down the cluster:
   kubectl -n ${NAMESPACE} scale statefulset/certd --replicas=0

2. Set restore env:
   kubectl -n ${NAMESPACE} set env statefulset/certd HQL_BACKUP_RESTORE=s3:${BACKUP_NAME}

3. Scale the cluster back up:
   kubectl -n ${NAMESPACE} scale statefulset/certd --replicas=3

4. After restore succeeds, remove the env var:
   kubectl -n ${NAMESPACE} set env statefulset/certd HQL_BACKUP_RESTORE-
EOF

