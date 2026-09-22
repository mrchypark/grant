# Database Schema

이 문서는 현재 [`internal/grant/store_schema.sql`](../internal/grant/store_schema.sql)을 기준으로 실제 스키마를 설명한다.

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
- Rhiza의 SQLite materializer 위에서 애플리케이션이 무결성과 권한 전개를 관리하며, 필요한 관계에는 물리 FK를 사용한다.

### 1차 단순 배포 기준

현재 배포 바이너리는 coarse authorization만 수행한다. 판정 key는 검증된 사용자/조직, application context, action, `provider_id`, `resource_type`, `resource_id`의 exact tuple이며 Grant는 schema key, JSON pointer, field set, mask를 입력받거나 반환하지 않는다. Exposure read V1은 이 기존 key를 `provider_id=exposure`, `action_id=exposure.read`, `resource_type=exposureId`, `resource_id=resourceId`로 사용한다. `artifactDigest`는 exact 71 ASCII byte `sha256:<64 lowercase hex>` literal만 받고 normalize 없이 byte-for-byte echo한다. `exposureVersion`과 `artifactDigest`는 새 DB key나 policy row가 아니라 short-lived decision에 exact echo하는 replay binding이다. 따라서 permission은 같은 exposure의 published version에 이어지고, Exposure가 business read 전에 tuple/binding/expiry를 검증한다. 외부 resource의 version이나 digest는 Grant DB의 `created_at`/`updated_at`에서 만들 수 없으므로 Grant는 Compos를 조회하거나 `resourceVersion`을 만들지 않는다.

fresh schema seed는 `grant` provider와 Grant 자체 action만 만든다. `exposure` provider/action, application action/trust 또는 consent, role/action, exact exposure/resource binding은 자동 seed하지 않는다. 운영자가 기존 admin/org API와 application provisioning 절차로 이 state를 명시적으로 만들기 전에는 Exposure read가 deny되는 것이 정상적인 fail-closed 상태다. 이 provisioning은 새 table이나 migration을 요구하지 않는다.

필드 권한용 테이블은 기존 구현과 향후 forward/restore 계획을 위해 물리 스키마에 보존하지만 coarse evaluator와 readiness는 이 테이블을 조회하거나 shape dependency로 사용하지 않는다. 시작 시 migration 전 preflight와 migration 후 검사를 모두 수행한다. `actions.visibility_granting`에 0이 아닌 값이 있거나, `role_action_field_views`, `field_view_policy_audit_outbox` 또는 이미 존재하는 field audit delivery/mask catalog/activation 테이블에 row가 하나라도 있으면 자동 삭제하거나 비활성화하지 않고 시작을 실패한다. 테이블이 없거나 비어 있는 것만 coarse 배포와 호환된다.

## Grant 테이블

### `users`

사용자 기본 프로필. 로그인 주체의 내부 ID, 이메일 스냅샷, 표시 이름, 활성 여부를 저장한다. `users.active`는 Grant의 로컬 kill switch다. identity 동기화나 bootstrap은 기존 사용자의 값을 덮어쓰지 않으며, master 관리자 API만 이 값을 변경한다. 비활성 사용자는 인증 및 모든 권한 판정에서 거부된다.

### `user_identities`

외부 인증 공급자와 내부 사용자 매핑. unique key는 `(provider, issuer, collection_id, external_user_id)`다. PocketBase는 `provider=pocketbase`, 설정된 canonical origin인 `issuer`, auth-refresh로 검증한 `collection_id`와 raw record ID를 모두 사용한다. Rauthy는 기존 subject 의미를 유지하고 issuer/collection은 빈 문자열이다. CHECK 제약은 PocketBase의 두 namespace 값이 모두 있어야 하고 다른 provider에서는 모두 비어 있어야 함을 보장한다. 같은 raw ID가 다른 issuer나 collection에 있어도 별도 사용자이며 이메일로 자동 연결하지 않는다. 명시적 master owner bootstrap은 선택 조직의 owner membership/binding을 원자 생성하며 application은 별도 등록한다. 인증 응답의 organizations relation은 사용하지 않는다. 정상 소속·권한의 권위는 Grant local row다.

### `auth_identity_cache`

외부 토큰 introspection 캐시. Rauthy bearer token 확인 결과와 `client_id`를 저장하며, `token_hash`가 PK이고 `expires_at` 기준으로 만료된다. PocketBase는 이 cache를 사용하지 않고 매 요청 auth-refresh한다. 과거처럼 client 정보가 없는 cache row는 application authz에 사용하지 않는다.

### `parties`

법인/사업자 식별자. `business_no`는 unique이며 `organization.id`와 같은 값으로 보지 않는다.

### `organizations`

테넌트와 권한 평가의 기본 단위. `business_no`는 외부 식별자 스냅샷이고, 내부 canonical key는 `id`다.

### `organization_party_links`

조직과 legal party 연결. 같은 조직의 같은 `relation_type`은 하나만 허용한다.

### `organization_memberships`

사용자와 조직의 소속 관계. `(organization_id, user_id)`는 unique다.

### `patch_data_authority_imports`, `patch_data_authority_import_entities`, `organization_hierarchy_edges`

