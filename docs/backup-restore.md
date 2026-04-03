# Rust certd Backup / Restore

이 문서는 `hiqlite + 분리된 PDF 저장소` 기준 백업/복구 절차를 정리한다.

## 범위

- 메타데이터:
  `hiqlite` Raft state machine / logs / snapshots
- 산출물:
  PDF 객체 또는 filesystem 파일

JSON 산출물 파일은 별도 보관하지 않는다. 정규화 데이터는 `hiqlite`에 저장한다.

## 기본 원칙

- `hiqlite` 메타데이터는 S3 호환 스토리지에 암호화 백업한다.
- PDF 저장소는 `hiqlite` HA와 분리한다.
- quorum node는 `StatefulSet`으로 운영한다.
- local read replica는 `split` 또는 향후 `learner-only` 런타임으로 운영한다.
- PDF는 굳이 스케일아웃하지 않고 단일 object store로 운영해도 된다.
- 전체 클러스터 복구는 `hiqlite` restore와 PDF 저장소 복구를 조합한다.

## 필수 환경변수

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

## 정상 운영 시

`hiqlite`는 cron 기준으로 주기 백업을 생성하고, leader가 S3 호환 버킷으로 업로드한다.

운영 토폴로지는 다음을 전제로 한다.

- `certd-voter`: `StatefulSet` 3개
- `certd-learner-{1,2,3}`: `Deployment` 3개
- artifact 저장과 backup object는 모두 MinIO 사용

voter는 stable pod identity로 quorum을 형성하고, learner는 `split` 런타임의 read replica로 붙는다.

중요:

- `cert-backups` 버킷이 먼저 존재해야 한다.
- 로컬 `k3d` 검증에서는 [scripts/k3d-bootstrap.sh](/Users/cypark/Documents/work/cert/scripts/k3d-bootstrap.sh) 와 [scripts/k3d-backup-restore.sh](/Users/cypark/Documents/work/cert/scripts/k3d-backup-restore.sh)가 이 버킷을 보장한다.
- `GET /api/v1/admin/backups`는 local backup 목록을 보여주고, restore 대상 이름도 여기서 읽는다.

PDF 저장소는 배포 형태에 따라 다르게 본다.

- S3 backend:
  - 버킷 버전닝
  - 필요 시 site replication 또는 외부 백업 정책

## Pod 재시작

이 문서는 quorum node에 영속 볼륨이 없거나, local read replica가 휘발성인 경우를 같이 고려한다.

즉:

1. 단순 컨테이너 재시작이라 `emptyDir`가 유지되면 서비스는 계속 동작할 수 있다.
2. Pod 재생성 또는 노드 이동이 일어나면 `HQL_BACKUP_RESTORE` 기반 복구가 필요하다.

`split` 런타임 sidecar는 로컬 read 경로만 잃으면 되므로, 다시 붙은 뒤 `readReady=true`가 될 때까지는 strong read 또는 폴백 정책으로 버틸 수 있다.

`CERTD_LOCAL_NODE_ROLE=learner`를 주면 로컬 `hiqlite`는 learner로만 기동한다. 이 경우 `writeReady`는 원격 quorum 경로 상태를 뜻하고, `readReady`는 로컬 learner 복제본 상태를 뜻한다.

## 전체 클러스터 손실

`hiqlite` 공식 restore 절차:

1. learner deployment를 먼저 중지한다.
2. voter StatefulSet을 중지한다.
3. 복구할 백업 이름을 `HQL_BACKUP_RESTORE=s3:<backup-name>`로 지정한다.
4. voter StatefulSet을 다시 시작한다.
5. quorum이 회복되면 `HQL_BACKUP_RESTORE`를 제거한다.
6. learner deployment를 다시 시작한다.

즉 `cert` 배포에서는 voter quorum을 먼저 복원하고, learner는 그 뒤에 다시 붙여야 한다.

## 순서

1. `cert-backups` 버킷 및 PDF 저장소 상태 확인
2. `certd-learner-1/2/3` scale down
3. `certd-voter` scale down
4. `HQL_BACKUP_RESTORE` 설정
5. `certd-voter` scale up
6. quorum health 확인
7. `HQL_BACKUP_RESTORE` 제거
8. learner scale up
9. 앱 health 확인
10. 최근 `business / plant / sync_runs` 행 확인
11. 대표 PDF 다운로드 확인

기존 단일 Deployment 복구 절차는 이 토폴로지에서는 사용하지 않는다.

## 검증 포인트

- `GET /healthz`
- `readReady` / `writeReady` 값 확인
- `kubectl -n certd get pods`
- `statefulset/certd-voter` rollout 상태
- `deployment/certd-learner-*` rollout 상태
- `GET /api/v1/plants/{facilityCode}`
- PDF 저장소 내 문서 개수
- 대표 PDF HEAD/GET 성공
- Keploy replay 성공
