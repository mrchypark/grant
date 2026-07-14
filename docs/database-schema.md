# Database Schema

이 문서는 현재 [`crates/infra/src/database/mod.rs`](../crates/infra/src/database/mod.rs)와 [`schema.rs`](../crates/infra/src/database/schema.rs) 기준의 실제 스키마를 설명한다.

## 프로젝트 경계

현재 repo의 프로젝트 경계는 `grant`다. `grant`는 조직, 법인 식별, 조직 내부 권한, 조직 간 권한 근거, application consent, service-provider action catalog, authz 판정을 가진다.

공동인증서를 받아 발전소 데이터를 수집/저장하는 `cert-data` 경계는 이 repo에서 제거한다.

## 핵심 원칙

- `organization`이 테넌트 겸 권한 평가의 기본 단위다.
- `organization.id`와 `business_no`는 다른 값이다. 법인 식별은 `parties`와 `organization_party_links`로 분리한다.
- 로그인 주체(`users`, `user_identities`, `auth_identity_cache`)는 identity/authz 보안 경계라 동적 object로 내리지 않는다.
- 공동인증서 credential, 저장 인증서, gateway challenge, crawl task token은 `cert-data` 경계라 이 repo에서 제거한다.
- 권한 원본은 `role_bindings`다. plant/asset 전개 cache였던 `effective_*` 테이블은 cert-data 분리와 함께 제거한다.
- `service_providers`가 compos 같은 외부 서비스와 grant 자체의 action catalog 경계를 선언한다.
- SQLite/hiqlite 위에서 앱 레벨 무결성과 권한 전개를 사용하므로, 현재 스키마는 물리 FK를 거의 두지 않는다.

## Grant 테이블

### `users`

사용자 기본 프로필. 로그인 주체의 내부 ID, 이메일 스냅샷, 표시 이름, 활성 여부를 저장한다. `users.active`는 Grant의 로컬 kill switch다. identity 동기화나 bootstrap은 기존 사용자의 값을 덮어쓰지 않으며, master 관리자 API만 이 값을 변경한다. 비활성 사용자는 인증 및 모든 권한 판정에서 거부된다.

### `user_identities`

외부 인증 공급자와 내부 사용자 매핑. 현재 런타임 provider 값은 Rauthy 연동의 `rauthy`이며, `(provider, external_user_id)`는 unique index로 관리한다.

### `auth_identity_cache`

외부 토큰 introspection 캐시. 현재 Rauthy bearer token 확인 결과와 `client_id`를 저장하며, `token_hash`가 PK이고 `expires_at` 기준으로 만료된다. 과거처럼 client 정보가 없는 cache row는 application authz에 사용하지 않는다.

### `parties`

법인/사업자 식별자. `business_no`는 unique이며 `organization.id`와 같은 값으로 보지 않는다.

### `organizations`

테넌트와 권한 평가의 기본 단위. `business_no`는 외부 식별자 스냅샷이고, 내부 canonical key는 `id`다.

### `organization_party_links`

조직과 legal party 연결. 같은 조직의 같은 `relation_type`은 하나만 허용한다.

### `organization_memberships`

사용자와 조직의 소속 관계. `(organization_id, user_id)`는 unique다.

### `groups`, `group_memberships`

조직 내부 권한 그룹과 membership 연결. `group_memberships.organization_membership_id`는 같은 조직의 active membership만 연결할 수 있다. 조회도 group과 membership의 조직이 같은 row만 반환한다. 그룹 삭제는 `subject_organization_id` 값과 관계없이 해당 group id를 참조하는 group-subject `role_bindings`가 하나라도 있으면 거부되고, 참조가 없을 때 membership과 그룹을 한 트랜잭션에서 삭제한다. 시작 시 missing reference 또는 cross-organization membership/group binding을 발견하면 자동 삭제하지 않고 schema validation을 실패시킨다. 권한 전개 입력이라 grant 경계에 남긴다.

### `service_providers`

grant가 권한 판정 대상으로 인정하는 서비스 제공자 catalog. 기본 provider는 `grant`이며, compos 같은 first-party 서비스도 이 테이블에 등록한 뒤 action manifest를 넣는다.

### `actions`, `roles`, `role_actions`, `role_bindings`