Patch Data API rule 이동의 deterministic import receipt와 소유권 provenance, 조직 parent-child snapshot이다. `grant.patch-data-authority.v1` manifest는 user/external identity, organization, membership, hierarchy edge, plant binding, active/ready application assignment를 정렬된 전체 snapshot으로 전달한다. receipt는 provider별 exact `generation`, 원본 ledger lowercase SHA-256, versioned payload 전체의 length-framed canonical SHA-256, manifest version, 적용 시각을 저장한다. canonical digest는 `manifestDigest` 자신과 `expectedPreviousGeneration`만 제외하므로 CAS 경로와 무관하게 같은 authority payload는 같은 digest를 갖는다.

`patch_data_authority_import_entities`는 importer가 교체할 수 있는 canonical user, organization, membership, plant binding, application assignment의 `(entity_type, entity_id)`만 추적한다. importer는 absent row를 만들거나 같은 kind/id가 이미 import-owned인 경우만 갱신한다. 우연히 같은 native row는 채택하지 않으며 import replacement나 omission으로 삭제하지도 않는다. 다음 generation은 이전 import-owned membership, binding, `application_actions` assignment만 제거한 뒤 새 snapshot과 provenance를 하나의 Rhiza transaction으로 기록한다. omitted user/organization/identity는 남을 수 있지만 imported membership/binding이 없어서 권한을 갖지 않는다. application 자체는 importer가 생성·수정·삭제하지 않고 manifest의 `applicationAssignments`가 가리키는 existing active/ready same-org application만 검증한다.

각 import는 `patch-data` provider, `patch-data.plant.read` action, global `role-patch-data-plant-reader` role/action을 canonical catalog에 보장하고, membership을 `organization_memberships`, plant permission을 `role_bindings`, application capability를 `application_actions`에 직접 반영한다. 따라서 strong evaluator와 plant-read decision은 정상 요청에서 Patch Data relation이나 import staging table을 dual-read하지 않는다. hierarchy는 child당 parent 하나, sorted/unique/acyclic/referentially complete를 검증하지만 의미가 확정되지 않아 evaluator에는 연결하지 않는다. `application_consent_assertions`는 generation CAS, catalog/native-row collision, application readiness, partial-write를 transaction 안에서 fail closed시키는 CHECK 기반 임시 row이고 성공 시 비워진다.

### `groups`, `group_memberships`

조직 내부 권한 그룹과 membership 연결. `group_memberships.organization_membership_id`는 같은 조직의 active membership만 연결할 수 있다. 조회도 group과 membership의 조직이 같은 row만 반환한다. 그룹 삭제는 `subject_organization_id` 값과 관계없이 해당 group id를 참조하는 group-subject `role_bindings`가 하나라도 있으면 거부되고, 참조가 없을 때 membership과 그룹을 한 트랜잭션에서 삭제한다. 시작 시 missing reference 또는 cross-organization membership/group binding을 발견하면 자동 삭제하지 않고 schema validation을 실패시킨다. 권한 전개 입력이라 grant 경계에 남긴다.

### `service_providers`

grant가 권한 판정 대상으로 인정하는 서비스 제공자 catalog. 기본 provider는 `grant`이며, compos 같은 first-party 서비스도 이 테이블에 등록한 뒤 action manifest를 넣는다.

### `provider_http_capability_sets`, `provider_http_capability_active`

PEP가 provider-native HTTP 요청을 권한 tuple로 고정할 때 사용하는 불변 HTTP capability catalog다. `provider_http_capability_sets`의 `(provider_id, generation)`은 1부터 연속 증가하며 canonical JSON과 digest를 보존한다. 같은 provider/digest를 다른 generation으로 재사용할 수 없고, 같은 generation의 byte-identical 재시도만 idempotent하다. capability는 canonical JSON에 한 번만 저장하며 enabled 상태도 불변 manifest 일부다. 최종 transaction은 저장된 JSON의 배열 형태, 항목 수, 필수 값, version/enabled/access mode 및 capability/version 중복을 검증한다. entry를 바꾸거나 끄려면 새 generation을 stage해야 한다.

`provider_http_capability_active`는 유일한 publication pointer다. stage된 최대 generation을 자동으로 읽지 않으며, CAS activation/rollback이 가리킨 generation만 catalog fetch와 decision에 사용한다. `provider_http_capability_activations`는 pointer 이동을 append-only sequence와 전역 unique `activation_receipt_id`로 기록한다. activation 응답은 transaction 밖의 `MAX(sequence)`를 읽지 않고, 같은 transaction에서 삽입한 receipt ID로 자신의 sequence를 다시 읽으므로 뒤따른 activation과 혼동되지 않는다. rollback도 기존 generation row를 수정하지 않고 pointer만 이전 retained generation으로 이동한다. `application_consent_assertions`는 provider/action, generation CAS, active PEP 조건을 transaction 내부에서 fail closed시키는 임시 CHECK row이고 성공 시 비워진다.

Grant는 capability에 provider origin을 저장하지 않는다. catalog에는 exact method, closed path template, access mode, action/resource tuple, bounded extractor와 닫힌 query/request/response policy DTO만 저장한다. Phase A path는 resource extractor가 명시적으로 모델링한 변수 하나만 허용하며 collection 같은 추가 변수는 거부한다. 실제 origin과 transport는 PEP 배포 설정 소유다. 기존 authority snapshot import는 capability를 생성하거나 활성화하지 않는다.

### `provider_pep_clients`, `provider_pep_client_receipts`

