# Grant Architecture

> Historical Hiqlite guide. Commands and source references apply only to the [c716877 legacy checkout](https://github.com/mrchypark/grant/tree/c716877e7b970b7d38392b377f64384936992884), not the current native Go runtime. See [current recovery](../../backup-restore.md).

이 repo의 현재 경계는 `grant`다. grant는 조직, 법인 식별, 조직 내부 권한, 조직 간 권한 근거, application consent, service-provider action catalog, authz 판정을 담당한다.

cert-data 수집/저장, 발전소 데이터, 인증서 gateway, plant/document 조회 API는 이 repo의 런타임 경계에서 제거했다.

## 핵심 원칙

- `organizations.id`가 tenant와 권한 평가의 기본 키다.
- 법인/사업자 식별은 `parties`와 `organization_party_links`에 분리한다.
- `service_providers`는 grant, compos, exposure 같은 권한 provider를 선언한다.
- `actions`는 provider별 action catalog다.
- `role_bindings`는 `provider_id + resource_type + resource_id` scope에 role을 부여한다.
- legacy field-view schema와 migration은 history/restore를 위해 보존하지만 coarse runtime dependency가 아니다.
- grant는 외부 provider 리소스 목록을 소유하지 않으므로 unbounded authz list API를 제공하지 않는다.
- 외부 서비스는 후보 리소스를 넘기고 `/api/v1/authz/filter`로 허용된 항목만 걸러간다.

## 코드 기준 구성 요소

- [crates/domain](https://github.com/mrchypark/grant/blob/c716877e7b970b7d38392b377f64384936992884/crates/domain)
  - grant 도메인 모델을 제공한다: `Organization`, `Party`, `ServiceProvider`, `Action`, `Role`, `RoleBinding`, `Application`, `ApplicationRedirectUri`, `ApplicationGrant`, `Agreement`.
- [crates/infra](https://github.com/mrchypark/grant/blob/c716877e7b970b7d38392b377f64384936992884/crates/infra)
  - 설정 로딩, `rauthy | patch-data` identity provider 연동, hiqlite runtime, schema/seed, coarse 권한 판정 service layer를 가진다. startup은 active legacy field/mask policy state를 감지하면 fail closed한다.
- [crates/api](https://github.com/mrchypark/grant/blob/c716877e7b970b7d38392b377f64384936992884/crates/api)
  - `/api/v1/me`, voter-only `POST /api/v1/me/materialize`, `/api/v1/providers`, `/api/v1/orgs/*`, `/api/v1/authz/*`, `/api/v1/application-authz/*`, 단일 `/api/v1/exposure-authz/read`, backup/admin, Rauthy mode의 `/auth/v1/*` proxy 라우트를 제공한다. authorization 응답은 binary-only이며 field-aware 입력은 capability-rejected 된다.
- [crates/app](https://github.com/mrchypark/grant/blob/c716877e7b970b7d38392b377f64384936992884/crates/app)
  - 환경변수로 설정을 읽고 voter/learner 역할에 맞는 Axum router를 띄운다.

## Public API Shape

- `GET /api/v1/me`
- `GET /api/v1/providers`
- `POST /api/v1/admin/providers`
- `PUT /api/v1/admin/providers/{providerId}/actions`
- `GET|POST /api/v1/orgs/{orgId}/apps`
- `GET /api/v1/orgs/{orgId}/apps/{appId}`
- `GET|PUT /api/v1/orgs/{orgId}/apps/{appId}/actions`
- `GET|PUT|DELETE /api/v1/orgs/{orgId}/apps/{appId}/consent`
- `POST /api/v1/admin/orgs/{orgId}/apps/{appId}/provisioning/verify`
- `GET|POST /api/v1/orgs/{orgId}/agreements`
- `GET|PUT /api/v1/orgs/{orgId}/agreements/{agreementId}`
- `GET|POST /api/v1/orgs/{orgId}/roles`
- `PUT /api/v1/orgs/{orgId}/roles/{roleId}/actions`
- `GET|POST /api/v1/orgs/{orgId}/bindings`
- `DELETE /api/v1/orgs/{orgId}/bindings/{bindingId}`
- `POST /api/v1/authz/check`
- `POST /api/v1/authz/batch-check`
- `POST /api/v1/authz/filter`
- `POST /api/v1/application-authz/check`
- `POST /api/v1/application-authz/batch-check`
- `POST /api/v1/application-authz/filter`
- `POST /api/v1/exposure-authz/read`

## Request Flow

1. [crates/app/src/main.rs](https://github.com/mrchypark/grant/blob/c716877e7b970b7d38392b377f64384936992884/crates/app/src/main.rs)가 설정을 읽고 `AppService`를 초기화한다.
2. [crates/api/src/lib.rs](https://github.com/mrchypark/grant/blob/c716877e7b970b7d38392b377f64384936992884/crates/api/src/lib.rs)가 shared read/authz 라우트와 voter-only write/admin 라우트를 조립한다.
3. 사용자 요청은 선택된 provider로 검증한 뒤 grant user/org context로 해석한다. Rauthy는 OAuth2 introspection/OIDC userinfo를 사용하고, Patch Data는 매 요청 내부 auth-refresh를 사용한다.
4. org-scoped mutation은 `organization.application.manage` 또는 `organization.permission.manage` action으로 보호한다.
5. service-to-service admin과 기존 trusted-provider `/authz/*` API는 `x-grant-master-key`로 보호하고, 최종 사용자 토큰을 판정하는 `/application-authz/*` API는 별도 `x-grant-app-auth-key`로 보호한다. 두 key는 서로 달라야 하며 교차 사용할 수 없다.

Developer console은 기존 organization/application 경계 안에 둔다. Rauthy mode의 조직 application 생성은 generated Rauthy client id를 예약할 뿐이며, client 생성·수정과 secret 회전은 별도 운영자/IaC 경계에서 수행한다. Grant backend는 `Clients` 그룹의 `read` 권한만 가진 Rauthy API key로 master provisioning verify의 단일 client 조회만 수행하고, 설정이 local reservation과 일치할 때 application을 `ready`로 전환한다. Patch Data mode의 master-authorized 최초 owner bootstrap은 설정된 고정 exposure application을 같은 트랜잭션에서 생성하며 Rauthy provisioning을 사용하지 않는다.

## Coarse Authorization Deployment Boundary

첫 three-service deploy에서 Grant는 verified subject, action, exact provider/resource scope에 대한
allow/deny만 소유한다. generic membership, consent/trust, roles/actions/bindings, conditions/facts,
agreement expiry와 resource authorization은 유지한다. response에는 field set, view mode, mask가 없다.

`resource.view`가 존재하는 요청은 값의 shape를 파싱하지 않고 여섯 trusted/application authorization
endpoint 모두에서 `400 Bad Request`로 거부한다. field policy management route는 voter와 learner 모두
없으며 `visibilityGranting=true` action mutation도 거부한다. 따라서 schema key/pointer와 field/mask
catalog는 simple deploy의 API/runtime contract가 아니다.

legacy field-view DDL은 이 phase에서 삭제하지 않는다. empty legacy table은 coarse readiness를
막지 않지만 active/nonempty policy/audit row나 알려진 mask/activation state가 있는 migrated database는 startup
blocker다. 해당 database에는 silent dormancy가 아니라 별도 forward/restore 계획이 필요하다.

`POST /api/v1/exposure-authz/read`는 application credential로 검증한 user/org/application에 대해 고정
`provider_id=exposure`, `action_id=exposure.read`, `resource_type=exposureId`, `resource_id=resourceId`를
strong consistency로 판정한다. `artifactDigest`는 exact 71-byte
`sha256:<64 lowercase hex>` literal만 허용하고 normalize 없이 byte-for-byte echo한다.
`exposureVersion`과 `artifactDigest`는 policy key가 아니고 5초 decision의
exact replay binding으로만 echo한다. 이 분리는 Exposure가 publish한 새 immutable version에 기존
exposure permission이 이어지는 V1 tradeoff다. Exposure는 business read 전에 tuple/resource/request binding과
expiry를 모두 비교한다. Grant는 Compos를 조회하지 않고 `resourceVersion`을 만들지 않으며 per-version
grant, field/mask/schema pointer, batch/write는 이 route의 non-goal이다. 보존된 field authorization 설계는
[field-view-authorization-development.md](../.././field-view-authorization-development.md)에 남아 있지만 현재
deploy contract에는 포함되지 않는다.

fresh DB는 Exposure catalog나 grant를 seed하지 않는다. 배포/IaC가 기존 provider/action admin API,
organization role/binding API, application provisioning/action/trust 또는 consent surface로 필요한 state를
명시적으로 만든 뒤에만 allow가 가능하며, 그 전에는 deny가 정상 동작이다.

Patch Data mode에서 외부 경계는 인증에 한정된다. Grant는 매 요청 고정 auth-refresh endpoint로 bearer와
verified identity를 확인하지만 Patch Data `organizations` relation은 최초 master bootstrap/import에서만
선택 조직 검증에 쓴다. 정상 조직 scope, hierarchy, membership, plant permission과 API authorization은
strong local Grant state가 권위다. 서버 설정의 Exposure application과 action/backend mapping만 사용하며
caller가 application, action, provider 또는 backend를 고를 수 없다. Rauthy mode의 기존 client mapping은
그대로 유지된다.

## Voter And Learner

- voter는 schema migration, seed, write/admin endpoint를 처리한다.
- learner는 schema migration과 bootstrap을 수행하지 않고 `/api/v1/me`, provider 조회, org read, authz check/filter 같은 read/authz endpoint를 로컬 read 경로로 처리할 수 있다. `/readyz`는 사용할 read 경로에서 필수 schema를 read-only로 검증한다.
- learner의 write/admin surface는 `requires_voter_node`로 막는다.
- Kubernetes production의 Grant와 Rauthy는 각각 3 voter `StatefulSet`이며 local state는 `emptyDir`이다. 한 노드 손실은 quorum에서 재가입하고 전체 손실은 workload별 S3 backup pair로 복구한다.
- lifecycle gate는 all-empty `steady` cluster의 자동 bootstrap을 막는다. production은 `fresh`를 거부하고 recovery ID가 결합된 restore authorization을 사용한다. Grant shell supervisor와 distroless Rauthy derivative image의 static launcher는 모두 Pod-local health-gated `pending/completed` 상태를 사용한다. Grant는 같은 recovery의 stale `pending`을 재시도하고 Rauthy는 명시적 re-arm 전까지 거부한다. workload ServiceAccount는 상태를 수정할 권한이 없다.
- 복구는 Rauthy 1→3, Grant 1→3 순서이고, voter 복구·schema migration 뒤 learner를 재가입시킨다.

## Related Docs

- schema 상세: [database-schema.md](../.././database-schema.md)
- 권한 모델 상세: [authorization-model.md](../.././authorization-model.md)
- 런타임 모드 상세: [runtime-modes.md](../.././runtime-modes.md)
- 백업/복구 상세: [backup-restore.md](../.././backup-restore.md)
