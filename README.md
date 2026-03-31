# certd

`certd`는 공동인증서로 `rps.energy.or.kr` 데이터를 수집하는 순수 Rust 서비스다.

핵심 원칙:

- 발전량, 이용률, REC, 문서 메타데이터는 모두 `hiqlite` 테이블에 저장한다.
- JSON 산출물 파일은 영구 저장하지 않는다.
- PDF만 별도 저장소에 보관한다.
- `GET` API는 저장된 데이터만 읽고, `POST /sync`만 실제 동기화를 수행한다.

## 아키텍처

- `crates/domain`
  - 도메인 타입
- `crates/infra`
  - 설정, 인증서 암복호화, NPKI signer, RPS collector, `hiqlite`, PDF 저장소
- `crates/api`
  - Axum HTTP API
- `crates/app`
  - 실행 바이너리

저장 계층:

- `hiqlite`
  - `users`
  - `businesses`
  - `business_memberships`
  - `legal_entities`
  - `certificates`
  - `plants`
  - `sync_runs`
  - `generation_records`
  - `rate_records`
  - `rec_records`
  - `documents`
- PDF 저장소
  - 기본: filesystem
  - 선택: S3 호환 object store

운영 권장:

- 메타데이터는 `hiqlite` 3노드 이상으로 HA
- PDF는 filesystem PVC 또는 단일 object store로 분리 운영
- `hiqlite` backup만 S3/MinIO로 보관

## 요구사항

- Rust toolchain
- Docker
- `curl`
- `jq`
- 공동인증서 4개 파일
  - `signCert.der`
  - `signPri.key`
  - `kmCert.der`
  - `kmPri.key`

권장 경로:

```bash
mkdir -p ./secrets/cert
cp /Volumes/Lexar/NPKI/.../signCert.der ./secrets/cert/
cp /Volumes/Lexar/NPKI/.../signPri.key  ./secrets/cert/
cp /Volumes/Lexar/NPKI/.../kmCert.der   ./secrets/cert/
cp /Volumes/Lexar/NPKI/.../kmPri.key    ./secrets/cert/
```

인증서 비밀번호는 코드나 문서에 저장하지 말고 환경변수나 외부 시크릿 저장소에서 주입한다.

## 로컬 실행

로컬 기준 구성:

- 메타데이터: `hiqlite` 단일 노드
- PDF 저장: 로컬 filesystem
- backup: MinIO

### 1. 작업 디렉터리 준비

```bash
export CERTD_ROOT="$(mktemp -d /tmp/certd-local.XXXXXX)"
mkdir -p "$CERTD_ROOT/data" "$CERTD_ROOT/artifacts"
echo "$CERTD_ROOT"
```

### 2. MinIO 실행

MinIO는 기본적으로 `hiqlite` backup 검증용이다. PDF는 기본 설정이면 filesystem에 저장된다.

```bash
docker run -d \
  --name cert-minio \
  -p 9000:9000 \
  -p 9001:9001 \
  -e MINIO_ROOT_USER=minioadmin \
  -e MINIO_ROOT_PASSWORD=minioadmin \
  quay.io/minio/minio server /data --console-address ':9001'
```

