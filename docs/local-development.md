# Local Development And API Smoke

이 문서는 현재 grant 경계 기준의 로컬 검증 가이드다. cert-data 인증서 업로드, 발전소 수집, plant/document 조회 smoke는 이 repo에서 제거했다.

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

docker buildx build --load -t grant:local .
bash ./scripts/vind-cluster.sh ensure-context
bash ./scripts/vind-exec.sh bash ./scripts/vind-bootstrap.sh
```

`vind-bootstrap.sh`는 기본적으로 `deploy/overlays/ephemeral`을 사용한다. 이 overlay는 Rauthy
`/app/data`를 `emptyDir`에 저장하므로 PVC를 만들지 않으며, Rauthy Pod가 교체되면 사용자,
client 변경, session, JWKS가 모두 초기화되고 bootstrap 파일이 다시 적용된다.
local validation Kustomization의 namespace는 `grant`로 고정되어 있으므로 `NAMESPACE`도
`grant`만 허용한다.

production과 ephemeral manifest를 정적으로 확인하려면:

```bash
sh ./scripts/verify-ephemeral-k8s.sh static
```

단일 엔트리포인트로 이미지 빌드와 `vind` bootstrap까지 실행하려면:

```bash
bash ./scripts/ci-local.sh
```

Docker Compose와 Kubernetes bootstrap은 `GRANT_RAUTHY_CLIENT_SECRET` 및
`GRANT_RAUTHY_ADMIN_API_KEY`에서 Rauthy bootstrap 파일을 생성한다. Grant와 Rauthy가 항상 같은
credential을 사용하므로 두 값은 각각 64자 이상의 영숫자 secret을 사용해야 한다.

## 주요 스크립트

- `scripts/ci-local.sh`
  - grant 이미지 빌드와 Rauthy 공식 이미지 pull
  - `vind` cluster lifecycle 관리
  - no-PVC Kubernetes bootstrap, Rauthy Pod 교체와 재-bootstrap smoke 실행
- `scripts/vind-cluster.sh`
  - `vcluster` create/connect/delete
  - control-plane taint normalization
- `scripts/vind-exec.sh`
  - phase 실행 시점에만 `vcluster connect`를 연다
- `scripts/vind-bootstrap.sh`
  - ephemeral Kustomization으로 Kubernetes 리소스 배포
  - Rauthy, grant service rollout 대기
- `scripts/verify-ephemeral-k8s.sh`
  - production PVC와 ephemeral/ied no-PVC render 검증
  - live cluster의 `emptyDir`, health, proxy, bootstrap client, introspection 검증
주의:

- 로컬/CI 회귀 검증은 `vind` 경로만 신뢰한다.
- `deploy/k8s` base는 production용 PVC를 유지한다. 운영 배포에 ephemeral overlay를 사용하지 않는다.
- Docker ARM 환경에서 `vind` create가 platform warning으로 실패하면 `docker pull --platform linux/arm64 alpine:latest` 상태를 먼저 확인한다.

## API Smoke 예시

아래 예시는 service가 로컬 `localhost:18080`으로 포트포워딩된 상태를 전제로 한다.
Rauthy의 OIDC issuer와 redirect URI도 이 origin에 맞춰져 있으므로 브라우저와 smoke 테스트에서
`127.0.0.1:18080` 대신 `localhost:18080`을 사용한다.

필수 변수:

```bash
export BASE_URL=http://localhost:18080
export MASTER_KEY=dev-only-master-key
export ORG_ID=conalog-grant
export TOKEN=<rauthy-access-token>
```

Rauthy로 로그인한 첫 운영자는 master key와 Rauthy bearer token으로 한 번 bootstrap해야 한다.

```bash
curl -sS -X POST "${BASE_URL}/api/v1/admin/bootstrap/rauthy-owner" \
  -H "authorization: Bearer ${TOKEN}" \
  -H "x-grant-master-key: ${MASTER_KEY}" | jq
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
  -H "x-grant-master-key: ${MASTER_KEY}" \
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
  -H "x-grant-master-key: ${MASTER_KEY}" \
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
  -H "x-grant-master-key: ${MASTER_KEY}" \
  -H "content-type: application/json" \
  --data-binary '{
    "subject": {"type": "user", "id": "conalog-grant", "org_id": "conalog-grant"},
    "action": "compos.object.read",
    "resource": {"provider": "compos", "type": "object_instance", "id": "obj_1"}
  }' | jq
```

후보 목록 필터는 `/api/v1/authz/filter`를 사용한다. grant는 외부 provider의 전체 리소스 목록을 소유하지 않으므로 unbounded list API를 제공하지 않는다.

### 5. Backup admin

```bash
curl -sS -X POST "${BASE_URL}/api/v1/admin/backups" \
  -H "x-grant-master-key: ${MASTER_KEY}"
curl -sS "${BASE_URL}/api/v1/admin/backups" \
  -H "x-grant-master-key: ${MASTER_KEY}" | jq
curl -sS "${BASE_URL}/api/v1/admin/backups/s3" \
  -H "x-grant-master-key: ${MASTER_KEY}" | jq
```

복구 절차는 [backup-restore.md](./backup-restore.md)를 본다.
