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
- `crates/trailbase-auth-bridge`
  - 별도 TrailBase auth bridge 바이너리
  - `certd`용 `/api/certd/introspect` 제공
- `desktop-agent`
  - 윈도우/macOS loopback HTTP agent
  - 로컬 인증서 탐색과 challenge 서명 담당
- `packages/certd-web-sdk`
  - 브라우저용 JS SDK
  - `stored`와 `ephemeral` certificate gateway 흐름 오케스트레이션 담당

## Certificate Gateway

`certd`는 사용자 로그인 대체가 아니라 공동인증서 기반 작업 실행 경계를 별도로 가진다.

- `ephemeral`
  - 브라우저 SDK가 challenge를 발급받고, local desktop agent로 서명한 뒤 즉시 1회성 crawl을 시작한다
  - 인증서는 서버에 저장하지 않는다
- `stored`
  - agent가 로컬 인증서를 열고, 서버는 인증서 bundle을 per-certificate DEK로 암호화해서 저장한다
  - 이후에는 짧은 TTL의 1회용 crawl task token으로 작업만 실행한다

환경변수 기반 KMS 어댑터가 첫 구현이며, 저장 경로는 `CERTD_KMS_ACTIVE_KEY_ID`, `CERTD_KMS_MASTER_KEYS`를 사용한다.

상세 API, desktop agent 로컬 계약, JS SDK 사용 예시는 [docs/rps-certificate-gateway.md](./docs/rps-certificate-gateway.md)에 정리했다.

저장 계층:

- `hiqlite`
  - `users`
  - `organizations`
  - `organization_edges`
  - `organization_memberships`
  - `groups`
  - `group_memberships`
  - `agreements`
  - `actions`
  - `roles`
  - `role_actions`
  - `role_bindings`
  - `organization_closure`
  - `effective_organization_plant_actions`
  - `effective_user_plant_actions`
  - `effective_user_plant_visibility`
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

- quorum metadata는 `StatefulSet 3` 기준으로 운영
- local read replica / sidecar는 `split` 런타임으로 운영
- PDF는 단일 object store로 분리 운영
- `hiqlite` backup만 S3/MinIO로 보관

## 권한 모델

- `organization`
  - 사업자 조직의 기본 단위
  - 발전사업자, 총판, 대리점, 시공사, 유지보수사 모두 포함
- `group`
  - 조직 내부 팀
  - 외부 조직 관계를 표현하지 않음
- `plant`
  - 하나의 `owner_organization_id`에 귀속
  - `created_by_organization_id`로 최초 생성 조직을 기록

권한 규칙:

- `organization.*`
  - 조직 스코프 권한
  - 예: `organization.plant.create`
  - 전개 테이블이 아니라 런타임 직접 평가로 처리
- `plant.*`
  - 특정 발전소 권한
  - 예: `plant.profile.read`, `plant.sync.execute`
  - `effective_*` 전개 테이블로 조회

목록 노출 규칙:

- plant 목록은 `effective_user_plant_visibility`만 본다
- 1차 구현 기준 visibility를 만드는 권한은 `plant.profile.read` 하나뿐이다
- `plant.document.read` 같은 다른 read 권한만으로는 plant 목록에 노출되지 않는다

Kubernetes 기준 권장 토폴로지:

- `certd-voter`: `StatefulSet` 3개, `emptyDir`, headless service
  - 의도적으로 PVC 대신 backup/restore 기반 복구를 기본 경로로 검증한다
- `certd-learner-{1,2,3}`: `Deployment` 3개, `split + learner + local`
- `trailbase`: 단일 `Deployment`, `emptyDir`, Litestream restore-then-run
- `minio`: `StatefulSet` 4개
- public API path: `Service/certd` -> voter pod
- auth path: `certd` -> in-cluster `trailbase`
- quorum path: voter pod 간 raft/api 통신
- auth recovery path: TrailBase local SQLite -> Litestream -> MinIO
- learner path: 다른 시스템에 sidecar로 붙여 로컬 authz/read 평가에 사용
- learner HTTP surface: auth passthrough + read/authz 중심, write/admin은 voter 전용

