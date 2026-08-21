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

### 1차 단순 배포 기준

현재 배포 바이너리는 coarse authorization만 수행한다. 판정 key는 검증된 사용자/조직, application context, action, `provider_id`, `resource_type`, `resource_id`의 exact tuple이며 Grant는 schema key, JSON pointer, field set, mask를 입력받거나 반환하지 않는다. Exposure read V1은 이 기존 key를 `provider_id=exposure`, `action_id=exposure.read`, `resource_type=exposureId`, `resource_id=resourceId`로 사용한다. `artifactDigest`는 exact 71 ASCII byte `sha256:<64 lowercase hex>` literal만 받고 normalize 없이 byte-for-byte echo한다. `exposureVersion`과 `artifactDigest`는 새 DB key나 policy row가 아니라 short-lived decision에 exact echo하는 replay binding이다. 따라서 permission은 같은 exposure의 published version에 이어지고, Exposure가 business read 전에 tuple/binding/expiry를 검증한다. 외부 resource의 version이나 digest는 Grant DB의 `created_at`/`updated_at`에서 만들 수 없으므로 Grant는 Compos를 조회하거나 `resourceVersion`을 만들지 않는다.

fresh schema seed는 `grant` provider와 Grant 자체 action만 만든다. `exposure` provider/action, application action/trust 또는 consent, role/action, exact exposure/resource binding은 자동 seed하지 않는다. 운영자가 기존 admin/org API와 application provisioning 절차로 이 state를 명시적으로 만들기 전에는 Exposure read가 deny되는 것이 정상적인 fail-closed 상태다. 이 provisioning은 새 table이나 migration을 요구하지 않는다.

필드 권한용 테이블은 기존 구현과 향후 forward/restore 계획을 위해 물리 스키마에 보존하지만 coarse evaluator와 readiness는 이 테이블을 조회하거나 shape dependency로 사용하지 않는다. 시작 시 migration 전 preflight와 migration 후 검사를 모두 수행한다. `actions.visibility_granting`에 0이 아닌 값이 있거나, `role_action_field_views`, `field_view_policy_audit_outbox` 또는 이미 존재하는 field audit delivery/mask catalog/activation 테이블에 row가 하나라도 있으면 자동 삭제하거나 비활성화하지 않고 시작을 실패한다. 테이블이 없거나 비어 있는 것만 coarse 배포와 호환된다.

## Grant 테이블

### `users`

사용자 기본 프로필. 로그인 주체의 내부 ID, 이메일 스냅샷, 표시 이름, 활성 여부를 저장한다. `users.active`는 Grant의 로컬 kill switch다. identity 동기화나 bootstrap은 기존 사용자의 값을 덮어쓰지 않으며, master 관리자 API만 이 값을 변경한다. 비활성 사용자는 인증 및 모든 권한 판정에서 거부된다.

### `user_identities`

외부 인증 공급자와 내부 사용자 매핑. 런타임 provider는 `rauthy` 또는 `patch-data`이며, `(provider, external_user_id)`는 unique index로 관리한다. Patch Data materialization은 이 provider-qualified key로만 새 local user를 만들며 email이나 bare external id로 기존 user를 연결하지 않는다. 최초 master-authorized external-owner bootstrap만 Patch Data `organizations` relation에서 선택 조직 하나를 검증해 같은 트랜잭션에서 owner membership/binding과 서버 설정 고정 exposure application을 만든다. 정상 요청의 membership과 권한은 Grant local row만 권위다.

### `auth_identity_cache`

외부 토큰 introspection 캐시. Rauthy bearer token 확인 결과와 `client_id`를 저장하며, `token_hash`가 PK이고 `expires_at` 기준으로 만료된다. Patch Data는 이 cache를 사용하지 않고 매 요청 auth-refresh한다. 과거처럼 client 정보가 없는 cache row는 application authz에 사용하지 않는다.

### `parties`

법인/사업자 식별자. `business_no`는 unique이며 `organization.id`와 같은 값으로 보지 않는다.

### `organizations`

테넌트와 권한 평가의 기본 단위. `business_no`는 외부 식별자 스냅샷이고, 내부 canonical key는 `id`다.

### `organization_party_links`

조직과 legal party 연결. 같은 조직의 같은 `relation_type`은 하나만 허용한다.

### `organization_memberships`

사용자와 조직의 소속 관계. `(organization_id, user_id)`는 unique다.

### `patch_data_authority_imports`, `organization_hierarchy_edges`

Patch Data API rule 이동의 deterministic import receipt와 조직 parent-child snapshot이다. receipt는 provider별 exact `generation`, canonical ledger SHA-256 digest, length-framed hierarchy foundation digest, 적용 시각을 저장한다. 같은 ledger receipt 아래 다른 hierarchy input도 conflict다. hierarchy는 source provider 안에서 child당 parent 하나만 허용하며 importer가 sorted/unique/acyclic input을 검증한 뒤 generation 전체를 원자 교체한다. 이 edge는 아직 coarse evaluator 입력이 아니며 membership/plant binding의 parallel authority도 아니다. `authority_import_assertions`는 CAS와 catalog collision을 transaction 안에서 fail closed시키는 CHECK 기반 임시 row이고 성공 시 비워진다.