권한 엔진의 원본 데이터. `actions.provider_id`는 action을 제공하는 `service_providers.id`를 가리킨다. `actions.code`는 provider 안에서만 unique이며, 같은 code를 서로 다른 provider가 재사용할 수 있다. 실제 참조 key인 `actions.id`는 전역 PK라 grant provider는 기존처럼 code를 id로 쓰고, 외부 provider action은 provider/code 조합에서 정규화한 id를 사용한다.

`role_bindings`는 `provider_id`, `resource_type`, `resource_id`를 함께 저장한다. grant 내부 조직 권한은 `provider_id = grant`, `resource_type = organization`, `resource_id = organizations.id`로 표현한다. compos 리소스는 해당 provider의 resource type/id를 그대로 저장하며, grant는 외부 리소스 존재 여부를 스키마에서 검증하지 않는다.

`agreements`는 별도 테이블로 다시 살아났지만, `role_bindings.agreement_id`는 물리 FK 없이 앱 레벨로 검증한다.
grant 조직 간 권한은 active `agreements`가 있어야 cross-org binding을 만들 수 있다.

과거 `account` provider에 남아 있던 `organization.*` action과 `organization` resource binding은 schema migration에서 `grant` provider로 이동한다. 동일 logical binding이 이미 `grant`에 있으면 legacy `account` row의 active 상태와 agreement 참조를 `grant` row에 병합한 뒤 legacy row를 삭제한다.

### `applications`, `application_redirect_uris`, `application_actions`, `application_grants`, `application_grant_actions`

API/OAuth application와 사용자 consent grant. active grant unique 제약과 action subset 검증 때문에 grant 경계에 남긴다.

`applications.rauthy_client_id`는 Rauthy client id와 연결된다. 조직 `POST /apps`는 사용자 application id를 Rauthy id로 재사용하지 않고 충돌 가능성이 사실상 없는 `grant-<uuid>` id를 서버에서 생성하며, `provisioning_status = unprovisioned`인 local reservation만 저장한다. Grant는 Rauthy client를 생성·수정하거나 secret을 회전하지 않는다. 운영자/IaC가 그 generated id와 반환된 application 설정으로 Rauthy client를 먼저 만든 뒤, master provisioning verify API가 read-only Rauthy GET으로 id, name, enabled/confidential, redirect URI, flow 설정을 정확히 검증해야 `ready`로 전환된다. 기존 application id를 client id로 backfill하지 않으며 legacy row는 `unprovisioned`로 남는다.

`applications.rauthy_client_id`는 Rauthy client와 연결된 실제 값에 대해 전역 partial unique index를 가진다. legacy row의 `NULL`, 빈 문자열, SQLite 기본 `TRIM` 기준의 ASCII space-only 문자열은 미연결 값으로 보아 index와 migration 중복 검사에서 제외한다. 기본 `TRIM`이 제거하지 않는 탭 등의 문자는 실제 client id 문자로 간주해 unique 제약을 적용한다. application authz는 토큰에서 검증한 비어 있지 않은 `client_id`를 이 컬럼의 `ready` application에 정확히 하나 매핑하며, 같은 조직의 active/ready application이 아니면 fail closed한다.

`first_party = 1`과 `consent_policy = auto_approve`의 조합만 trusted application이다. 조직 API는 이 두 값을 생성·수정하지 못하며 master trust API만 동시에 승격하거나 해제한다. trust 승격은 기존 active explicit grant를 같은 트랜잭션에서 revoke해 trusted 상태에 explicit grant가 남지 않게 한다. trusted application의 조직 보안 설정 변경은 write boundary에서 거부된다. `security_mutation_token`과 `security_mutation_expires_at`은 action set 같은 권한 상태 변경과 trust 변경을 직렬화하는 10분 lease다. 정상·실패 경로에서는 token을 해제하고, 프로세스 중단 시에는 만료 뒤 다음 변경이 stale lease를 회수한다.

