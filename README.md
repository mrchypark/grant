# grant

`grant`는 조직, 애플리케이션, 서비스 provider, resource scope에 대한 권한 부여와 authz 판정을 담당하는 Rust 서비스다.

핵심 원칙:

- `organizations.id`가 tenant와 권한 평가의 기본 키다.
- `service_providers`는 grant 자체와 compos 같은 외부 provider의 action catalog를 선언한다.
- `role_bindings`는 `provider_id + resource_type + resource_id` scope에 role을 부여한다.
- 외부 서비스는 후보 resource를 넘기고 `/api/v1/authz/filter`로 허용된 항목만 걸러간다.

## Workspace

- `crates/domain`
  - 도메인 타입과 공용 모델
- `crates/infra`
  - 설정, `rauthy | patch-data` identity provider 연동, hiqlite runtime, schema/seed, 권한 판정 서비스 레이어
- `crates/api`
  - Axum HTTP 라우터와 API 계약
- `crates/app`
  - `grant` 실행 바이너리
- `scripts/`
  - `vind` 기반 로컬 bootstrap과 운영 검증 보조 스크립트

## 빠른 시작

로컬 검증은 `vind` 경로만 공식 지원한다.

```bash
docker buildx build --load -t grant:local .
bash ./scripts/ci-local.sh
```

세부 준비물과 phase별 실행 흐름은 [docs/local-development.md](./docs/local-development.md)를 본다.

## 문서

- [아키텍처](./docs/architecture.md)
- [권한 모델](./docs/authorization-model.md)
- [런타임 모드와 읽기 일관성](./docs/runtime-modes.md)
- [로컬 개발과 API smoke](./docs/local-development.md)
- [데이터베이스 스키마](./docs/database-schema.md)
- [백업과 복구](./docs/backup-restore.md)

운영 Kubernetes 배포는 Rauthy provider의 `deploy/overlays/production` 또는 Patch Data provider의 `deploy/overlays/ied-patch-data`를 명시적으로 선택한다. production의 Grant와 Rauthy는 3-node no-PVC HA로 실행되며, 외부 S3 backup pair와 fail-closed lifecycle state가 필수다. 저장소의 production 파일은 ConfigMap/Secret 형식 예시만 제공하고 실제 credential은 포함하지 않는다.

Patch Data mode는 PocketBase 로그인/token verification만 위임한다. 정상 조직 소속, hierarchy, plant permission과 API authorization은 Grant local strong state가 권위이며 Patch Data `organizations` 변경으로 접근을 부여하거나 회수하지 않는다. relation은 master-authorized 최초 bootstrap과 deterministic rule-ledger import 호환성에서만 제한적으로 사용한다.

## 참고

- `crates/app/src/main.rs`는 시작 시 스키마 마이그레이션을 수행하고, voter/learner 역할에 따라 라우터를 선택한다.
- `crates/api/src/lib.rs`는 공용 read 라우트와 voter 전용 write/admin 라우트를 나눠서 노출한다.
- 스키마를 바꾸면 반드시 [docs/database-schema.md](./docs/database-schema.md)도 같이 갱신해야 한다.