Exposure 같은 PEP service identity와 provider를 연결한다. `pep_client_id`는 전역 primary key, `application_id`는 전역 unique라 둘 중 하나를 다른 provider로 재결합할 수 없다. binding은 active/disabled/deleted lifecycle과 선택적 RFC3339 expiry를 가지며, active/ready application만 fetch·decision에 사용할 수 있다. application auth key가 나타내는 application ID는 `GRANT_HTTP_CAPABILITY_PEP_APPLICATION_ID`에 명시하고 Grant가 그 application으로 binding을 역조회한다. 따라서 요청 body나 provider path가 PEP identity를 선택하지 못한다. 모든 lifecycle 변경은 `provider_pep_client_receipts`에 별도 receipt를 추가하며 capability history를 변경하지 않는다.

### `actions`, `roles`, `role_actions`, `role_bindings`

권한 엔진의 원본 데이터. `actions.provider_id`는 action을 제공하는 `service_providers.id`를 가리킨다. `actions.code`는 provider 안에서만 unique이며, 같은 code를 서로 다른 provider가 재사용할 수 있다. 실제 참조 key인 `actions.id`는 전역 PK라 grant provider는 기존처럼 code를 id로 쓰고, 외부 provider action은 provider/code 조합에서 정규화한 id를 사용한다.

coarse 배포에서는 모든 action의 `visibility_granting`이 false여야 한다. bootstrap은 `organization.profile.read`를 포함해 모두 0으로 seed하고, provider action 관리 및 DB write boundary는 `visibility_granting = true` 요청을 write 전에 거부한다. `grantable_to_applications`와 application consent 의미는 그대로 유지한다.

`role_bindings`는 `provider_id`, `resource_type`, `resource_id`를 함께 저장한다. grant 내부 조직 권한은 `provider_id = grant`, `resource_type = organization`, `resource_id = organizations.id`로 표현한다. 일반 외부 리소스는 해당 provider의 resource type/id를 그대로 저장하며, grant는 외부 리소스 존재 여부를 스키마에서 검증하지 않는다. 폐기된 Exposure 전용 mapping은 적용하지 않으며 범용 capability가 정의한 resource tuple을 사용한다.

`agreements`는 별도 테이블로 다시 살아났지만, `role_bindings.agreement_id`는 물리 FK 없이 앱 레벨로 검증한다.
grant 조직 간 권한은 active `agreements`가 있어야 cross-org binding을 만들 수 있다.

과거 `account` provider에 남아 있던 `organization.*` action과 `organization` resource binding은 schema migration에서 `grant` provider로 이동한다. 동일 logical binding이 이미 `grant`에 있으면 legacy `account` row의 active 상태와 agreement 참조를 `grant` row에 병합한 뒤 legacy row를 삭제한다.

### `role_action_field_views`

보존된 field-authorization 스키마다. `(role_id, action_id, resource_type, schema_key, field_pointer)`가 저장 key이고 `view_mode`는 `real` 또는 `anonymized`만 허용한다. role과 action 및 해당 `role_actions` row를 외래 키로 참조하고 `(role_id, action_id, resource_type, schema_key)` 조회 index를 유지한다. 현재 coarse 배포에는 이 테이블의 관리 API, evaluator, startup cleanup이 없으며 row가 하나라도 있으면 호환성 검사가 시작을 막는다. 따라서 이 설명은 보존된 row shape과 관계를 문서화할 뿐 현재 활성 정책 계약을 뜻하지 않는다.

### `field_view_policy_audit_outbox`

보존된 필드 열람 정책 교체 audit outbox 스키마다. `event_id`가 primary key이고 `idempotency_key`는 unique이며 요청 hash, 조직/행위자/role/action 범위, resource/schema key, before/after JSON snapshot과 생성 시각 컬럼을 유지한다. 현재 coarse 배포는 이 outbox에 쓰거나 delivery worker를 시작하지 않으며 row가 하나라도 있으면 시작을 실패한다.

### `applications`, `application_redirect_uris`, `application_actions`, `application_grants`, `application_grant_actions`

API/OAuth application와 사용자 consent grant. active grant unique 제약과 action subset 검증 때문에 grant 경계에 남긴다.

PocketBase 모드에서는 조직 앱 생성 시 `provisioning_status = ready`, `rauthy_client_id = NULL`을 저장한다. 별도 OIDC client 생성 절차가 없기 때문이다. 이 상태는 인증 연동 준비만 뜻하며, `first_party = 0`과 `consent_policy = explicit` 및 application action·사용자 권한 검증은 그대로 유지한다. 아래 Rauthy client 생성·검증 절차는 Rauthy 모드에 적용된다.

`applications.rauthy_client_id`는 호환성을 위해 보존한 기존 컬럼명이다. Rauthy 모드의 새 조직 앱 생성은 IdP client ID를 발급하지 않고 NULL과 unprovisioned 상태를 저장한다. 운영자가 IdP에 생성한 기존 client ID를 플랫폼 PUT identity-binding에 전달하면 설정된 Rauthy 읽기 전용 어댑터가 manifest를 검증한다. 성공 시 검증한 로컬 상태가 변하지 않았는지 트랜잭션에서 다시 확인하고 client ID와 ready를 원자 저장한다. 기존 ready 연결 및 unprovisioned 예약 데이터는 보존한다. 준비된 앱을 다른 client ID로 재연결할 수 없다. Grant는 외부 클라이언트 생성·수정·삭제나 secret 발급·회전을 수행하지 않는다.

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

### Provider 액션 lifecycle 정리

