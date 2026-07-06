# Local Development And API Smoke

이 문서는 현재 account 경계 기준의 로컬 검증 가이드다. cert-data 인증서 업로드, 발전소 수집, plant/document 조회 smoke는 이 repo에서 제거했다.

## 준비물

- Rust toolchain
- Docker
- `kubectl`
- `vcluster`
- `curl`
- `jq`

## 권장 검증 순서

```bash
cargo test --workspace

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
  - MinIO, TrailBase, account service rollout 대기
- `scripts/vind-chaos.sh`
  - `vind` 컨텍스트에서 reduced chaos 시나리오를 실행한다

주의:

- 로컬/CI 회귀 검증은 `vind` 경로만 신뢰한다.
- Colima ARM 환경에서 `vind` create가 platform warning으로 실패하면 `docker pull --platform linux/arm64 alpine:latest` 상태를 먼저 확인한다.

## API Smoke 예시

`scripts/vind-bootstrap.sh`와 `scripts/k3d-e2e.sh`까지 끝나서 service가 로컬 `127.0.0.1:18080`으로 포트포워딩된 상태를 전제로 한다.

필수 변수:

```bash
export BASE_URL=http://127.0.0.1:18080
export MASTER_KEY=dev-only-master-key
export ORG_ID=conalog-account
export TOKEN=conalog-account
```

### 1. 현재 사용자

```bash
curl -sS "${BASE_URL}/api/v1/me" \
  -H "authorization: Bearer ${TOKEN}" \
  -H "x-organization-id: ${ORG_ID}" | jq
```

### 2. Provider 등록

```bash
curl -sS -X POST "${BASE_URL}/api/v1/admin/providers" \
  -H "x-certd-master-key: ${MASTER_KEY}" \
  -H "content-type: application/json" \
  --data-binary '{
    "id": "compos",
    "name": "Compos",
    "description": "Object data provider",
    "active": true
  }' | jq
```

### 3. Provider action manifest 교체

```bash
curl -sS -X PUT "${BASE_URL}/api/v1/admin/providers/compos/actions" \
  -H "x-certd-master-key: ${MASTER_KEY}" \
  -H "content-type: application/json" \
  --data-binary '{
    "actions": [{
      "id": "compos.object.read",
      "code": "compos.object.read",
      "name": "Read Compos Object",
      "description": null,
      "riskLevel": "low",
      "visibilityGranting": true,
      "grantableToApplications": true,
      "consentLabel": "Read Compos Object",
      "consentDescription": null
    }]
  }' | jq
```

`PUT`은 manifest replace다. 요청에서 빠진 provider action은 role/application grant 참조와 함께 제거된다.

### 4. Authz 판정

```bash
curl -sS -X POST "${BASE_URL}/api/v1/authz/check" \
  -H "x-certd-master-key: ${MASTER_KEY}" \
  -H "content-type: application/json" \
  --data-binary '{
    "subject": {"type": "user", "id": "conalog-account", "org_id": "conalog-account"},
    "action": "compos.object.read",
    "resource": {"provider": "compos", "type": "object_instance", "id": "obj_1"}
  }' | jq
```

후보 목록 필터는 `/api/v1/authz/filter`를 사용한다. account는 외부 provider의 전체 리소스 목록을 소유하지 않으므로 unbounded list API를 제공하지 않는다.

### 5. Backup admin

```bash
curl -sS -X POST "${BASE_URL}/api/v1/admin/backups" \
  -H "x-certd-master-key: ${MASTER_KEY}"
curl -sS "${BASE_URL}/api/v1/admin/backups" \
  -H "x-certd-master-key: ${MASTER_KEY}" | jq
curl -sS "${BASE_URL}/api/v1/admin/backups/s3" \
  -H "x-certd-master-key: ${MASTER_KEY}" | jq
```

복구 절차는 [backup-restore.md](./backup-restore.md)를 본다.