## 런타임 모드

`certd`는 이제 `hiqlite` 접근 방식을 세 가지로 나눈다.

- `embedded`
  - 로컬 `hiqlite` 노드만 사용
  - 개발/단일 노드 운영용
- `remote`
  - 로컬 상태 없이 원격 quorum cluster만 사용
- `split`
  - 로컬 읽기용 client와 원격 quorum client를 같이 사용
  - learner sidecar는 일반 조회/authz를 로컬로 처리하고, voter만 full write/admin surface를 가진다
  - 읽기는 기본 local이고, 최신성이 꼭 필요한 요청만 명시적으로 strong을 요청한다

관련 환경변수:

```bash
export CERTD_RUNTIME_MODE=embedded   # embedded | remote | split
export CERTD_LOCAL_NODE_ROLE=voter   # voter | learner
export CERTD_READ_CONSISTENCY=local  # local | strong | prefer-strong

export CERTD_REMOTE_NODES=127.0.0.1:18100,127.0.0.1:18110
export CERTD_REMOTE_TLS=false
export CERTD_REMOTE_TLS_NO_VERIFY=false
export CERTD_REMOTE_API_SECRET=AnotherSecureSecret42
export CERTD_REMOTE_WITH_PROXY=false
```

`CERTD_REMOTE_NODES`는 `host:port` 형식을 권장한다. 기존 `http://...` 또는 `https://...` 값이 들어와도 앱에서 scheme를 제거하고 `hiqlite`에 전달한다.

읽기 의미는 다음과 같다.

- `local`
  - 로컬 SQLite 우선
- `strong`
  - quorum 강일관성 읽기
  - write 경로가 죽어 있으면 실패
- `prefer-strong`
  - quorum 읽기를 우선 시도
  - quorum이 불가하면 로컬로 폴백

HTTP read 계약은 더 좁게 잡는다.

- 기본값은 local read다.
- 읽기 요청에서만 `?consistency=strong`을 붙여 quorum read를 명시적으로 요청할 수 있다.
- `?consistency=strong`은 실패 시 실패하며, 로컬로 숨은 폴백을 하지 않는다.
- `consistency` query는 현재 `strong`만 허용하고, 다른 값은 `400`이다.
- write 후 최신 상태가 꼭 필요하면 follow-up read를 `?consistency=strong`으로 호출한다.

`GET /healthz`는 아래 의미를 같이 노출한다.

- `readReady`
  - 로컬 또는 대체 read 경로가 사용 가능한지
- `writeReady`
  - quorum write / strong read 경로가 사용 가능한지
- `runtimeMode`
  - 현재 런타임 모드
- `defaultReadConsistency`
  - 기본 읽기 정책

`CERTD_LOCAL_NODE_ROLE=learner`면 로컬 `hiqlite` 노드는 voter 승격을 시도하지 않고 learner로 유지된다. Kubernetes 배포 기준 learner 기본 읽기 정책은 `local`로 둔다.
이 기능은 `mrchypark/hiqlite` 포크의 `main` 최신 `3e2112cd140c24730a0d1493054ad007cdabc48e` 커밋을 `crates.io` 패치로 지정해서 제공된다.

운영 판단 기준:

- `certd` public API는 voter quorum 상태를 직접 탄다.
- learner는 public API endpoint가 아니라 sidecar/보조 read 경로로 본다.
- learner 장애는 disposable replica 재생성 문제다.
- voter 장애는 quorum 유지 여부가 기준이다.

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

로컬 검증 엔트리포인트는 `vind`만 지원한다. 임시 compose 스택이나 ad-hoc 빈 상태 스크립트는 더 이상 유지하지 않는다.

필수 도구:

- Docker
- `kubectl`
- `vcluster`
- `curl`
- `jq`

권장 검증 순서:

1. `docker buildx build --load -t certd:local .`
2. `docker buildx build --load -f Dockerfile.trailbase-auth-bridge -t trailbase-auth-bridge:local .`
3. `bash ./scripts/vind-cluster.sh ensure-context`
4. `bash ./scripts/vind-exec.sh bash ./scripts/vind-bootstrap.sh`
5. `bash ./scripts/vind-exec.sh bash ./scripts/k3d-e2e.sh`
6. `bash ./scripts/vind-exec.sh bash ./scripts/vind-chaos.sh`

단일 엔트리포인트로 e2e/chaos를 돌리려면 아래를 사용한다.

```bash
bash ./scripts/ci-chaos-local.sh
```

설명:

- `scripts/ci-chaos-local.sh`
  - 이미지 빌드
  - `vind` cluster lifecycle 관리
  - bootstrap, e2e, reduced chaos까지 단일 경로로 실행
- `scripts/vind-cluster.sh`
  - Colima preflight
  - `vcluster` create/connect/delete
  - control-plane taint normalization
- `scripts/vind-exec.sh`
  - `vcluster connect ... -- <command>`를 phase 단위로 감싼다
- `scripts/vind-bootstrap.sh`
  - Kubernetes 리소스 배포
  - MinIO/TrailBase/certd rollout 대기
- `scripts/vind-chaos.sh`
  - `vind` 컨텍스트에서 reduced chaos 시나리오를 실행한다

주의:

- `scripts/k3d-e2e.sh`, `scripts/k3d-chaos.sh`는 내부 phase 구현체로 남아 있지만, 로컬 진입점으로 직접 쓰지 않는다.
- 로컬/CI 회귀 검증은 `vind` 경로만 신뢰한다.

### production compose

릴리즈용 이미지는 별도 compose와 env 파일 예시를 사용한다.

파일:

- `docker-compose.prod.yml`
- `.env.prod.example`

시작 전 `.env.prod.example`를 복사해서 실제 값으로 채운다.

```bash
cp .env.prod.example .env.prod
```

그 다음:

```bash
docker-compose -f docker-compose.prod.yml --env-file .env.prod up -d --build
```

이 구성은 다음을 전제로 한다.

- `certd`는 `8080`
- 내부 `trailbase`는 외부에 publish하지 않음
- MinIO bucket은 `cert-artifacts`, `cert-backups`
- 운영 비밀값은 `.env.prod`에서만 주입

API 예시는 이미 `vind` 경로로 bootstrap이 끝났고, `scripts/k3d-e2e.sh` 또는 `scripts/vind-chaos.sh`가 사용하는 것처럼 `svc/certd`가 로컬 `127.0.0.1:18080`으로 포트포워딩되어 있다는 전제를 둔다.

필요한 최소 변수:

```bash
export BUSINESS_NO=2768603291
export ORGANIZATION_ID=2768603291
export FACILITY_CODE=PV15-R-090795
export CERT_PASSWORD='...'
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
  "http://127.0.0.1:18080/api/v1/organizations/${ORGANIZATION_ID}/certificate" \
  -H 'content-type: application/json' \
  --data-binary "$(jq -c --arg businessNo "${BUSINESS_NO}" '. + {businessNo: $businessNo}' /tmp/cert-upload.json)" | jq
```

응답 예시:

```json
{
  "id": "75c4b33393cb4375b1e66e38cb22c25e",
  "organizationId": "2768603291",
  "partyId": "party-2768603291",
  "businessNo": "2768603291",
  "commonName": "커널로그에너지"
}
```

### 2. 전체 수집

최초 1회는 `mode=full`이 맞다.

```bash
curl -sS -X POST \
  "http://127.0.0.1:18080/api/v1/organizations/${ORGANIZATION_ID}/sync" \
  -H 'content-type: application/json' \
  --data-binary "{\"facilityCode\":\"${FACILITY_CODE}\",\"mode\":\"full\"}" | jq
```

응답 예시:

```json
{
  "id": "eec4046dce084006b2e1ed40635eef7e",
  "organizationId": "2768603291",
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
  "http://127.0.0.1:18080/api/v1/organizations/${ORGANIZATION_ID}/sync" \
  -H 'content-type: application/json' \
  --data-binary "{\"facilityCode\":\"${FACILITY_CODE}\",\"mode\":\"delta\"}" | jq
```

응답 예시:

```json
{
  "id": "01b37be6efa94f25ae0c34c7813f714e",
  "organizationId": "2768603291",
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

### vind TrailBase + certd chaos topology

현재 로컬 검증은 `vind`가 클러스터 lifecycle을 소유하고, 그 안에서 `certd` 데이터 평면과 `TrailBase` 인증 평면을 분리해서 본다.

- `certd-voter`
  - `StatefulSet` 3개
  - `emptyDir`
  - quorum과 backup/restore 담당
- `certd-learner-{1,2,3}`
  - `Deployment` 3개
  - `split + learner`
  - read path 담당
- `trailbase`
  - 단일 `Deployment`
  - Litestream restore-then-run
  - MinIO로 local SQLite fast recovery
- `minio`
  - `cert-artifacts`
  - `cert-backups`

운영 흐름은 아래 순서다.

1. TrailBase user seed
2. `certd` bootstrap owner 연결
3. plant/list/read/sync e2e
4. quorum 및 auth cache chaos
5. `hiqlite` backup 또는 restore
6. TrailBase restore check

주요 스크립트:

- [scripts/ci-chaos-local.sh](/Users/cypark/.codex/worktrees/6cf6/account/scripts/ci-chaos-local.sh)
  - 로컬/CI 공용 엔트리포인트
  - 이미지 빌드
  - `vind` cluster ensure
  - bootstrap, e2e, reduced chaos 실행
- [scripts/vind-cluster.sh](/Users/cypark/.codex/worktrees/6cf6/account/scripts/vind-cluster.sh)
  - `vind` create/connect/delete
  - Colima preflight
  - node taint normalization
- [scripts/vind-exec.sh](/Users/cypark/.codex/worktrees/6cf6/account/scripts/vind-exec.sh)
  - phase 실행 시점에만 `vcluster connect`를 연다
- [scripts/vind-bootstrap.sh](/Users/cypark/.codex/worktrees/6cf6/account/scripts/vind-bootstrap.sh)
  - MinIO bucket bootstrap
  - TrailBase rollout 대기
  - `certd` quorum/learner rollout 대기
- [scripts/vind-chaos.sh](/Users/cypark/.codex/worktrees/6cf6/account/scripts/vind-chaos.sh)
  - reduced chaos wrapper
  - 내부적으로 `scripts/k3d-chaos.sh` phase 구현체를 호출

권장 명령:

```bash
bash ./scripts/ci-chaos-local.sh
```

phase를 수동으로 나눠 확인하려면:

```bash
bash ./scripts/vind-cluster.sh ensure-context
bash ./scripts/vind-exec.sh bash ./scripts/vind-bootstrap.sh
bash ./scripts/vind-exec.sh bash ./scripts/k3d-e2e.sh
bash ./scripts/vind-exec.sh bash ./scripts/vind-chaos.sh
```

`k3d-*` 스크립트는 phase 구현체와 DR 시나리오 보조 스크립트로 유지하지만, 로컬 검증 진입점으로 직접 지원하지 않는다.

최근 검증 결과:

- `scripts/ci-chaos-local.sh`: 통과
- 내부 reduced chaos phase(`scripts/vind-chaos.sh` -> `scripts/k3d-chaos.sh`): 통과
- DR 보조 스크립트(`scripts/k3d-backup-restore.sh`, `scripts/k3d-prod-restore-chaos.sh`): 통과
- `scripts/k3d-trailbase-restore-check.sh`: TrailBase restore 확인용
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

이미지는 `vind` 로컬 검증 경로에서 그대로 사용한다.

```bash
docker build -t certd:local .
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
