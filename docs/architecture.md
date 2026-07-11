# Grant Architecture

이 repo의 현재 경계는 `grant`다. grant는 조직, 법인 식별, 조직 내부 권한, 조직 간 권한 근거, application consent, service-provider action catalog, authz 판정을 담당한다.

cert-data 수집/저장, 발전소 데이터, 인증서 gateway, plant/document 조회 API는 이 repo의 런타임 경계에서 제거했다.

## 핵심 원칙

- `organizations.id`가 tenant와 권한 평가의 기본 키다.
- 법인/사업자 식별은 `parties`와 `organization_party_links`에 분리한다.
- `service_providers`는 grant, compos 같은 권한 provider를 선언한다.
- `actions`는 provider별 action catalog다.
- `role_bindings`는 `provider_id + resource_type + resource_id` scope에 role을 부여한다.
- grant는 외부 provider 리소스 목록을 소유하지 않으므로 unbounded authz list API를 제공하지 않는다.
- 외부 서비스는 후보 리소스를 넘기고 `/api/v1/authz/filter`로 허용된 항목만 걸러간다.

## 코드 기준 구성 요소

- `crates/domain`
  - grant 도메인 모델을 제공한다: `Organization`, `Party`, `ServiceProvider`, `Action`, `Role`, `RoleBinding`, `Application`, `ApplicationRedirectUri`, `ApplicationGrant`, `Agreement`.
- `crates/infra`
  - 설정 로딩, Rauthy OIDC identity 연동, hiqlite runtime, schema/seed, 권한 판정 service layer를 가진다.
- `crates/api`
  - `/api/v1/me`, `/api/v1/providers`, `/api/v1/orgs/*`, `/api/v1/authz/*`, backup/admin, Rauthy `/auth/v1/*` proxy 라우트를 제공한다.
- `crates/app`
  - 환경변수로 설정을 읽고 voter/learner 역할에 맞는 Axum router를 띄운다.

## Public API Shape

- `GET /api/v1/me`
- `GET /api/v1/providers`
- `POST /api/v1/admin/providers`
- `PUT /api/v1/admin/providers/{providerId}/actions`
- `GET|POST /api/v1/orgs/{orgId}/apps`
- `GET|PUT /api/v1/orgs/{orgId}/apps/{appId}`
- `GET|PUT /api/v1/orgs/{orgId}/apps/{appId}/actions`
- `GET|POST|DELETE /api/v1/orgs/{orgId}/apps/{appId}/redirect-uris`
- `POST /api/v1/orgs/{orgId}/apps/{appId}/client-secret/rotate`
- `GET|POST /api/v1/orgs/{orgId}/app-grants`
- `PUT /api/v1/orgs/{orgId}/app-grants/{grantId}/actions`
- `POST /api/v1/orgs/{orgId}/app-grants/{grantId}/revoke`
- `GET|POST /api/v1/orgs/{orgId}/agreements`
- `GET|PUT /api/v1/orgs/{orgId}/agreements/{agreementId}`
- `GET|POST /api/v1/orgs/{orgId}/roles`
- `PUT /api/v1/orgs/{orgId}/roles/{roleId}/actions`
- `GET|POST /api/v1/orgs/{orgId}/bindings`
- `DELETE /api/v1/orgs/{orgId}/bindings/{bindingId}`
- `POST /api/v1/authz/check`
- `POST /api/v1/authz/batch-check`
- `POST /api/v1/authz/filter`

## Request Flow

1. `crates/app/src/main.rs`가 설정을 읽고 `AppService`를 초기화한다.
2. `crates/api/src/lib.rs`가 shared read/authz 라우트와 voter-only write/admin 라우트를 조립한다.
3. 사용자 요청은 Rauthy bearer token을 OAuth2 introspection과 OIDC userinfo로 확인한 뒤 grant user/org context로 해석한다.
4. org-scoped mutation은 `organization.application.manage` 또는 `organization.permission.manage` action으로 보호한다.
5. service-to-service admin/authz API는 현재 `x-grant-master-key` header로 보호한다.

Developer console은 기존 organization/application 경계 안에 둔다. 외부 provider는 Grant API만 호출하고, Grant backend가 제한된 Rauthy Admin API key로 해당 application의 Rauthy client redirect URI와 client secret rotation을 동기화한다.

## Voter And Learner

- voter는 schema migration, seed, write/admin endpoint를 처리한다.
- learner는 `/api/v1/me`, provider 조회, org read, authz check/filter 같은 read/authz endpoint를 로컬 read 경로로 처리할 수 있다.
- learner의 write/admin surface는 `requires_voter_node`로 막는다.

## Related Docs

- schema 상세: [database-schema.md](./database-schema.md)
- 권한 모델 상세: [authorization-model.md](./authorization-model.md)
- 런타임 모드 상세: [runtime-modes.md](./runtime-modes.md)
- 백업/복구 상세: [backup-restore.md](./backup-restore.md)
