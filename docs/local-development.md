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

coarse authorization deploy boundary를 확인할 때는 전체 workspace test 전에 아래 focused test를 실행한다.
binary allow/deny, agreement expiry, 모든 endpoint의 field-aware rejection, field policy route 부재,
`visibilityGranting=true`의 no-mutation rejection을 검증한다.

```bash
cargo test -p infra --test database_schema
cargo test -p infra --test authz_activity
cargo test -p api --test field_view_admin
cargo test -p api --test field_view_authz
cargo test -p api --test organization_authorization authz_check_and_filter_use_role_binding
cargo test -p api --test organization_authorization expired_agreement_binding_denies_coarse_authorization
```

현재 local 구현은 coarse evaluator와 binary trusted/application authz check/batch/filter를 포함한다.
`resource.view` 값은 schema/pointer로 해석하지 않고 key 존재만으로 거부한다. legacy field-view table은
삭제하지 않지만 empty table은 runtime/readiness dependency가 아니며 active migrated state는 startup에서
fail closed한다. Exposure V1은 `exposureId`, 양의 `exposureVersion`, 정확한
`sha256:<64 lowercase hex>` `artifactDigest`, `resourceId`, opaque `requestBinding`의 exact tuple을
decision replay binding으로 사용한다. policy key는 version/digest와 분리되며, Grant는 accepted digest를
byte-for-byte echo한다. Exposure는 business data read 전에 echo field와 request binding, 5초 freshness
envelope의 `expiresAt`을 원 요청과 비교해야 하며 다른 tuple/resource에 decision을 재사용할 수 없다.

```bash
cargo test -p api --test exposure_authz
cargo test -p api --test organization_members
```

`vind-bootstrap.sh`는 기본적으로 `deploy/overlays/ephemeral`을 사용한다. 이 overlay는 Grant와
Rauthy를 각각 3 voter, `emptyDir`로 실행하므로 PVC를 만들지 않는다. 한 Rauthy Pod 교체는 남은
quorum에서 재가입하며 identity/JWKS를 초기화하지 않는다. 세 Pod를 동시에 잃으면 로컬 `fresh`
mode가 bootstrap 데이터를 사용해 새 개발 cluster를 만든다. 이 동작은 production의 fail-closed
`steady` mode와 다르다.
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

저장소의 Rauthy launcher digest가 아직 publish 전 marker라면 이 명령은 Docker build나 image pull 전에
실패한다. merge 후 발행된 immutable digest가 있지만 manifest pin 후속 PR 전이라면 명시적으로 override한다.

```bash
RAUTHY_IMAGE='ghcr.io/conalog/grant-rauthy-launcher:rauthy-0.36.0@sha256:<64-hex-digest>' \
  bash ./scripts/ci-local.sh
```

GitHub PR workflow는 marker 상태에서 source/contract 정적 검증은 수행하되 runtime bootstrap은 계속 skip한다.

Docker Compose와 Kubernetes bootstrap은 `GRANT_RAUTHY_CLIENT_SECRET` 및
`GRANT_RAUTHY_CLIENT_READ_API_KEY`에서 읽기 전용 Rauthy bootstrap 파일을 생성한다. Grant와 Rauthy가 항상 같은
credential을 사용하므로 두 값은 각각 64자 이상의 영숫자 secret을 사용해야 한다.
Grant는 admin용 `GRANT_MASTER_KEY`와 application-authz용 `GRANT_APP_AUTH_KEY`도 요구하며 두 값은
반드시 서로 달라야 한다. 로컬 bootstrap script는 구분된 개발 기본값을 secret에 넣는다.

## 주요 스크립트

- `scripts/ci-local.sh`
  - grant 이미지 빌드와 Rauthy 공식 이미지 pull
  - `vind` cluster lifecycle 관리
  - no-PVC Kubernetes bootstrap, Rauthy Pod 교체와 quorum 재가입 smoke 실행
- `scripts/vind-cluster.sh`
  - `vcluster` create/connect/delete
  - control-plane taint normalization
- `scripts/vind-exec.sh`
  - phase 실행 시점에만 `vcluster connect`를 연다
- `scripts/vind-bootstrap.sh`
  - ephemeral Kustomization으로 Kubernetes 리소스 배포
  - Rauthy, grant service rollout 대기