`application_grants`는 `(user_id, application_id, organization_id)`의 active row를 하나만 허용한다. 사용자 self-consent PUT은 body의 action 목록만 받아 exact 로그인 사용자에 대해 active grant를 natural-key upsert하고 grant/action 전체 교체를 원자적으로 수행한다. 같은 사용자의 첫 PUT이 동시에 실행되어도 partial unique key의 conflict update로 하나의 active grant에 수렴한다. application이 same-org active explicit non-trusted 상태이고 사용자가 active member이며 action이 현재 `application_actions`의 exact subset인지는 같은 트랜잭션에서 재검증한다. 최대 999개 action은 250개씩 multi-row insert해 Raft transaction statement 수를 제한한다. `application_consent_assertions`는 consent와 application security/trust 트랜잭션의 CHECK 기반 내부 assertion용 임시 row만 담고 성공 시 즉시 비워지며, 실제 action id를 sentinel로 사용하지 않는다. application action을 축소하면 stale grant action도 같은 트랜잭션에서 제거되며, 권한 판정도 항상 현재 `application_actions`와 다시 join한다. revoke는 active grant를 idempotent하게 revoked 상태로 바꾼다.

`application_redirect_uris`는 향후 검증 가능한 provisioning manifest를 위한 application별 redirect URI 목록이며 key는 `(application_id, redirect_uri)`다. 현재 release는 조직 redirect mutation을 노출하지 않으므로 reservation은 빈 목록을 의도 설정으로 사용하고 verify도 Rauthy의 빈 redirect 목록을 요구한다.

## 제거된 테이블

`ensure_schema_only()`는 아래 테이블을 더 이상 생성하지 않고, 기존 DB에 남아 있으면 삭제한다. 이 테이블들이 사용하던 `plant.*`, `asset.*`, `document.*`, `cert-data.*`, `cert_data.*`, `organization.plant.*`, `organization.credential.use` action catalog와 role/application 참조도 함께 삭제한다. 이 cleanup은 과거 이름인 `account` provider에 남아 있는 동일 범위의 stale 권한 row도 함께 제거한다.

- `agreement_documents`
- `assets`
- `asset_profiles`
- `registries`
- `registry_accounts`
- `registry_projects`
- `data_sources`
- `generation_data`
- `rate_data`
- `rate_data_old_real_value`
- `energy_certificates`
- `auth_credentials`
- `certificates`
- `cert_gateway_challenges`
- `crawl_task_tokens`
- `credential_bindings`
- `country_capabilities`
- `sync_runs`
- `sync_run_items`
- `effective_organization_asset_actions`
- `effective_user_asset_actions`
- `effective_user_asset_visibility`
- `effective_organization_plant_actions`
- `effective_user_plant_actions`
- `effective_user_plant_visibility`
- `plants`
- `generation_records`
- `rate_records`
- `rec_records`
- `documents`

## 주요 릴레이션

- `user_identities.user_id` -> `users.id`
- `organization_memberships.user_id` -> `users.id`
- `organization_memberships.organization_id` -> `organizations.id`
- `organization_party_links.organization_id` -> `organizations.id`
- `organization_party_links.party_id` -> `parties.id`
- `groups.organization_id` -> `organizations.id`
- `group_memberships.group_id` -> `groups.id`
- `group_memberships.organization_membership_id` -> `organization_memberships.id`
- `agreements.grantor_organization_id` -> `organizations.id`
- `agreements.grantee_organization_id` -> `organizations.id`
- `actions.provider_id` -> `service_providers.id`
- `roles.organization_id` -> `organizations.id`
- `role_actions.role_id` -> `roles.id`
- `role_actions.action_id` -> `actions.id`
- `role_bindings.role_id` -> `roles.id`
- `role_bindings.agreement_id` -> `agreements.id`
- `role_bindings.subject_organization_id` -> `organizations.id`
- `role_bindings.provider_id` -> `service_providers.id`
- `application_redirect_uris.application_id` -> `applications.id`
- `application_actions.application_id` -> `applications.id`
- `application_actions.action_id` -> `actions.id`
- `application_grants.user_id` -> `users.id`
- `application_grants.application_id` -> `applications.id`
- `application_grants.organization_id` -> `organizations.id`
- `application_grant_actions.application_grant_id` -> `application_grants.id`
- `application_grant_actions.action_id` -> `actions.id`

## 주의할 점

- `cert-data` 저장/실행 모델은 별도 프로젝트에서 다시 정한다.
- `role_bindings.resource_type`은 `provider_id`와 함께 해석한다. `grant/organization`만 grant가 직접 존재 검증하고, 외부 provider resource는 해당 서비스가 후보 목록을 넘긴 뒤 `/api/v1/authz/filter`로 걸러야 한다.