최초 foundation import는 `patch-data` provider, `patch-data.plant.read` action, global `role-patch-data-plant-reader` role/action을 함께 설치한다. 사용자/조직 membership과 plant resource binding은 승인된 rule-ledger mapper가 기존 canonical `organization_memberships`와 `role_bindings`에 써야 하며 정상 요청 시 Patch Data relation을 dual-read/dual-write하지 않는다.

### `groups`, `group_memberships`

조직 내부 권한 그룹과 membership 연결. `group_memberships.organization_membership_id`는 같은 조직의 active membership만 연결할 수 있다. 조회도 group과 membership의 조직이 같은 row만 반환한다. 그룹 삭제는 `subject_organization_id` 값과 관계없이 해당 group id를 참조하는 group-subject `role_bindings`가 하나라도 있으면 거부되고, 참조가 없을 때 membership과 그룹을 한 트랜잭션에서 삭제한다. 시작 시 missing reference 또는 cross-organization membership/group binding을 발견하면 자동 삭제하지 않고 schema validation을 실패시킨다. 권한 전개 입력이라 grant 경계에 남긴다.

### `service_providers`

grant가 권한 판정 대상으로 인정하는 서비스 제공자 catalog. 기본 provider는 `grant`이며, compos 같은 first-party 서비스도 이 테이블에 등록한 뒤 action manifest를 넣는다.

### `actions`, `roles`, `role_actions`, `role_bindings`

권한 엔진의 원본 데이터. `actions.provider_id`는 action을 제공하는 `service_providers.id`를 가리킨다. `actions.code`는 provider 안에서만 unique이며, 같은 code를 서로 다른 provider가 재사용할 수 있다. 실제 참조 key인 `actions.id`는 전역 PK라 grant provider는 기존처럼 code를 id로 쓰고, 외부 provider action은 provider/code 조합에서 정규화한 id를 사용한다.

coarse 배포에서는 모든 action의 `visibility_granting`이 false여야 한다. bootstrap은 `organization.profile.read`를 포함해 모두 0으로 seed하고, provider action 관리 및 DB write boundary는 `visibility_granting = true` 요청을 write 전에 거부한다. `grantable_to_applications`와 application consent 의미는 그대로 유지한다.

`role_bindings`는 `provider_id`, `resource_type`, `resource_id`를 함께 저장한다. grant 내부 조직 권한은 `provider_id = grant`, `resource_type = organization`, `resource_id = organizations.id`로 표현한다. 일반 외부 리소스는 해당 provider의 resource type/id를 그대로 저장하며, grant는 외부 리소스 존재 여부를 스키마에서 검증하지 않는다. Exposure read V1만 frozen mapping에 따라 `resource_type`에 exact `exposureId`, `resource_id`에 exact `resourceId`를 저장한다. version/digest를 이 두 값에 합성 인코딩하지 않는다.

`agreements`는 별도 테이블로 다시 살아났지만, `role_bindings.agreement_id`는 물리 FK 없이 앱 레벨로 검증한다.
grant 조직 간 권한은 active `agreements`가 있어야 cross-org binding을 만들 수 있다.

과거 `account` provider에 남아 있던 `organization.*` action과 `organization` resource binding은 schema migration에서 `grant` provider로 이동한다. 동일 logical binding이 이미 `grant`에 있으면 legacy `account` row의 active 상태와 agreement 참조를 `grant` row에 병합한 뒤 legacy row를 삭제한다.

### `role_action_field_views`

보존된 field-authorization 스키마다. `(role_id, action_id, resource_type, schema_key, field_pointer)`가 저장 key이고 `view_mode`는 `real` 또는 `anonymized`만 허용한다. role과 action 및 해당 `role_actions` row를 외래 키로 참조하고 `(role_id, action_id, resource_type, schema_key)` 조회 index를 유지한다. 현재 coarse 배포에는 이 테이블의 관리 API, evaluator, startup cleanup이 없으며 row가 하나라도 있으면 호환성 검사가 시작을 막는다. 따라서 이 설명은 보존된 row shape과 관계를 문서화할 뿐 현재 활성 정책 계약을 뜻하지 않는다.

### `field_view_policy_audit_outbox`

보존된 필드 열람 정책 교체 audit outbox 스키마다. `event_id`가 primary key이고 `idempotency_key`는 unique이며 요청 hash, 조직/행위자/role/action 범위, resource/schema key, before/after JSON snapshot과 생성 시각 컬럼을 유지한다. 현재 coarse 배포는 이 outbox에 쓰거나 delivery worker를 시작하지 않으며 row가 하나라도 있으면 시작을 실패한다.

### `applications`, `application_redirect_uris`, `application_actions`, `application_grants`, `application_grant_actions`