- `scripts/verify-ephemeral-k8s.sh`
  - base/ephemeral/ied/production의 no-PVC 3-node HA render 검증
  - production hard spread, Grant/Rauthy derivative digest, 외부 backup/lifecycle 참조와 restore dry-run 검증
  - live cluster의 `emptyDir`, health, proxy, bootstrap client, introspection 검증
- `deploy/rauthy-launcher`, `.github/workflows/rauthy-launcher-image.yml`
  - distroless Rauthy용 static restore/startup supervisor와 amd64/arm64 image 계약 검증
  - GHCR publish는 수동 confirmation 뒤에만 수행하며 출력 manifest digest를 StatefulSet에 pin해야 한다.

private launcher image를 사용하는 local bootstrap에는 package read 권한만 가진 credential이 필요하다.

```bash
export GHCR_USERNAME='<github-user-or-app>'
export GHCR_TOKEN='<read-packages-only-token>'
bash scripts/ci-local.sh
```

script는 token을 argv에 넣지 않고 임시 Docker config와 namespace `rauthy-launcher-pull` Secret을 만든다. credential이 없으면 public image로 fallback하지 않고 실패한다.
`RAUTHY_IMAGE`를 이 private launcher repository가 아닌 public/custom image로 명시하면 `ci-local.sh`는 `GHCR_*`를 요구하지 않고 현재 Docker context의 기본 credential로 pull한다. `ghcr.io/conalog/grant-rauthy-launcher`의 bare, `:tag`, `@digest`, `:tag@digest` 형식은 모두 private 경로로 분류되므로 이 분기를 사용할 수 없으며 credential 누락 시 계속 실패한다.

GKE의 `scripts/gke-skaffold-run.sh`도 같은 입력 계약을 사용한다. script는 manifest에 고정된 launcher image를 임의의 `RAUTHY_IMAGE`로 우회하지 못하게 하고, publish 전 digest marker나 immutable digest가 아닌 image를 Docker 및 cluster 접근 전에 거부한다. credential 검증과 GHCR login이 성공하면 raw `GHCR_USERNAME`과 `GHCR_TOKEN`을 즉시 환경에서 제거한다. 임시 Docker config로 namespace의 `rauthy-launcher-pull`을 적용한 직후 config와 그 환경 경로를 삭제하고 Grant Secret과 Skaffold에는 전달하지 않는다. Skaffold는 추적 가능한 child PID로 실행하며 HUP, INT, TERM을 받으면 같은 신호를 child에 전달하고 최대 2초 기다린다. child가 신호를 무시하면 KILL 후 회수하며 wrapper는 각각 129, 130, 143으로 종료한다. 정상 종료도 Skaffold 상태를 보존한다. 따라서 새 namespace를 배포할 때도 위 두 변수를 먼저 설정해야 하며, Secret을 별도로 수동 생성할 필요는 없다.

- `scripts/create-recovery-pair.sh`, `scripts/restore-full-cluster.sh`
  - exact S3 version recovery record 생성과 전체 cluster restore
  - 로컬에서는 `--dry-run`만 사용하고 실제 복구는 보호된 운영 절차를 따른다.
주의:

- 로컬/CI 회귀 검증은 `vind` 경로만 신뢰한다.
- `deploy/k8s` base도 no-PVC지만 로컬 bootstrap용 `fresh` mode와 개발 설정을 포함한다. 운영에는 외부 Secret/ConfigMap과 `steady` gate를 강제하는 `deploy/overlays/production`만 사용한다.
- Docker ARM 환경에서 `vind` create가 platform warning으로 실패하면 `docker pull --platform linux/arm64 alpine:latest` 상태를 먼저 확인한다.

## API Smoke 예시

아래 예시는 service가 로컬 `localhost:18080`으로 포트포워딩된 상태를 전제로 한다.
Rauthy의 OIDC issuer와 redirect URI도 이 origin에 맞춰져 있으므로 브라우저와 smoke 테스트에서
`127.0.0.1:18080` 대신 `localhost:18080`을 사용한다.

필수 변수:

```bash
export BASE_URL=http://localhost:18080
export MASTER_KEY=local-dev-master-key
export APP_AUTH_KEY=local-dev-app-auth-key
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
      "visibilityGranting": false,
      "grantableToApplications": true,
      "consentLabel": "Read Compos Object",
      "consentDescription": null
    }]
  }' | jq
```

이 action은 field/view 권한을 부여하지 않는 coarse authorization용이다. `visibilityGranting=true`는
mutation 전에 거부되므로 catalog provisioning에 사용할 수 없다.

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
