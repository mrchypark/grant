#!/usr/bin/env bash
set -euo pipefail

NAMESPACE="${NAMESPACE:-certd}"
BACKUP_NAME="${1:-}"

if [[ -z "${BACKUP_NAME}" ]]; then
  echo "usage: $0 <backup-name>" >&2
  exit 1
fi

cat <<EOF
1. Scale down learners and voters:
   kubectl -n ${NAMESPACE} scale deployment/certd-learner-1 --replicas=0
   kubectl -n ${NAMESPACE} scale deployment/certd-learner-2 --replicas=0
   kubectl -n ${NAMESPACE} scale deployment/certd-learner-3 --replicas=0
   kubectl -n ${NAMESPACE} scale statefulset/certd-voter --replicas=0

2. Set restore env on the voter StatefulSet:
   kubectl -n ${NAMESPACE} set env statefulset/certd-voter HQL_BACKUP_RESTORE=s3:${BACKUP_NAME}

3. Scale the voter quorum back up:
   kubectl -n ${NAMESPACE} scale statefulset/certd-voter --replicas=3

4. Remove the restore env after quorum recovery:
   kubectl -n ${NAMESPACE} set env statefulset/certd-voter HQL_BACKUP_RESTORE-

5. Scale learners back up:
   kubectl -n ${NAMESPACE} scale deployment/certd-learner-1 --replicas=1
   kubectl -n ${NAMESPACE} scale deployment/certd-learner-2 --replicas=1
   kubectl -n ${NAMESPACE} scale deployment/certd-learner-3 --replicas=1
EOF