내부 등록 API의 lifecycle 의미는 다음과 같다. 물리적 테이블·컬럼·인덱스나 canonical key는 변경하지 않는다. 구현·검증 상태는 [등록 API 관리](registration-api-management.md)를 따른다.

- 활성 `provider_http_capability_sets`의 catalog가 참조하는 액션은 Provider 액션 전체 교체로 삭제할 수 없다. 해당 참조가 없는 catalog를 먼저 활성화해야 하며, 활성화 시 현재 액션 레지스트리와의 참조도 다시 검증한다.
- `actions.grantable_to_applications`를 false로 변경하면 해당 액션의 `application_actions`와 `application_grant_actions` 연결을 제거한다. `role_actions` 연결은 유지한다. 액션 자체를 삭제할 때의 기존 역할·앱·동의 연결 정리와는 구분한다.
- Application의 액션 선택은 활성 Provider의 앱 부여 가능 액션만 허용한다. 조직별 후보 조회도 같은 조건을 사용하며, 후보에 나타나는 사실은 사용자 RBAC나 리소스 binding을 대신하지 않는다.

#### 역할·앱 액션의 개별 연결

역할과 앱의 액션 연결은 전체 교체(`PUT .../actions`)와 개별 추가·삭제(`PUT/DELETE .../actions/{actionId}`)를 지원한다. 개별 조작은 다른 연결을 보존한다.

- `role_actions`: `(role_id, action_id)`는 복합 PK다. 개별 PUT은 `ON CONFLICT DO NOTHING`으로 연결을 추가하고, 개별 DELETE는 해당 row만 제거한다.
- `application_actions`: `(application_id, action_id)`는 복합 PK다. 개별 PUT은 멤버십을 추가하고, 개별 DELETE는 해당 row만 제거한다. `application_grant_actions`의 stale row도 같은 트랜잭션에서 정리한다.

단일 액션 변경 시 전체 목록을 다시 보내지 않으므로 다른 액션의 동시 변경을 덮어쓰지 않는다. 개별 PUT·DELETE는 멱등적이다. 같은 액션에 대한 상반된 변경은 트랜잭션 실행 순서를 따른다.

#### Provider 액션 개별 관리

Provider 액션은 전체 교체(`PUT`) 외에 개별 생성(`POST`), 수정(`PATCH`), 삭제(`DELETE`)를 지원한다.

- `POST /api/v1/admin/providers/{providerId}/actions`: 새 액션을 생성한다. ID는 `providerId.code`로 정규화한다.
- `PATCH /api/v1/admin/providers/{providerId}/actions/{actionId}`: 기존 액션의 이름, 설명, 위험 수준 등을 수정한다. 전체 교체와 달리 다른 액션에 영향을 주지 않는다.
- `DELETE /api/v1/admin/providers/{providerId}/actions/{actionId}`: 액션과 모든 참조를 삭제한다. 활성 catalog가 참조하는 액션은 삭제할 수 없다.

#### 동시성 제어 (Concurrency control)

액션 멤버십과 consent 교체는 `If-Match`/`ETag` 헤더로 동시성 제어를 지원한다.

- Provider·역할·앱 액션 목록과 현재 동의의 GET 응답은 본문과 같은 strong SQL snapshot에서 계산한 `ETag`를 반환한다. 액션 메타데이터 변경도 태그에 반영한다.
- 전체 교체 PUT은 해당 GET에서 얻은 `If-Match`가 필수다. 현재 동의가 없는 최초 생성에는 `If-None-Match: *`를 사용한다.
- 누락은 `428`, 불일치는 `412`다. 읽기 이후 쓰기 직전 상태도 동일 트랜잭션의 assertion으로 비교한다. 비교용 snapshot은 기존 `grant_import_payloads`에 불변 청크로 준비하며 권한 판정에는 사용하지 않는다.
- 개별 액션 추가·삭제와 메타데이터 PATCH에는 전체 목록 ETag를 요구하지 않는다. 같은 필드를 동시에 PATCH하면 나중 트랜잭션의 값이 남는다.

이 메커니즘은 오래된 전체 목록으로 다른 요청의 변경을 덮어쓰는 일을 막는다. 테이블·컬럼 추가는 없다.

이 구조의 이유는 catalog가 사라진 액션을 계속 참조하지 않도록 하고, 앱에 대한 부여 중단이 과거 동의로 되살아나지 않도록 하기 위해서다. 역할 자체의 권한 정의와 앱이 사용할 수 있는 권한을 구분하므로 앱 부여 가능 여부만 변경했을 때 역할 연결은 보존한다.

- Patch Data의 42개 컬렉션 읽기 카탈로그는 기존 `role_bindings.(provider_id, resource_type, resource_id)`와 `role_actions.action_id`를 사용한다. `provider_id`는 `patch-data`, resource type은 컬렉션 이름이며 `plants`만 기존 `plant`를 유지한다. action ID는 `patch-data.<resource_type>.read`다. 컬렉션 이름과 PocketBase ID 경로는 동일한 canonical tuple을 사용하고 resource ID는 응답 레코드의 `id`다.
- 뷰도 독립된 resource type을 사용한다. 원본과 뷰의 레코드 ID가 같거나 업무 relation이 연결되어 있어도 권한을 전파하지 않는다. 조직 및 발전소 권한 import가 새 컬렉션의 binding을 자동 생성하지 않는다. 스키마 DDL과 기존 canonical key의 의미는 변경하지 않는다.

### 컬렉션 읽기 권한 구조의 이유

