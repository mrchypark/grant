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
- PDF는 굳이 스케일아웃하지 않고 PVC 또는 단일 object store로 운영해도 된다.
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

중요:

- `cert-backups` 버킷이 먼저 존재해야 한다.
- 로컬 `k3d` 검증에서는 [scripts/k3d-backup-restore.sh](/Users/cypark/Documents/work/cert/scripts/k3d-backup-restore.sh)가 시작 시 이 버킷을 보장한다.
- `GET /api/v1/admin/backups`는 local backup 목록을 보여주고, restore 대상 이름도 여기서 읽는다.

PDF 저장소는 배포 형태에 따라 다르게 본다.

- filesystem backend:
  - PVC 스냅샷 또는 노드 백업
- S3 backend:
  - 버킷 버전닝
  - 필요 시 site replication 또는 외부 백업 정책

## 노드 1개 손실

`hiqlite` 공식 권장 절차:

1. 손실된 노드의 Pod를 중지한다.
2. 손실된 노드의 깨진 로컬 데이터를 제거한다.
3. Pod를 다시 시작한다.
4. 최신 leader에서 snapshot + raft logs를 다시 받아 자동 복구한다.

이 경우 S3 backup restore는 필요 없다.

## 전체 클러스터 손실

`hiqlite` 공식 restore 절차:

1. 전체 클러스터를 중지한다.
2. 복구할 백업 이름을 `HQL_BACKUP_RESTORE=s3:<backup-name>`로 지정한다.
3. 클러스터를 다시 시작한다.
4. 복구 후 `HQL_BACKUP_RESTORE`를 제거한다.

이후 PDF 저장소도 복구 시점과 일치하도록 되돌린다.

## 순서

1. `cert-backups` 버킷 및 PDF 저장소 상태 확인
2. `certd` StatefulSet scale down
3. `HQL_BACKUP_RESTORE` 설정
4. `certd` StatefulSet scale up
5. 앱 health 확인
6. 최근 `business / plant / sync_runs` 행 확인
7. 대표 PDF 다운로드 확인

## 검증 포인트

- `GET /healthz`
- `GET /api/v1/plants/{facilityCode}`
- PDF 저장소 내 문서 개수
- 대표 PDF HEAD/GET 성공
- Keploy replay 성공
