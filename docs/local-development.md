# 로컬 개발과 API smoke

로컬 검증 엔트리포인트는 `vind`만 공식 지원한다. `k3d-*` 스크립트는 내부 phase 구현체와 보조 DR 시나리오로 남아 있지만, 직접 진입점으로 보지 않는다.

## 준비물

- Rust toolchain
- Docker
- `kubectl`
- `vcluster`
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
cp /path/to/signCert.der ./secrets/cert/
cp /path/to/signPri.key  ./secrets/cert/
cp /path/to/kmCert.der   ./secrets/cert/
cp /path/to/kmPri.key    ./secrets/cert/
```

인증서 비밀번호는 코드나 문서에 저장하지 말고 환경변수나 외부 시크릿 저장소에서 주입한다.

## 권장 검증 순서

```bash
docker buildx build --load -t certd:local .
docker buildx build --load -f Dockerfile.trailbase-auth-bridge -t trailbase-auth-bridge:local .
bash ./scripts/vind-cluster.sh ensure-context
bash ./scripts/vind-exec.sh bash ./scripts/vind-bootstrap.sh
bash ./scripts/vind-exec.sh bash ./scripts/k3d-e2e.sh
bash ./scripts/vind-exec.sh bash ./scripts/vind-chaos.sh
```

단일 엔트리포인트로 e2e/chaos까지 실행하려면:

```bash
bash ./scripts/ci-chaos-local.sh
```

## 주요 스크립트

- `scripts/ci-chaos-local.sh`
  - 이미지 빌드
  - `vind` cluster lifecycle 관리
  - bootstrap, e2e, reduced chaos 실행
- `scripts/vind-cluster.sh`
  - Colima preflight
  - `vcluster` create/connect/delete
  - control-plane taint normalization
- `scripts/vind-exec.sh`
  - phase 실행 시점에만 `vcluster connect`를 연다
- `scripts/vind-bootstrap.sh`
  - Kubernetes 리소스 배포
  - MinIO, TrailBase, `certd` rollout 대기
- `scripts/vind-chaos.sh`
  - `vind` 컨텍스트에서 reduced chaos 시나리오를 실행한다

주의:

- 로컬/CI 회귀 검증은 `vind` 경로만 신뢰한다.
- Colima ARM 환경에서 `vind` create가 platform warning으로 실패하면 `docker pull --platform linux/arm64 alpine:latest` 상태를 먼저 확인한다.

## API smoke 예시

`scripts/vind-bootstrap.sh`와 `scripts/k3d-e2e.sh`까지 끝나서 `svc/certd`가 로컬 `127.0.0.1:18080`으로 포트포워딩된 상태를 전제로 한다.

필수 변수:

```bash
export BUSINESS_NO=2768603291
export ORGANIZATION_ID=2768603291
export FACILITY_CODE=PV15-R-090795
export CERT_PASSWORD='...'
```

### 1. 저장형 인증서 업로드 (escrow)

이 예시는 서버 저장형 인증서 smoke test다. 로컬 실행형 `runEphemeral()` 검증에 이 요청을 사용하면 안 된다.

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

### 2. 전체 수집

```bash
curl -sS -X POST \
  "http://127.0.0.1:18080/api/v1/organizations/${ORGANIZATION_ID}/sync" \
  -H 'content-type: application/json' \
  --data-binary "{\"facilityCode\":\"${FACILITY_CODE}\",\"mode\":\"full\"}" | jq
```

### 3. 증분 수집

```bash
curl -sS -X POST \
  "http://127.0.0.1:18080/api/v1/organizations/${ORGANIZATION_ID}/sync" \
  -H 'content-type: application/json' \
  --data-binary "{\"facilityCode\":\"${FACILITY_CODE}\",\"mode\":\"delta\"}" | jq
```

### 4. 조회 API

```bash
curl -sS "http://127.0.0.1:18080/api/v1/plants/${FACILITY_CODE}" | jq
curl -sS "http://127.0.0.1:18080/api/v1/plants/${FACILITY_CODE}/generation" | jq
curl -sS "http://127.0.0.1:18080/api/v1/plants/${FACILITY_CODE}/rates" | jq
curl -sS "http://127.0.0.1:18080/api/v1/plants/${FACILITY_CODE}/rec" | jq
curl -sS "http://127.0.0.1:18080/api/v1/plants/${FACILITY_CODE}/documents" | jq
```

### 5. 문서 다운로드

```bash
DOC_ID=$(curl -sS "http://127.0.0.1:18080/api/v1/plants/${FACILITY_CODE}/documents" | jq -r '.[0].id')
curl -OJ "http://127.0.0.1:18080/api/v1/documents/${DOC_ID}/download"
```

### 6. backup admin

```bash
curl -sS -X POST "http://127.0.0.1:18080/api/v1/admin/backups"
curl -sS "http://127.0.0.1:18080/api/v1/admin/backups" | jq
curl -sS "http://127.0.0.1:18080/api/v1/admin/backups/s3" | jq
```

복구 절차는 [backup-restore.md](./backup-restore.md)를 본다.