PocketBase service token은 자체 사용자 규칙을 우회할 수 있으므로 업무 relation을 근거로 허용 범위를 확대하지 않는다. 목록은 각 canonical record binding을 기존 membership·application·consent 평가기로 검사해 허용 ID를 만들고, 상세 조회는 같은 action/resource tuple을 검사한다. `patch_data_collections.json`은 경로와 검색 가능한 scalar 필드만 선언하며 실제 권한이나 응답 필드 마스킹을 생성하지 않는다. 비밀번호 및 업무 계정 컬렉션을 포함한 읽기 action은 전체 레코드 접근을 뜻한다. 운영자는 해당 데이터에 접근 가능한 주체에게만 명시적으로 부여해야 한다. 등록 및 activation 절차는 [카탈로그 현황](catalog-coverage.md)에 설명한다.

### 컬렉션 변경 및 파일 권한 릴레이션

30개 base 컬렉션 생성은 `patch-data.<resource_type>.create`와 `(patch-data, collection, 컬렉션 이름)`을 함께 검사한다. 수정 및 삭제는 각각 `.update`, `.delete`와 기존 레코드 tuple을 검사한다. 파일이 있는 5개 컬렉션의 다운로드는 `.file.read`와 같은 레코드 tuple을 사용하며 일반 `.read`나 업로드 권한으로 대신할 수 없다. 이름/ID URL alias는 이 canonical key를 공유한다. `org_parent_map`은 원본 hook이 만드는 62자 hex ID를 수용하도록 `asciiToken` 추출기를 사용한다. 원본 키의 의미를 바꾸지는 않는다.

이 구조는 native API의 전체 작업 단위를 권한으로 표현한다. relation 변경, hook 및 cascade 부작용도 해당 쓰기 권한에 포함하며 관련 레코드별 권한을 추가로 검사한다고 주장하지 않는다. PocketBase 업무 데이터의 관계를 Grant 권한으로 자동 전개하지 않고 새 권한은 명시적 action/role/binding으로만 부여한다. 뷰에는 변경 권한을 발행하지 않는다.

큰 provider action manifest는 기존 `grant_import_payloads`에 `kind='provider-actions'`로 불변 문자열 조각을 먼저 저장한다. 한 최종 transaction에서 전체 action 교체, 생략된 action의 role/application/consent 연결 삭제, staging 정리를 함께 수행한다. 중간 chunk는 권한 판단에 사용하지 않으며 최종 실패 시 기존 권한을 유지한다. 실패한 요청의 참조되지 않는 staging 조각이 남을 수 있다. 새 테이블이나 DDL 변경은 없다.

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
- `provider_http_capability_sets.provider_id` -> `service_providers.id`
- `provider_http_capability_active.(provider_id, generation)` -> `provider_http_capability_sets.(provider_id, generation)`
- `provider_pep_clients.provider_id` -> `service_providers.id`
- `provider_pep_clients.application_id` -> `applications.id`
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

## Go/Rhiza storage port

현재 스키마는 Grant 테이블 33개(논리 snapshot 대상 31개, 내부 staging 1개,
restore receipt 1개)와 명시적 인덱스 16개를 생성한다. 사용하지 않던 import/capability
assertion 테이블 2개와 중복 capability projection, V2 authority snapshot 저장 기능을 제거했다.
실제 transaction 검증은 공통 `application_consent_assertions`를 계속 사용한다.
membership, party 사업자번호, group membership, activation sequence/receipt의 중복 인덱스
5개는 제거했지만 같은 키를 강제하는 PK/UNIQUE 제약은 유지한다.

이번 정리는 호환성을 깨는 canonical schema 교체다. migration과 snapshot 버전 번호를
올리거나 이전 DB/snapshot 변환기를 추가하지 않았다. 이전 Go DB는 migration checksum이
달라 재사용할 수 없으며, 이전 logical snapshot은 schema digest/table inventory가 달라
restore 검증에서 거부된다. 기존 데이터 경로에 덮어쓰거나 자동 삭제하지 않는다.
새 스키마로 만든 별도 데이터 경로와 snapshot을 사용해야 한다.

The Go runtime's canonical schema is [store_schema.sql](../internal/grant/store_schema.sql),
with indexes in [store_indexes.sql](../internal/grant/store_indexes.sql). Rhiza v0.12.3
applies the three named migrations through its replicated migration ledger. Existing
Hiqlite directories are never opened as Rhiza stores; use the explicit, separate-path
export/restore procedure in [backup-restore.md](backup-restore.md).

`grant_import_payloads(import_id, kind, ordinal, payload)` is internal immutable staging
for Patch Data authority imports. Its composite primary key identifies a staged input
chunk; it is not queried by any authorization evaluator. Batches stay below Rhiza's
128 KiB certified-command bound. Only the final replicated transaction checks the prior
generation, validates canonical ownership, replaces canonical rows and provenance,
updates the receipt, and removes that import's staged chunks. A failed final transaction
leaves all canonical authority unchanged. Interrupted staging can leave unreferenced
chunks; the import ID binds the full input digest and retrying requires byte-identical
chunk payloads. A different payload cannot overwrite an existing staged chunk.
This table is excluded from logical application backups because it is not authoritative.

Application-consent assertion rows are temporary transaction checks. The Go mutation
boundary removes successful assertion rows in the same transaction. Failed assertions
roll back the mutation, its receipt, and all assertion inserts.