API/OAuth application와 사용자 consent grant. active grant unique 제약과 action subset 검증 때문에 grant 경계에 남긴다.

`applications.rauthy_client_id`는 Rauthy client id와 연결된다. 조직 `POST /apps`는 사용자 application id를 Rauthy id로 재사용하지 않고 충돌 가능성이 사실상 없는 `grant-<uuid>` id를 서버에서 생성하며, `provisioning_status = unprovisioned`인 local reservation만 저장한다. Grant는 Rauthy client를 생성·수정하거나 secret을 회전하지 않는다. 운영자/IaC가 그 generated id와 반환된 application 설정으로 Rauthy client를 먼저 만든 뒤, master provisioning verify API가 read-only Rauthy GET으로 id, name, enabled/confidential, redirect URI, flow 설정을 정확히 검증해야 `ready`로 전환된다. 기존 application id를 client id로 backfill하지 않으며 legacy row는 `unprovisioned`로 남는다.

`applications.rauthy_client_id`는 Rauthy client와 연결된 실제 값에 대해 전역 partial unique index를 가진다. legacy row의 `NULL`, 빈 문자열, SQLite 기본 `TRIM` 기준의 ASCII space-only 문자열은 미연결 값으로 보아 index와 migration 중복 검사에서 제외한다. 기본 `TRIM`이 제거하지 않는 탭 등의 문자는 실제 client id 문자로 간주해 unique 제약을 적용한다. application authz는 토큰에서 검증한 비어 있지 않은 `client_id`를 이 컬럼의 `ready` application에 정확히 하나 매핑하며, 같은 조직의 active/ready application이 아니면 fail closed한다.

`first_party = 1`과 `consent_policy = auto_approve`의 조합만 trusted application이다. 조직 API는 이 두 값을 생성·수정하지 못하며 master trust API만 동시에 승격하거나 해제한다. trust 승격은 기존 active explicit grant를 같은 트랜잭션에서 revoke해 trusted 상태에 explicit grant가 남지 않게 한다. trusted application의 조직 보안 설정 변경은 write boundary에서 거부된다. `security_mutation_token`과 `security_mutation_expires_at`은 action set 같은 권한 상태 변경과 trust 변경을 직렬화하는 10분 lease다. 정상·실패 경로에서는 token을 해제하고, 프로세스 중단 시에는 만료 뒤 다음 변경이 stale lease를 회수한다.

`application_grants`는 `(user_id, application_id, organization_id)`의 active row를 하나만 허용한다. 사용자 self-consent PUT은 body의 action 목록만 받아 exact 로그인 사용자에 대해 active grant를 natural-key upsert하고 grant/action 전체 교체를 원자적으로 수행한다. 같은 사용자의 첫 PUT이 동시에 실행되어도 partial unique key의 conflict update로 하나의 active grant에 수렴한다. application이 same-org active explicit non-trusted 상태이고 사용자가 active member이며 action이 현재 `application_actions`의 exact subset인지는 같은 트랜잭션에서 재검증한다. 최대 999개 action은 250개씩 multi-row insert해 Raft transaction statement 수를 제한한다. `application_consent_assertions`는 consent와 application security/trust 트랜잭션의 CHECK 기반 내부 assertion용 임시 row만 담고 성공 시 즉시 비워지며, 실제 action id를 sentinel로 사용하지 않는다. application action을 축소하면 stale grant action도 같은 트랜잭션에서 제거되며, 권한 판정도 항상 현재 `application_actions`와 다시 join한다. revoke는 active grant를 idempotent하게 revoked 상태로 바꾼다.

`application_redirect_uris`는 provisioning manifest를 위한 application별 redirect URI 목록이며 key는 `(application_id, redirect_uri)`다. 조직 `POST /apps`는 선택적 단일 `redirectUri`만 받고 application row와 같은 transaction에서 0 또는 1개 row를 저장한다. 생략하면 legacy와 동일한 빈 manifest다. URI는 canonical absolute HTTPS 또는 explicit-port loopback-IP HTTP만 허용하며, 조직 redirect mutation API는 없다. verify는 strong read로 local row가 정확히 0 또는 1개인지와 유효성을 확인하고 Rauthy의 redirect URI 배열이 빈 배열 또는 canonical 저장값 하나와 byte-for-byte 일치할 때만 ready로 전환한다.

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
- `role_action_field_views.role_id` -> `roles.id`
- `role_action_field_views.action_id` -> `actions.id`
- `role_action_field_views.(role_id, action_id)` -> `role_actions.(role_id, action_id)`
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
- coarse readiness는 core authorization schema만 확인한다. 보존 field table의 존재, 누락, shape는 readiness 조건이 아니고 활성 row 여부는 별도 startup compatibility 검사에서 fail closed한다.
- field 정책이 활성화됐던 migrated DB는 coarse 바이너리가 자동 정리하지 않는다. 배포 전에 별도의 forward migration 또는 검증된 restore 계획이 필요하다.