콘솔: [http://127.0.0.1:9001](http://127.0.0.1:9001)

### 3. 환경변수 설정

```bash
export BUSINESS_NO=2768603291
export FACILITY_CODE=PV15-R-090795
export CERT_PASSWORD='...'

export CERTD_BIND_ADDR=127.0.0.1:18080
export CERTD_MASTER_KEY=local-dev-master-key
export CERTD_BOOTSTRAP_USER_ID=conalog-account
export CERTD_BOOTSTRAP_USER_NAME='Conalog Account'

export ARTIFACTS_BACKEND=fs
export ARTIFACTS_ROOT_DIR="$CERTD_ROOT/artifacts"

export HQL_NODE_ID=1
export HQL_NODES=$'1 127.0.0.1:18100 127.0.0.1:18200\n'
export HQL_LISTEN_ADDR_API=127.0.0.1
export HQL_LISTEN_ADDR_RAFT=127.0.0.1
export HQL_DATA_DIR="$CERTD_ROOT/data"
export HQL_SECRET_RAFT=SuperSecureSecret1337
export HQL_SECRET_API=AnotherSecureSecret42
export HQL_ENC_KEYS_FROM=env
export ENC_KEYS=$'bVCyTsGaggVy5yqQ/UzluN29DZW41M3hTSkx6Y3NtZmRuQkR2TnJxUTYzcjQ=\n'
export ENC_KEY_ACTIVE=bVCyTsGaggVy5yqQ

export HQL_S3_URL=http://127.0.0.1:9000
export HQL_S3_BUCKET=cert-artifacts
export HQL_S3_REGION=us-east-1
export HQL_S3_PATH_STYLE=true
export HQL_S3_KEY=minioadmin
export HQL_S3_SECRET=minioadmin
```

### 4. 서버 실행

```bash
cargo run -p app
```

health check:

```bash
curl -sS http://127.0.0.1:18080/healthz | jq
```

## API 사용 순서

### 1. 인증서 업로드

```bash
SIGN_CERT=$(base64 < ./secrets/cert/signCert.der | tr -d '\n')
SIGN_PRI=$(base64 < ./secrets/cert/signPri.key | tr -d '\n')
KM_CERT=$(base64 < ./secrets/cert/kmCert.der | tr -d '\n')
KM_PRI=$(base64 < ./secrets/cert/kmPri.key | tr -d '\n')

cat > /tmp/cert-upload.json <<JSON
{
  "password": "${CERT_PASSWORD}",
  "signCertDerBase64": "${SIGN_CERT}",
  "signPriKeyBase64": "${SIGN_PRI}",
  "kmCertDerBase64": "${KM_CERT}",
  "kmPriKeyBase64": "${KM_PRI}"
}
JSON

curl -sS -X PUT \
  "http://127.0.0.1:18080/api/v1/businesses/${BUSINESS_NO}/certificate" \
  -H 'content-type: application/json' \
  --data-binary @/tmp/cert-upload.json | jq
```

응답 예시:

```json
{
  "id": "75c4b33393cb4375b1e66e38cb22c25e",
  "businessNo": "2768603291",
  "commonName": "커널로그에너지"
}
```

### 2. 전체 수집

최초 1회는 `mode=full`이 맞다.

```bash
curl -sS -X POST \
  "http://127.0.0.1:18080/api/v1/businesses/${BUSINESS_NO}/sync" \
  -H 'content-type: application/json' \
  --data-binary "{\"facilityCode\":\"${FACILITY_CODE}\",\"mode\":\"full\"}" | jq
```

응답 예시:

```json
{
  "id": "eec4046dce084006b2e1ed40635eef7e",
  "businessNo": "2768603291",
  "plantId": "PV15-R-090795",
  "mode": "full",
  "status": "completed",
  "startedAt": "2026-03-31T07:49:18.211743+00:00",
  "finishedAt": "2026-03-31T07:49:42.621976+00:00",
  "durationMs": 24408
}
```

### 3. 증분 수집

다음 달 이후에는 `mode=delta`를 쓴다. 마지막 저장 월부터 현재 월까지만 다시 수집하고 PDF는 재다운로드하지 않는다.

```bash
curl -sS -X POST \
  "http://127.0.0.1:18080/api/v1/businesses/${BUSINESS_NO}/sync" \
  -H 'content-type: application/json' \
  --data-binary "{\"facilityCode\":\"${FACILITY_CODE}\",\"mode\":\"delta\"}" | jq
```

응답 예시:

```json
{
  "id": "01b37be6efa94f25ae0c34c7813f714e",
  "businessNo": "2768603291",
  "plantId": "PV15-R-090795",
  "mode": "delta",
  "status": "completed",
  "startedAt": "2026-03-31T07:50:11.686213+00:00",
  "finishedAt": "2026-03-31T07:50:12.643992+00:00",
  "durationMs": 956
}
```

## 조회 API

### 발전소 요약

```bash
curl -sS "http://127.0.0.1:18080/api/v1/plants/${FACILITY_CODE}" | jq
```

실제 예시:

```json
{
  "plant": {
    "code": "PV15-R-090795",
    "name": "커널로그1호 태양광발전소",
    "businessNo": "2768603291",
    "contractNo": "5001058115",
    "address": "부산광역시 기장군 장안읍 명례리 907-4",
    "installedKw": 221.615,
    "recStartDate": "2024-09-05",
    "confirmIssued": "2024-11-12"
  },
  "lastSyncedAt": "2026-03-31T07:49:42.621976+00:00",
  "lastSync": {
    "id": "eec4046dce084006b2e1ed40635eef7e",
    "businessNo": "2768603291",
    "plantId": "PV15-R-090795",
    "mode": "full",
    "status": "completed",
    "startedAt": "2026-03-31T07:49:18.211743+00:00",
    "finishedAt": "2026-03-31T07:49:42.621976+00:00",
    "durationMs": 24408
  },
  "generation": {
    "count": 18,
    "totalKwh": 438529.0
  },
  "rates": {
    "count": 17
  },
  "rec": {
    "count": 18,
    "total": 657
  },
  "documents": {
    "count": 22,
    "categories": {
      "development": 4,
      "drawings": 3,
      "equipment": 4,
      "finance": 2,
      "permit": 4,
      "photos": 2,
      "plant_info": 1,
      "site_land": 2
    }
  }
}
```

의미:

- `generation.count`: 월간 발전량 레코드 수
- `generation.totalKwh`: 월간 발전량 합계
- `rates.count`: 공개된 이용률 레코드 수
- `rec.count`: REC 발급 레코드 수
- `rec.total`: REC 발급 수량 합계
- `documents.categories`: 문서 카테고리별 건수

### 월간 발전량

```bash
curl -sS "http://127.0.0.1:18080/api/v1/plants/${FACILITY_CODE}/generation" | jq
```

### 인접/전국 이용률

미공개 월은 응답에서 제외된다.

```bash
curl -sS "http://127.0.0.1:18080/api/v1/plants/${FACILITY_CODE}/rates" | jq
```

### REC 발급내역

```bash
curl -sS "http://127.0.0.1:18080/api/v1/plants/${FACILITY_CODE}/rec" | jq
```

### 문서 목록

```bash
curl -sS "http://127.0.0.1:18080/api/v1/plants/${FACILITY_CODE}/documents" | jq
```

응답 예시:

```json
{
  "id": "2764d6df475842aa8a97d065af6e3488",
  "category": "development",
  "fileClass": "개발행위",
  "fileSecName": "개발행위 허가서",
  "fileName": "19.개발행위(변경)허가서_대성부산1호. (주)대성엄정에너지.조성천.pdf",
  "required": false,
  "downloadUrl": "/api/v1/documents/2764d6df475842aa8a97d065af6e3488/download"
}
```

### 문서 다운로드

```bash
DOC_ID=$(curl -sS "http://127.0.0.1:18080/api/v1/plants/${FACILITY_CODE}/documents" | jq -r '.[0].id')
curl -OJ "http://127.0.0.1:18080/api/v1/documents/${DOC_ID}/download"
```

## backup

### backup 트리거

```bash
curl -sS -X POST "http://127.0.0.1:18080/api/v1/admin/backups"
```

### 로컬 backup 목록

```bash
curl -sS "http://127.0.0.1:18080/api/v1/admin/backups" | jq
```

### S3 backup 목록

```bash
curl -sS "http://127.0.0.1:18080/api/v1/admin/backups/s3" | jq
```

복구 절차는 [docs/backup-restore.md](/Users/cypark/Documents/work/cert/docs/backup-restore.md)에 정리돼 있다.

## Keploy 외부 시스템 E2E

`certd`의 핵심 회귀 리스크는 `POST /sync`가 `kcert.energy.or.kr`, `rps.energy.or.kr`를 직접 호출하는 구간이다.  
Keploy는 이 흐름을 한 번 실호출로 기록하고, 이후에는 외부 시스템 없이 replay test를 돌리는 용도로 붙인다.

원칙:

- `record`는 실제 공동인증서 로그인과 RPS 조회를 한 번 수행한다.
- `test`는 기록된 testcase/mock으로 replay만 수행한다.
- JSON 산출물 파일은 만들지 않는다.
- PDF는 계속 app runtime storage에만 저장한다.

Keploy 자산:

- 설정: [.keploy/keploy.yml](/Users/cypark/Documents/work/cert/.keploy/keploy.yml)
- record 스크립트: [scripts/keploy-record.sh](/Users/cypark/Documents/work/cert/scripts/keploy-record.sh)
- replay 스크립트: [scripts/keploy-test.sh](/Users/cypark/Documents/work/cert/scripts/keploy-test.sh)
- k8s replay 스크립트: [scripts/keploy-k8s-test.sh](/Users/cypark/Documents/work/cert/scripts/keploy-k8s-test.sh)
- 시나리오 드라이버: [scripts/keploy-scenario.sh](/Users/cypark/Documents/work/cert/scripts/keploy-scenario.sh)

설정 포인트:

- `/healthz`는 testcase에서 제외
- `id`, `startedAt`, `finishedAt`, `durationMs`, `lastSyncedAt`, `lastSync.*`는 global noise 처리
- 문서 ID는 이제 랜덤이 아니라 stable hash라서 후속 replay에 유리
- replay host는 `127.0.0.1`
- 외부 HTTPS는 `--pass-through-ports 443`로 TLS passthrough 처리
- `test.apiTimeout=120`으로 느린 full sync까지 허용

### 준비

- Docker daemon
- `certd:local` 이미지
- native Keploy CLI
- 공동인증서 파일 4개가 [secrets/cert](/Users/cypark/Documents/work/cert/secrets/cert)에 존재
- `CERT_PASSWORD` 환경변수

Keploy CLI 설치:

```bash
curl -sL https://keploy.io/install.sh | bash -s -- -platform zsh -noRoot
~/.keploy/bin/keploy --help
```

`colima`나 다른 macOS Docker context를 쓰면 `scripts/keploy-record.sh`, `scripts/keploy-test.sh`가 현재 docker context에서 `DOCKER_HOST`를 자동으로 가져온다.

### record

```bash
export CERT_PASSWORD='...'
/Users/cypark/Documents/work/cert/scripts/keploy-record.sh
```

이 스크립트가 수행하는 일:

- native Keploy CLI 실행
- `certd-keploy` 컨테이너 실행
- 공동인증서 업로드
- `mode=full` sync 호출
- `plant`, `generation`, `rates`, `rec`, `documents` 조회
- Keploy testcase/mock 저장

기록 결과는 아래에 남는다.

- testcase: [.keploy/keploy/test-set-0/tests](/Users/cypark/Documents/work/cert/.keploy/keploy/test-set-0/tests)
- mocks: [.keploy/keploy/test-set-0/mocks.yaml](/Users/cypark/Documents/work/cert/.keploy/keploy/test-set-0/mocks.yaml)

최근 검증에서는 `test-1.yaml`, `test-2.yaml` 2건이 기록됐고, full sync는 약 `22.7s`였다.

### replay test

```bash
/Users/cypark/Documents/work/cert/scripts/keploy-test.sh
```

최근 검증 결과:

- `Total tests: 2`
- `Total test passed: 2`
- `Total test failed: 0`
- `test-set-0 2 passed 0 failed`

### k3d / chaos 기준 replay

이미 k3d에 배포된 `certd`를 상대로 replay만 수행하려면 아래 스크립트를 쓴다.

```bash
/Users/cypark/Documents/work/cert/scripts/keploy-k8s-test.sh
```

이 스크립트는 `kubectl port-forward svc/certd 18080:80`를 열고, Keploy를 Docker 모드로 실행한 뒤 `--host 127.0.0.1 --port 18080` 기준으로 기존 testcase를 재생한다. `--base-path`는 현재 Keploy 버전에서 agent lifecycle과 충돌해 쓰지 않는다.

카오스/복구 스크립트도 이 경로를 기준으로 검증한다.

- [scripts/k3d-chaos.sh](/Users/cypark/Documents/work/cert/scripts/k3d-chaos.sh)
  - baseline replay
  - `certd` pod kill
  - `mode=delta` sync
  - replay 재실행
  - `minio` pod kill
  - 문서 download 확인
  - replay 재실행
- [scripts/k3d-backup-restore.sh](/Users/cypark/Documents/work/cert/scripts/k3d-backup-restore.sh)
  - `cert-backups` 버킷 보장
  - backup 생성
  - PVC 삭제 + `HQL_BACKUP_RESTORE`
  - restore 후 plant/documents 확인
  - replay 재실행

최근 검증 결과:

- [scripts/k3d-chaos.sh](/Users/cypark/Documents/work/cert/scripts/k3d-chaos.sh): 통과
- [scripts/k3d-backup-restore.sh](/Users/cypark/Documents/work/cert/scripts/k3d-backup-restore.sh): 통과
- restore 후 `GET /api/v1/plants/PV15-R-090795` 정상
- restore 후 대표 PDF `HEAD` 정상
- restore 후 Keploy replay `2 / 2` 통과

### 시나리오만 직접 실행

앱이 이미 떠 있는 상태라면 아래만 따로 돌릴 수 있다.

```bash
export CERT_PASSWORD='...'
/Users/cypark/Documents/work/cert/scripts/keploy-scenario.sh
```

참고:

- Keploy 자체는 native macOS CLI로 실행한다.
- 앱은 Docker 컨테이너로 띄우고, Keploy가 그 트래픽을 record/replay 한다.
- 이번 시나리오는 외부 시스템 회귀에 집중하기 위해 문서 binary download는 포함하지 않는다.
- 문서 download API는 별도 smoke test로 유지하면 충분하다.
- 외부 HTTPS는 passthrough라서 record/test 동안 `kcert.energy.or.kr`, `rps.energy.or.kr`의 TLS를 Keploy가 MITM하지 않는다.

## Docker

이미지는 Rust 바이너리만 포함한다.

```bash
docker build -t certd:local .
```

예시 실행:

```bash
docker run --rm \
  -p 18080:8080 \
  -e CERTD_MASTER_KEY=local-dev-master-key \
  -e CERTD_BOOTSTRAP_USER_ID=conalog-account \
  -e CERTD_BOOTSTRAP_USER_NAME='Conalog Account' \
  -e ARTIFACTS_BACKEND=fs \
  -e ARTIFACTS_ROOT_DIR=/data/artifacts \
  -e HQL_NODE_ID=1 \
  -e HQL_NODES=$'1 0.0.0.0:8100 0.0.0.0:8200\n' \
  -e HQL_LISTEN_ADDR_API=0.0.0.0 \
  -e HQL_LISTEN_ADDR_RAFT=0.0.0.0 \
  -e HQL_DATA_DIR=/data/db \
  -e HQL_SECRET_RAFT=SuperSecureSecret1337 \
  -e HQL_SECRET_API=AnotherSecureSecret42 \
  -e HQL_ENC_KEYS_FROM=env \
  -e ENC_KEYS=$'bVCyTsGaggVy5yqQ/UzluN29DZW41M3hTSkx6Y3NtZmRuQkR2TnJxUTYzcjQ=\n' \
  -e ENC_KEY_ACTIVE=bVCyTsGaggVy5yqQ \
  -v "$PWD/runtime:/data" \
  certd:local
```

## 검증 기준

현재 로컬 인증서 기준 실동작 검증:

- `PUT /certificate` 성공
- `POST /sync` with `mode=full` 성공
- `POST /sync` with `mode=delta` 성공
- `GET /plants/{facilityCode}` 응답 정상
- `GET /generation`, `GET /rates`, `GET /rec`, `GET /documents` 응답 정상
- `GET /documents/{id}/download` PDF 응답 정상

## 정리

- JSON 파일 적재는 하지 않는다.
- 모든 메타데이터는 DB에 저장한다.
- PDF만 별도 저장한다.
- 전체 재수집은 `mode=full`
- 증분 갱신은 `mode=delta`
