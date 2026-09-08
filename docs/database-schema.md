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

외부 인증 공급자와 내부 사용자 매핑. unique key는 `(provider, issuer, collection_id, external_user_id)`다. PocketBase는 `provider=pocketbase`, 설정된 canonical origin인 `issuer`, auth-refresh로 검증한 `collection_id`와 raw record ID를 모두 사용한다. Rauthy는 기존 subject 의미를 유지하고 issuer/collection은 빈 문자열이다. CHECK 제약은 PocketBase의 두 namespace 값이 모두 있어야 하고 다른 provider에서는 모두 비어 있어야 함을 보장한다. 같은 raw ID가 다른 issuer나 collection에 있어도 별도 사용자이며 이메일로 자동 연결하지 않는다. 명시적 master external-owner bootstrap만 선택 조직의 owner membership/binding과 고정 application을 원자 생성한다. 인증 응답의 organizations relation은 사용하지 않는다. 정상 소속·권한의 권위는 Grant local row다.

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

- Patch Data의 35개 컬렉션 읽기 카탈로그는 기존 `role_bindings.(provider_id, resource_type, resource_id)`와 `role_actions.action_id`를 사용한다. `provider_id`는 `patch-data`, resource type은 컬렉션 이름이며 `plants`만 기존 `plant`를 유지한다. action ID는 `patch-data.<resource_type>.read`다. 컬렉션 이름과 PocketBase ID 경로는 동일한 canonical tuple을 사용하고 resource ID는 응답 레코드의 `id`다.
- 뷰도 독립된 resource type을 사용한다. 원본과 뷰의 레코드 ID가 같거나 업무 relation이 연결되어 있어도 권한을 전파하지 않는다. 조직 및 발전소 권한 import가 새 컬렉션의 binding을 자동 생성하지 않는다. 스키마 DDL과 기존 canonical key의 의미는 변경하지 않는다.

### 컬렉션 읽기 권한 구조의 이유

PocketBase service token은 자체 사용자 규칙을 우회할 수 있으므로 업무 relation을 근거로 허용 범위를 확대하지 않는다. 목록은 각 canonical record binding을 기존 membership·application·consent 평가기로 검사해 허용 ID를 만들고, 상세 조회는 같은 action/resource tuple을 검사한다. `patch_data_collections.json`은 경로와 검색 가능한 scalar 필드만 선언하며 실제 권한이나 응답 필드 마스킹을 생성하지 않는다. 비밀번호 및 업무 계정 컬렉션을 포함한 읽기 action은 전체 레코드 접근을 뜻한다. 운영자는 해당 데이터에 접근 가능한 주체에게만 명시적으로 부여해야 한다. 등록 및 activation 절차는 [카탈로그 현황](catalog-coverage.md)에 설명한다.

### 컬렉션 변경 및 파일 권한 릴레이션

27개 base 컬렉션 생성은 `patch-data.<resource_type>.create`와 `(patch-data, collection, 컬렉션 이름)`을 함께 검사한다. 수정 및 삭제는 각각 `.update`, `.delete`와 기존 레코드 tuple을 검사한다. 파일이 있는 5개 컬렉션의 다운로드는 `.file.read`와 같은 레코드 tuple을 사용하며 일반 `.read`나 업로드 권한으로 대신할 수 없다. 이름/ID URL alias는 이 canonical key를 공유한다. `org_parent_map`은 원본 hook이 만드는 62자 hex ID를 수용하도록 `asciiToken` 추출기를 사용한다. 원본 키의 의미를 바꾸지는 않는다.

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