Logical snapshots contain the complete canonical table/column inventory, per-table row
counts and SHA-256 digests. Restore writes only a new, isolated directory. A pending
restore marker blocks normal startup until all table digests match and a verified
cutover marker has been written. The original Hiqlite database and export are retained.

Fresh multi-voter logical restore uses an explicit, previously empty S3 prefix and a
new target cluster identity. `grant_restore_activation(id, snapshot_digest, cluster_id,
activated_at)` is an internal replicated admission receipt (migration 3), excluded
from application snapshots. Every configured voter must acknowledge the same prepared
source snapshot and member set before opening Rhiza. An immutable local admission
marker prevents reuse of an occupied data directory. Canonical tables are restored
with a single transaction, including an empty-target assertion and the activation
receipt; staged input never affects authorization reads. Startup returns only after
all restored table digests match. Interrupted staging or activation can retry the same
admission; it cannot select another snapshot or target identity. Existing source data
and object-store prefixes are preserved. See [backup-restore.md](backup-restore.md)
for the preparatory command and the distinction from offline single-voter restore.

PocketBase identity 분리로 `user_identities.issuer`, `collection_id`와 4-part unique key가 추가됐다. V1 authority import의 `users`는 `issuer`, `collectionId`를 필수로 전달하며 canonical user digest frame에도 포함한다. 내부 `userId` 및 membership/binding 참조는 바꾸지 않는다. 인증을 통해 생성한 identity와 import identity는 이 namespace가 정확히 같을 때만 같은 사용자를 가리킨다. 기존 `(patch-data, raw ID)`를 자동 재연결하지 않으며 기존 DB/backup은 schema checksum 변경으로 거부한다. 버전 alias나 자동 마이그레이션은 없고 실제 데이터 삭제는 수행하지 않았다.

### Catalog-driven record list scopes

Capability canonical JSON may include a bounded record-scope policy; no separate scope table or materialized permission copy is created. Candidate resource IDs come from active `role_bindings` for the exact provider, resource type and subject organization, and every candidate passes through the existing authorization SQL with the current user and application. The evaluator additionally exposes its existing application-scope/consent intermediate for empty-list admission without changing the allow rule. Catalog, membership and authorization queries use one fenced applied slot. Constant list targets are request bindings, not collection-wide authority; scope overflow is an error, and `*` has no wildcard meaning.

## 배치 권한의 canonical 관계

배치 계약은 `provider_http_capability_sets.canonical_json` 내부의 `batch` 정책으로 저장하며 새 테이블은 추가하지 않는다. 정책이 참조하는 child capability는 같은 generation의 활성 mutation 계약이어야 한다. Patch Data의 배치 루트는 `patch-data.batch.execute` action과 `(patch-data, batch, patch-data)` resource tuple을 사용하고, 이를 허용해도 child 레코드/컬렉션 권한을 대신하지 않는다. 원본 body digest와 ordered child digest를 함께 응답에 결합하고, 동일한 fenced snapshot에서 루트 및 모든 child의 membership/application/role/consent를 평가한다.

PUT upsert는 catalog의 명시적 create/update 계약 쌍을 참조한다. 요청 ID가 있으면 collection 생성 권한과 해당 record 수정 권한을 모두 요구하고, 없으면 생성 권한만 요구한다. 존재 여부를 미리 조회하여 권한을 선택하지 않는다. 실제 PocketBase transaction은 변경하지 않으며, 권한 실패 시 원본 배치 전체를 upstream에 보내지 않는다.

## 응답 관계 확장의 canonical 관계

`provider_http_capability_sets.canonical_json`에는 선택적 `responseResources` 그래프를 한 번만 저장한다. 그래프가 없으면 JSON 필드를 생략하여 기존 catalog digest를 유지한다. 각 노드의 action/resource type 및 record ID/collection identity 필드와 명시적 relation edge를 catalog digest에 포함한다. 개별 GET 계약은 `responseAuthorization.rootKey`로 그래프를 참조하며 새 DDL이나 업무 DB 테이블은 만들지 않는다.

Patch Data 그래프는 고정 schema snapshot의 42개 컬렉션과 공개 relation 필드 74개에서 추출했다. 뷰는 독립적인 노드/action/resource type을 유지한다. 관계 edge는 조회 경로만 선언하며 권한을 전개하지 않는다. 응답의 루트와 관련 레코드는 각각 기존 role/application 권한을 충족해야 한다. 원본 응답 hash, 길이, 순서가 있는 reference digest를 같은 요청에 결합하고 한 fenced snapshot에서 다시 판단한다. 목록은 collection 이름의 가상 grant를 요구하지 않고 기존 목록 admission 및 반환된 root ID를 재검사한다.

### 최신 business schema 통합

Patch Data `af6b560`의 최신 snapshot과 인증 분리 브랜치를 통합한 목록은 base 31개와 view 11개다. 이 중 `rsd_command_tasks`는 native API rule이 업무 상태 전이를 제한하므로 superuser 경로에서 쓰기를 허용하지 않는다. manifest의 `readOnly`가 CRUD 및 batch upsert 생성을 막으며 조회 권한만 등록한다. 나머지 base 30개의 전체 레코드 변경 권한은 기존 의미를 유지한다. MFA challenge/send-window 두 컬렉션은 Patch Auth 내부 상태로 분리하여 사업 리소스 graph에 포함하지 않는다.

### 플랫폼 관리 인증과 권한 관계

