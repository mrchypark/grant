# Grant Backup / Restore

`grant`의 영속 상태는 grant hiqlite metadata와 Rauthy identity data로 나뉜다.

## 범위

- grant metadata: hiqlite Raft state machine, logs, snapshots
- Rauthy identity data: Rauthy `/app/data` directory or the production database/backend chosen for Rauthy

## 기본 원칙

- repo가 Kubernetes나 Compose 안에 object storage를 배포하지 않는다.
- hiqlite 원격 백업은 운영자가 준비한 외부 S3 호환 스토리지를 사용한다.
- 기본 Kubernetes 토폴로지는 `grant-voter` `StatefulSet` 3개다.
- 기본 Rauthy manifest는 단일 `StatefulSet`과 1Gi `ReadWriteOnce` PVC로 `/app/data`를 보존한다. 운영 배포에서는 용량, storage class, RPO/RTO에 맞는 별도 Rauthy 백업 정책을 추가한다.
- `deploy/overlays/ephemeral`과 `deploy/overlays/ied`는 검증 전용으로 Rauthy `/app/data`에 `emptyDir`을 사용한다. Pod 교체 시 사용자, client 변경, session, JWKS가 유실되고 bootstrap 데이터만 다시 생성되므로 운영에 사용하지 않는다.
- voter quorum 복구를 먼저 끝낸 뒤 API를 검증한다.
- restore는 외부 운영 절차로 수행한다. 앱 프로세스가 자동 restore를 판단하지 않는다.
- Rauthy data 백업/복구는 Rauthy 운영 절차로 다룬다.

## 원격 백업 환경변수

외부 S3 호환 백업을 켤 때만 아래 값을 `grant-config`와 `grant-secrets`에 설정한다.

- `HQL_BACKUP_CRON`
- `HQL_BACKUP_KEEP_DAYS`
- `HQL_BACKUP_KEEP_DAYS_LOCAL`
- `HQL_S3_URL`
- `HQL_S3_BUCKET`
- `HQL_S3_REGION`
- `HQL_S3_PATH_STYLE`
- `HQL_S3_KEY`
- `HQL_S3_SECRET`
- `ENC_KEYS`
- `ENC_KEY_ACTIVE`

예시 버킷 이름은 `grant-backups`다. 로컬 MinIO는 지원하지 않는다.

## 정상 운영

`hiqlite`는 cron 기준으로 주기 백업을 생성하고, leader가 외부 S3 호환 버킷으로 업로드한다.

백업 설정과 최근 로그 확인:

```bash
kubectl -n grant get configmap grant-config -o yaml
kubectl -n grant get secret grant-secrets -o yaml
kubectl -n grant logs statefulset/grant-voter --tail=200
aws s3 ls "s3://${HQL_S3_BUCKET}/"
```

`GET /api/v1/admin/backups`는 local backup 목록을 보여주고, restore 대상 이름도 여기서 읽는다.

## 전체 클러스터 손실

복구 순서:

1. voter StatefulSet을 중지한다.
2. 복구할 백업 이름을 `HQL_BACKUP_RESTORE=s3:<backup-name>`로 지정한다.
3. voter StatefulSet을 다시 시작한다.
4. quorum과 `writeReady=true`를 확인한다.
5. `HQL_BACKUP_RESTORE`를 제거한다.
6. `/healthz`, `/api/v1/providers`, `/api/v1/authz/check`로 API를 확인한다.

예시 명령:

```bash
NAMESPACE=grant
BACKUP_NAME=s3:<backup-name>

kubectl -n "${NAMESPACE}" scale statefulset/grant-voter --replicas=0

kubectl -n "${NAMESPACE}" set env statefulset/grant-voter HQL_BACKUP_RESTORE="${BACKUP_NAME}"
kubectl -n "${NAMESPACE}" scale statefulset/grant-voter --replicas=3
kubectl -n "${NAMESPACE}" rollout status statefulset/grant-voter --timeout=15m

kubectl -n "${NAMESPACE}" set env statefulset/grant-voter HQL_BACKUP_RESTORE-
```

## 검증 포인트

- `GET /healthz`
- `readReady` / `writeReady`
- `kubectl -n grant get pods`
- `statefulset/grant-voter` rollout 상태
- `GET /api/v1/admin/backups`
- 대표 `POST /api/v1/authz/check` 응답