`X-Grant-Master-Key` 헤더가 없을 때 플랫폼 관리 라우트에서 bearer 토큰으로 대체 인증이 가능하다. `GRANT_MANAGEMENT_ORGANIZATION_ID` 환경 변수로 설정된 신뢰된 운영 조직만이 이 경로를 사용할 수 있다. 미설정 시 bearer 관리는 비활성화된다.

**인증 흐름:**

1. `X-Grant-Master-Key`가 유효하면 기존 master 인증이 적용된다. 잘못된 값은 bearer로 전환되지 않고 401을 반환한다.
2. 기존 master 인증 라우트에서 헤더가 없으면 `LookupManagementRoute(method, path)`로 등록된 플랫폼 라우트인지 확인한다. bootstrap처럼 등록되지 않은 master 라우트는 401을 반환한다. 일반 사용자 라우트의 인증 방식은 그대로 유지한다.
3. 플랫폼 라우트이면 bearer 토큰과 `X-Organization-ID` 헤더를 추출하여 `AuthorizePlatformManagement`를 호출한다.

**`AuthorizePlatformManagement(ctx, token, org, actionCode)` 검증:**

- `ManagementOrganizationID`가 설정되어 있어야 한다 (없으면 503).
- 요청 `org`가 설정된 관리 조직과 정확히 일치해야 한다 (불일치 시 403).
- `Authenticate`로 토큰의 외부 주체가 활성 사용자이고 관리 조직의 활성 구성원인지 확인한다.
- `CheckApplicationAuthorizations`로 토큰에 연결된 애플리케이션의 허용 액션과 사용자 역할을 확인한다. 신뢰된 앱은 명시적 동의를 생략하며, 그 외 앱에는 해당 액션에 대한 활성 사용자 동의가 필요하다.

**릴레이션:** 기존 `actions` → `role_actions` → `role_bindings` 관계로 세부 관리 권한을 등록한다. 플랫폼 API는 `(grant-management, platform, grant)`, 조직 API는 `(grant-management, organization, 조직 ID)`, 사용자 정보와 provider 목록은 `(grant-management, service, grant)` 리소스를 사용한다. `application_actions`와 신뢰 설정 또는 `application_grants`·`application_grant_actions`가 앱의 사용 범위를 추가로 제한한다. 새 테이블이나 컬럼은 없다.

**구조의 이유:** 리소스 유형으로 관리 범위를 구분하고, 설정으로 지정한 운영 조직인지도 별도로 검사한다. 다른 조직이 같은 플랫폼 리소스에 역할을 연결해도 이 조직 검사를 통과하지 못한다. 운영 조직의 역할 관리자는 플랫폼 권한을 부여할 수 있으므로 해당 조직은 운영자 전용으로 관리해야 한다.

**기존 관리 권한 이전:** `grant-management.manage` 정의를 보존하는 것은 이전 카탈로그의 운영·롤백을 위한 것이며 새 액션으로 권한을 전개하지 않는다. 이전 세 그룹 경로를 계속 허용하려면 활성화 전에 해당 역할의 `role_actions`, 앱의 `application_actions`, 비신뢰 앱 사용자의 `application_grant_actions`에 `grant-management.groups.read`, `.groups.create`, `.groups.delete`를 명시적으로 추가해야 한다. 기존 조직 리소스 바인딩은 재사용한다. 동의는 사용자 동의 API로 갱신하며, 이전 `manage` 권한을 전체 관리 권한으로 자동 확대하지 않는다. 이 데이터 이전이 완료되지 않으면 이전 카탈로그를 유지한다.

**가입 전 본인 식별:** `POST /api/v1/me/materialize`는 조직 가입 전에 로컬 사용자 ID를 확보하는 경로다. 이 경로의 정확한 capability에 한해 사용자 인증, 활성 사용자·앱·조직·provider, 앱의 `grant-management.user.materialize` 액션 등록을 검사하되 멤버십·역할·동의는 요구하지 않는다. 이 요청은 본인 identity만 등록하며 멤버십이나 역할, 앱 동의를 생성하지 않는다. 가입하지 않은 사용자는 `membershipStatus: none`을 받고, 다른 관리 경로는 계속 일반 권한 검사를 받는다. 이 예외에는 본문 없는 `forward-user` 요청만 해당한다. 새 테이블이나 컬럼은 없다.

**Actor attribution:** bearer 경로에서 `WithManagementActor(ctx, userID)`로 컨텍스트에 인증된 사용자 ID를 저장한다. 마스터 경로에서는 `"grant-master"`가 기본값으로 유지된다. `ActivateHttpCapabilityGenerationAdmin`과 `SetProviderPepClientAdmin`은 `ManagementActorFromContext(ctx)`를 사용하여 기록된 actor를 결정한다.

**비활성 조직 재활성화 PDP 경로:** `POST /api/v1/orgs/{organizationId}/reactivate`만 조직 활성 상태 검사를 예외 처리한다. `managementReactivationCapability`와 SQL은 정확한 `grant-management.organization.reactivate` 액션, 자기 조직 리소스, 일치하는 조직 경로를 요구한다. 앱 ID를 유지하므로 역할·앱 허용 액션·신뢰 또는 동의 검사는 계속 적용된다. 원본 API는 별도로 `organization.permission.manage` 권한을 확인한다. 일반 관리 요청은 비활성 조직에서 계속 거절하며, 예외 경로에는 `recordScope`, `batch`, `responseAuthorization`를 사용할 수 없다.

### 인증 identity 연결 원칙

모든 인증 어댑터는 정확한 `user_identities` 매핑만 재사용한다. 매핑이 없으면 새 내부 사용자 ID를 발급하고 identity만 생성한다. Rauthy에서도 이메일 일치나 외부 subject와 내부 사용자 ID의 문자열 일치로 기존 사용자를 자동 채택하지 않는다. 기존에 등록된 매핑과 membership/binding은 그대로 보존한다. 매핑 없이 이메일에 의존하던 레거시 계정은 운영자가 소유권을 확인하여 명시적으로 연결해야 한다. 이 원칙은 일반 로그인으로 초기 소유자 등록의 승인 경계를 우회하지 못하게 하기 위한 것이다. 테이블 구조와 canonical identity key는 바뀌지 않는다.

## 관리 수명주기의 릴레이션과 구조의 이유

이번 변경은 테이블·컬럼·인덱스나 canonical key를 추가하지 않으며, 기존 row의 수명주기와 연결 의미를 다음처럼 명시한다. HTTP 계약은 [API 수명주기](api-lifecycle.md)에 있다.

- 조직 생성은 `organizations`, owner `organization_memberships`, 해당 조직 자원에 대한 `role-system-bootstrap-owner`의 개인 `role_bindings`를 같은 트랜잭션으로 생성한다. 기존 활성 사용자를 명시하며 조직 생성이 IdP 계정 생성이나 호출자 자동 승격을 뜻하지 않는다.
- 구성원 제거는 `organization_memberships.status=inactive`로 이력을 남긴다. 해당 소속의 `group_memberships`, 해당 사용자와 소속 조직의 개인 `role_bindings`를 제거하고 조직 내 활성 `application_grants`를 revoked로 전환한다. owner 제거는 거절한다. 재등록은 기존 소속 row를 재활성화하지만 제거한 개인 권한·그룹·동의는 복원하지 않는다. 조직 전체 Binding은 제거 대상이 아니므로 재가입 후 적용될 수 있다.
- 앱 비활성화는 기존 동의를 철회한다. 재활성화는 ready 연결이나 동의를 새로 만들어 주지 않는다. 앱 삭제는 grant actions, grants, requested actions, redirect manifest와 application을 함께 지운다. `provider_pep_clients` 참조가 있으면 삭제를 거절하고, first-party 앱의 조직 단위 상태 변경·삭제도 거절한다. 외부 IdP 클라이언트는 변경하지 않는다.
- 역할 삭제는 어떤 상태의 Binding이든 참조가 있으면 409다. 참조가 없을 때 `role_action_field_views`, `role_actions`, `roles`를 함께 제거한다. 조직 API는 전역 시스템 역할을 수정·삭제하지 않는다.
- 조직 관리자의 동의 철회는 row를 삭제하지 않고 `status`, `revoked_at`, `revoked_by_user_id`를 기록한다. 과거 Action 연결이 남아도 활성 동의가 아니므로 권한 판정에 사용되지 않는다.
- identity binding은 기존 `rauthy_client_id`를 외부에서 할당한 검증 대상 client ID로 사용한다. 검증된 이름·종류·redirect manifest·상태와 조직 활성 여부·보안 변경 lease·전역 client ID 유일성을 트랜잭션으로 다시 검사한다. 이미 ready인 다른 연결로 교체하지 않는다. 기존 저장 데이터와 유일성 인덱스는 유지한다.

구조의 이유는 소속 재활성화가 과거 개인 위임을 자동 복구하지 않도록 하고, 사용자 권한과 앱 동의의 별도 철회 의미를 보존하기 위해서다. 참조된 실행기 앱이나 역할을 암묵적으로 지우지 않아서 운영 권한의 의도하지 않은 소실을 막는다. 검증과 저장 사이의 manifest 변경도 ready 상태로 승인하지 않으며, Grant의 권한 객체 삭제가 IdP 계정 관리로 번지지 않는다.

### Agreement 기간 비교와 초기 소유자 감사

Agreement의 새 생성·PUT·PATCH는 RFC3339 입력을 검증하고 UTC 소수점 이하 정확히 9자리의 고정 폭 문자열로 저장한다. 판정 시각도 같은 형식이며 시작·종료를 포함하는 비교를 수행한다. 별도 여유 시간은 없다. 기존 row는 일괄 이관하지 않으며 유효한 offset 표현을 포함한 비정규 기존 기간도 권한 판정에서는 거절한다. 권한 있는 grantor의 명시적 PATCH로 정규화한다. Binding 생성과 사용자·그룹·조직 주체의 모든 평가 경로가 같은 검사를 적용한다. null 경계는 무제한이며 다른 시각 필드의 저장 형식은 바꾸지 않는다.

초기 조직 owner Binding의 `created_by_user_id`는 `ManagementActorFromContext`에서 얻은 실제 운영 행위자를 기록한다. `subject_id`는 지정된 owner이며 두 필드는 서로 다른 의미다. 감사 컬럼이나 canonical key는 바꾸지 않는다.

구성원 제거 시 owner 외의 `membership_role`은 member로 내리고 status를 inactive로 남긴다. 앱 Action 집합의 조건부 변경 snapshot에는 `applications.id`, `organization_id`, `created_at`도 포함하여 삭제·재생성된 앱에 이전 ETag를 사용할 수 없도록 한다. 공개 응답 배열과 DB 컬럼 구성은 바꾸지 않는다.
