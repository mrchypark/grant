# Database Schema

이 문서는 현재 [`crates/infra/src/database.rs`](../crates/infra/src/database.rs) 기준의 실제 스키마를 설명한다.

핵심 원칙은 다음과 같다.

- `organization`이 테넌트 겸 권한 평가의 기본 단위다.
- `asset_id`가 물리 발전 설비의 내부 canonical 식별자다.
- `plant`는 한국 RPS 호환 계층의 명칭이며, `plants.id`는 내부 호환 row ID다. KR RPS 외부 설비 코드는 `facility_code`에만 둔다.
- `facility_code`는 한국 RPS 외부 설비 식별자이며 `asset_profiles`/`registry_projects`의 외부 ID로 다룬다.
- `plants.id`는 한국 RPS 호환 row ID이고, 신규 글로벌 모델의 canonical 식별자는 `asset_id`다.
- `facility_code`는 한국 RPS 외부 설비 식별자이며 `asset_profiles`/`registry_projects`의 외부 ID로 다룬다.
- 권한 원본과 조회 최적화 결과를 분리한다.
- 공동인증서, OAuth, API key, 로그인 credential은 `auth_credentials` 추상화로 수용한다.
- SQLite/hiqlite 위에서 앱 레벨 무결성과 권한 전개를 사용하므로, 현재 스키마는 물리 FK를 거의 두지 않는다.

## 테이블 설명

### `users` > 사용자 기본 프로필

- `id` > 사용자 식별자
- `email` > 현재 사용자 이메일 스냅샷
- `display_name` > 표시 이름
- `active` > 활성 사용자 여부
- `created_at` > 생성 시각

### `user_identities` > 외부 인증 공급자와 내부 사용자 매핑

- `id` > 매핑 식별자
- `user_id` > 내부 사용자 ID
- `provider` > 인증 공급자 이름
- `external_user_id` > 공급자 쪽 사용자 ID
- `email_snapshot` > 연동 시점 이메일 스냅샷
- `display_name_snapshot` > 연동 시점 표시 이름 스냅샷
- `last_seen_at` > 마지막 관찰 시각
- `created_at` > 생성 시각
- `updated_at` > 갱신 시각

### `auth_identity_cache` > 외부 토큰 introspection 캐시

- `token_hash` > 외부 토큰 해시, PK
- `provider` > 인증 공급자 이름
- `external_user_id` > 공급자 사용자 ID
- `email` > 공급자 응답 이메일
- `display_name` > 공급자 응답 표시 이름
- `active` > 공급자 기준 활성 여부
- `expires_at` > 캐시 만료 시각
- `updated_at` > 마지막 갱신 시각

### `organizations` > 사업자/총판/대리점/시공사 등 조직 기본 테이블

- `id` > 내부 조직 식별자
- `business_no` > 사업자번호
- `name` > 조직명
- `inactivated_at` > 조직이 비활성화된 시각, NULL이면 활성
- `created_at` > 생성 시각
- `updated_at` > 갱신 시각

### `organization_memberships` > 사용자와 조직의 소속 관계

- `id` > membership 식별자
- `organization_id` > 소속 조직 ID
- `user_id` > 사용자 ID
- `membership_role` > 조직 내 역할 (`owner`, `admin`, `member`)
- `status` > membership 상태
- `created_at` > 생성 시각
- `updated_at` > 갱신 시각

### `groups` > 조직 내부 팀

- `id` > 그룹 식별자
- `organization_id` > 그룹이 속한 조직 ID
- `name` > 그룹명
- `description` > 설명
- `created_at` > 생성 시각
- `updated_at` > 갱신 시각

### `group_memberships` > 그룹과 조직 멤버십 연결

- `group_id` > 그룹 ID
- `organization_membership_id` > 조직 membership ID

### `agreements` > 조직 간 권한 근거 계약

- `id` > 계약 식별자
- `agreement_type` > 계약 종류 (`delegation`, `epc`, `om`, `distributor`, `dealer`)
- `grantor_organization_id` > 권한을 부여하는 조직 ID
- `grantee_organization_id` > 권한을 받는 조직 ID
- `status` > 계약 상태
- `title` > 계약명
- `reference_no` > 계약 참조 번호
- `starts_at` > 시작 시각
- `ends_at` > 종료 시각
- `created_at` > 생성 시각
- `updated_at` > 갱신 시각

### `actions` > 세분화된 권한 액션 정의

- `id` > 액션 식별자
- `code` > 액션 코드
- `name` > 액션명
- `description` > 설명
- `risk_level` > 위험도
- `visibility_granting` > 이 액션이 plant visibility를 부여하는지 여부
- `created_at` > 생성 시각
- `updated_at` > 갱신 시각

### `roles` > 권한 묶음

- `id` > 롤 식별자
- `organization_id` > 조직 전용 롤이면 조직 ID, 공통 롤이면 NULL
- `code` > 롤 코드
- `name` > 롤명
- `description` > 설명
- `created_at` > 생성 시각
- `updated_at` > 갱신 시각

### `role_rules` > wildcard 기반 롤 규칙 정의

- `id` > 규칙 식별자
- `role_id` > 롤 ID
- `effect` > 효과, 현재는 allow
- `action_pattern` > wildcard 패턴
- `created_at` > 생성 시각
- `updated_at` > 갱신 시각

### `role_actions` > 롤에 실제로 속한 액션 전개 결과

- `role_id` > 롤 ID
- `action_id` > 액션 ID

### `role_bindings` > 주체에 롤을 부여한 원본 권한

- `id` > binding 식별자
- `subject_type` > 권한 주체 타입 (`organization`, `group`, `user`)
- `subject_id` > 주체 식별자
- `subject_organization_id` > 주체가 속한 organization context
- `resource_type` > 리소스 타입 (`organization`, `asset`, `plant`)
- `resource_id` > 리소스 식별자
- `role_id` > 부여된 롤 ID
- `agreement_id` > 계약 근거 ID
- `state` > binding 상태
- `created_by_user_id` > 생성한 사용자 ID
- `created_at` > 생성 시각
- `updated_at` > 갱신 시각

### `effective_organization_plant_actions` > 조직 기준 plant 액션 전개 결과

- `organization_id` > 조직 ID
- `plant_id` > plant ID
- `action_id` > 액션 ID
- `updated_at` > 갱신 시각

### `effective_user_plant_actions` > 사용자 기준 plant 액션 전개 결과

- `user_id` > 사용자 ID
- `organization_id` > active organization context
- `plant_id` > plant ID
- `action_id` > 액션 ID
- `updated_at` > 갱신 시각

### `effective_user_plant_visibility` > plant 목록 조회용 visibility 전개 결과

- `user_id` > 사용자 ID
- `organization_id` > active organization context
- `plant_id` > plant ID
- `updated_at` > 갱신 시각

### `effective_organization_asset_actions` > 조직 기준 asset 액션 전개 결과

- `organization_id` > 조직 ID
- `asset_id` > asset ID
- `action_id` > 액션 ID
- `updated_at` > 갱신 시각

### `effective_user_asset_actions` > 사용자 기준 asset 액션 전개 결과

- `user_id` > 사용자 ID
- `organization_id` > active organization context
- `asset_id` > asset ID
- `action_id` > 액션 ID
- `updated_at` > 갱신 시각

### `effective_user_asset_visibility` > asset 목록 조회용 visibility 전개 결과

- `user_id` > 사용자 ID
- `organization_id` > active organization context
- `asset_id` > asset ID
- `updated_at` > 갱신 시각

### `assets` > 글로벌 물리 발전 설비 마스터

- `id` > 내부 canonical asset ID
- `owner_organization_id` > 현재 소유 조직 ID
- `created_by_organization_id` > 최초 생성 조직 ID
- `name` > 설비명
- `country_code` > ISO 국가 코드
- `address` > 주소
- `capacity_kw` > 설비 용량
- `timezone` > 설비 기준 timezone
- `created_at` > 생성 시각
- `updated_at` > 갱신 시각

### `asset_profiles` > 국가/시장/제도별 asset 프로필

- `id` > profile 식별자
- `asset_id` > 연결된 asset ID
- `country_code` > 국가 코드
- `provider` > 프로필 외부 ID namespace를 소유한 provider/system 코드
- `profile_type` > 프로필 종류 (`kr_rps`, `irec`, `utility` 등)
- `external_id` > 해당 제도/시스템의 외부 식별자
- `metadata_json` > 프로필별 보조 메타데이터
- `created_at` > 생성 시각
- `updated_at` > 갱신 시각
- 유니크 제약 > `(country_code, provider, profile_type, external_id)`

### `registries` > 외부 registry/utility/market/carbon 시스템

- `id` > registry 식별자
- `registry_type` > registry 종류 (`rec`, `utility`, `market`, `carbon`)
- `country_code` > 국가 코드, 글로벌 registry면 NULL 가능
- `provider` > 공급자/시스템 코드
- `name` > 표시 이름
- `metadata_json` > registry별 보조 메타데이터
- `created_at` > 생성 시각
- `updated_at` > 갱신 시각
- 유니크 제약 > 국가 registry는 `(country_code, provider, registry_type)`, 글로벌 registry는 `(provider, registry_type)` partial unique index로 관리한다.

### `registry_accounts` > registry 계정

- `id` > registry account 식별자
- `registry_id` > registry ID
- `organization_id` > 계정을 소유하거나 사용하는 조직 ID
- `external_account_id` > registry 쪽 계정 ID
- `display_name` > 표시 이름
- `status` > 계정 상태
- `metadata_json` > 계정별 보조 메타데이터
- `created_at` > 생성 시각
- `updated_at` > 갱신 시각

### `registry_projects` > registry에 등록된 프로젝트/설비/계량기

- `id` > registry project 식별자
- `registry_id` > registry ID
- `asset_id` > 연결된 asset ID
- `registry_account_id` > registry account ID
- `external_project_id` > registry 쪽 project/device/meter ID
- `project_type` > 프로젝트 종류
- `status` > registry project 상태
- `metadata_json` > 프로젝트별 보조 메타데이터
- `created_at` > 생성 시각
- `updated_at` > 갱신 시각

### `data_sources` > 발전량/정산/registry 데이터 출처

- `id` > data source 식별자
- `asset_id` > 연결된 asset ID
- `registry_id` > registry ID
- `registry_project_id` > registry project ID
- `source_type` > 소스 종류 (`utility`, `registry`, `manual`, `api`)
- `provider` > 공급자/시스템 코드
- `external_source_id` > 외부 소스 ID
- `unit` > 기본 단위
- `metadata_json` > 소스별 보조 메타데이터
- `created_at` > 생성 시각
- `updated_at` > 갱신 시각

### `generation_data` > source-attributed 발전량 데이터

- `id` > generation datum 식별자
- `asset_id` > asset ID
- `source_id` > data source ID
- `period_start` > 측정 구간 시작
- `period_end` > 측정 구간 종료
- `metric_type` > `value`의 의미를 나타내는 adapter별 metric code (`gross_generation_kwh`, `net_export_kwh`, `meter_import_kwh`, `settled_generation_mwh` 등)
- `value` > `metric_type`이 지정한 측정값
- `unit` > 단위
- `quality` > 데이터 품질/상태
- `metadata_json` > 소스별 보조 메타데이터
- `recorded_at` > 기록 시각
- `updated_at` > 갱신 시각

### `rate_data` > source-attributed 가격/요율 데이터

- `id` > rate datum 식별자
- `asset_id` > asset ID
- `source_id` > data source ID
- `period_start` > 적용 구간 시작
- `period_end` > 적용 구간 종료
- `metric_type` > 요율/가격 metric 종류 (`self_rate`, `national_average`, `nearby_average`, `nearby_highest`, `ppa_price`, `fit_rate`, `utility_tariff` 등)
- `value` > metric 값
- `unit` > 단위 (`KRW/kWh`, `USD/MWh` 등)
- `currency` > 통화 코드
- `quality` > 데이터 품질/상태 (`published`, `unpublished` 등)
- `metadata_json` > 소스별 보조 메타데이터
- `recorded_at` > 기록 시각
- `updated_at` > 갱신 시각

### `energy_certificates` > 범용 에너지 인증서/REC

- `id` > 인증서 식별자
- `certificate_type` > 인증서 종류 (`REC`, `IREC`, `LGC`, `STC` 등)
- `registry_id` > 발급 registry ID
- `registry_project_id` > registry project ID
- `asset_id` > 연결된 asset ID
- `serial_number` > registry 기준 serial number
- `vintage_start` > vintage 시작일
- `vintage_end` > vintage 종료일
- `quantity_mwh` > 인증 수량
- `owner_account_id` > 현재 소유 registry account ID
- `status` > 인증서 상태
- `issued_at` > 발급 시각
- `retired_at` > retirement/소각 시각
- `metadata_json` > 인증서별 보조 메타데이터
- `updated_at` > 갱신 시각

### `auth_credentials` > 외부 시스템 접속 credential 암호문

- `id` > credential 식별자
- `organization_id` > credential 소유 조직 ID
- `credential_type` > credential 종류 (`certificate`, `oauth`, `api_key`, `login`)
- `country_code` > 국가 코드
- `provider` > 공급자/시스템 코드
- `external_subject_id` > 외부 subject/account 식별자
- `encrypted_blob` > credential payload 암호문
- `wrapped_dek` > payload 암호화에 쓴 DEK를 master key로 감싼 값
- `kms_key_id` > wrap에 사용한 master key 버전
- `active` > 현재 사용 가능 여부
- `last_used_at` > 마지막 사용 시각
- `created_at` > 생성 시각
- `updated_at` > 갱신 시각

### `credential_bindings` > credential과 registry account 연결

- `credential_id` > credential ID
- `registry_account_id` > registry account ID
- `created_at` > 생성 시각

### `country_capabilities` > 국가별 기능 capability

- `country_code` > 국가 코드, PK
- `has_rps` > RPS 제도 존재 여부
- `has_rec_registry` > REC registry 존재 여부
- `has_utility_api` > utility API 존재 여부
- `has_certificate_login` > 인증서 로그인 존재 여부
- `enabled_features_json` > 활성 기능 목록 JSON
- `updated_at` > 갱신 시각

### `certificates` > 저장형 공동인증서 암호문 보관

- `id` > 저장 인증서 식별자
- `organization_id` > 인증서를 소유한 조직 ID
- `common_name` > 인증서 주체 표시명
- `encrypted_blob` > 인증서 bundle 암호문
- `wrapped_dek` > bundle 암호화에 쓴 DEK를 master key로 감싼 값
- `kms_key_id` > wrap에 사용한 master key 버전
- `active` > 현재 사용 가능 여부
- `last_used_at` > 마지막 사용 시각
- `created_at` > 저장 시각

### `cert_gateway_challenges` > 짧은 수명의 인증서 증명 challenge

- `id` > challenge 식별자
- `organization_id` > 요청 조직 ID
- `mode` > `stored` 또는 `ephemeral`
- `nonce` > 서명 대상 nonce
- `payload_hash` > `cert-gateway:` payload 원문 SHA-256 해시. payload는 canonical JSON을 URL-safe base64로 감싼 값이며 `organizationId`, `mode`, `nonce`, `expiresAt`를 기본으로 포함하고, 요청에 있으면 `origin`, `facilityCode`, `startMonth`, `endMonth`, `signCertSha256Hex`까지 함께 묶는다. `businessNo`는 요청값 또는 조직에 연결된 법인 사업자번호가 있으면 정규화해 함께 묶는다.
- `facility_code` > challenge가 특정 발전소 실행 스코프에 묶인 경우의 발전소 코드
- `expires_at` > 만료 시각
- `consumed_at` > 1회 사용 완료 시각
- `created_by_user_id` > 요청 사용자 ID
- `created_at` > 생성 시각

### `crawl_task_tokens` > 저장형 인증서용 1회성 작업 토큰

- `id` > 토큰 식별자
- `organization_id` > 요청 조직 ID
- `certificate_id` > 저장 인증서 ID
- `token_hash` > 작업 토큰 해시
- `scope_json` > 작업 스코프 JSON. `facility_code`는 요청 body 원문이 아니라 service authorization이 확정한 facility scope이며, 생략 요청이 단일 실행 가능 plant로 해석되면 그 facility code가 저장된다. bounded full sync처럼 실행 불가능한 shape은 token 발급 및 소비 전 검증에서 거부된다.
- `expires_at` > 만료 시각
- `used_at` > 사용 완료 시각
- `created_by_user_id` > 요청 사용자 ID
- `created_at` > 생성 시각

### `plants` > 한국 RPS 호환 발전소 마스터

- `id` > 내부 plant row ID. 새 생성/수집 경로에서는 `facility_code` 원문을 ID로 쓰지 않고, 동일 facility의 동시 최초 sync가 같은 row로 수렴하도록 provider-scoped opaque ID를 쓴다. KR RPS adapter는 `plant-` + SHA-256 32 hex prefix를 사용한다.
- `facility_code` > 외부 RPS 설비 코드
- `owner_organization_id` > 현재 소유 조직 ID
- `created_by_organization_id` > 최초 생성 조직 ID
- `name` > 발전소명
- `contract_no` > 계약 번호
- `address` > 주소
- `installed_kw` > 설비 용량
- `rec_start_date` > REC 시작일
- `confirm_issued` > 확인 발급 상태 문자열
- `created_at` > 생성 시각
- `updated_at` > 갱신 시각

### `sync_runs` > 발전소 수집 실행 이력

- `id` > 실행 식별자
- `organization_id` > 실행 요청 조직 ID
- `plant_id` > 실행 대상 내부 plant row ID
- `mode` > `full` 또는 `delta`
- `status` > 실행 상태
- `started_at` > 시작 시각
- `finished_at` > 종료 시각
- `duration_ms` > 소요 시간
- `error_message` > 실패 이유

### `sync_run_items` > 수집 실행의 facility/source 단위 write 이력

- `id` > item 식별자
- `sync_run_id` > 상위 sync run ID
- `asset_id` > canonical asset ID
- `registry_project_id` > registry project ID
- `data_source_id` > data source ID
- `facility_code` > KR RPS 외부 설비 코드
- `status` > item 상태
- `generation_count` > projection한 발전량 row 수
- `rate_count` > projection한 rate row 수
- `rec_count` > projection한 REC row 수
- `document_count` > projection한 document row 수
- `error_message` > item 실패 이유
- `write_version` > write/projection 버전 시각
- `created_at` > 생성 시각
- `updated_at` > 갱신 시각

### `generation_records` > 월별 발전량 데이터

- `plant_id` > 내부 plant row ID. KR RPS 외부 설비 코드는 `facility_code`에만 저장한다.
- `month` > 기준 월
- `generation_kwh` > 총 발전량
- `self_consumption_kwh` > 자가소비량
- `supply_kwh` > 공급량
- `application_deadline` > 신청 마감일
- `updated_at` > 갱신 시각

### `rate_records` > 월별 요율/단가 데이터

- `plant_id` > 내부 plant row ID. KR RPS 외부 설비 코드는 `facility_code`에만 저장한다.
- `month` > 기준 월
- `self_rate` > 자사 요율
- `national_average` > 전국 평균
- `nearby_average` > 인근 평균
- `nearby_highest` > 인근 최고값
- `published` > 공시 여부
- `source_control_flag` > 소스 제어 플래그
- `updated_at` > 갱신 시각

### `rec_records` > REC 신청/발급 이력

- `plant_id` > 내부 plant row ID. KR RPS 외부 설비 코드는 `facility_code`에만 저장한다.
- `application_no` > 신청 번호
- `month` > 기준 월
- `supply_kwh` > 공급량
- `rec_quantity` > REC 수량
- `fee_krw` > 수수료
- `issued_date` > 발급일
- `expiry_date` > 만료일
- `virtual_account` > 가상계좌
- `bank_name` > 은행명
- `status` > 상태 문자열
- `updated_at` > 갱신 시각

### `documents` > 발전소 관련 문서 메타데이터

- `id` > 문서 식별자
- `plant_id` > 내부 plant row ID. KR RPS 외부 설비 코드는 `facility_code`에만 저장한다.
- `category` > 문서 카테고리
- `attachment_code` > 외부 첨부 코드
- `file_class` > 원본 분류
- `file_sec_name` > 세부 분류명
- `file_name` > 파일명
- `required` > 필수 제출 문서 여부
- `source_sha256` > 원본 해시
- `output_sha256` > 변환/저장 결과 해시
- `object_key` > blob/object storage 키. 수집 저장 경로는 외부 `facility_code`가 아니라 내부 `plants.id`를 key segment로 사용한다.
- `content_type` > MIME type
- `updated_at` > 갱신 시각

## 릴레이션

현재 스키마는 FK 제약 대신 앱 레벨로 관계를 유지한다. 주요 릴레이션은 아래와 같다.

- `user_identities.user_id` -> `users.id`
- `organization_memberships.user_id` -> `users.id`
- `organization_memberships.organization_id` -> `organizations.id`
- `groups.organization_id` -> `organizations.id`
- `group_memberships.group_id` -> `groups.id`
- `group_memberships.organization_membership_id` -> `organization_memberships.id`
- `agreements.grantor_organization_id` -> `organizations.id`
- `agreements.grantee_organization_id` -> `organizations.id`
- `roles.organization_id` -> `organizations.id`
- `role_rules.role_id` -> `roles.id`
- `role_actions.role_id` -> `roles.id`
- `role_actions.action_id` -> `actions.id`
- `role_bindings.role_id` -> `roles.id`
- `role_bindings.agreement_id` -> `agreements.id`
- `role_bindings.subject_organization_id` -> `organizations.id`
- `role_bindings.resource_id` -> `organizations.id`, `plants.id`, `assets.id` 중 하나이며 `resource_type`에 따라 달라짐
- `effective_organization_plant_actions.organization_id` -> `organizations.id`
- `effective_organization_plant_actions.plant_id` -> `plants.id`
- `effective_organization_plant_actions.action_id` -> `actions.id`
- `effective_user_plant_actions.user_id` -> `users.id`
- `effective_user_plant_actions.organization_id` -> `organizations.id`
- `effective_user_plant_actions.plant_id` -> `plants.id`
- `effective_user_plant_actions.action_id` -> `actions.id`
- `effective_user_plant_visibility.user_id` -> `users.id`
- `effective_user_plant_visibility.organization_id` -> `organizations.id`
- `effective_user_plant_visibility.plant_id` -> `plants.id`
- `effective_organization_asset_actions.organization_id` -> `organizations.id`
- `effective_organization_asset_actions.asset_id` -> `assets.id`
- `effective_organization_asset_actions.action_id` -> `actions.id`
- `effective_user_asset_actions.user_id` -> `users.id`
- `effective_user_asset_actions.organization_id` -> `organizations.id`
- `effective_user_asset_actions.asset_id` -> `assets.id`
- `effective_user_asset_actions.action_id` -> `actions.id`
- `effective_user_asset_visibility.user_id` -> `users.id`
- `effective_user_asset_visibility.organization_id` -> `organizations.id`
- `effective_user_asset_visibility.asset_id` -> `assets.id`
- `assets.owner_organization_id` -> `organizations.id`
- `assets.created_by_organization_id` -> `organizations.id`
- `asset_profiles.asset_id` -> `assets.id`
- `registry_accounts.registry_id` -> `registries.id`
- `registry_accounts.organization_id` -> `organizations.id`
- `registry_projects.registry_id` -> `registries.id`
- `registry_projects.asset_id` -> `assets.id`
- `registry_projects.registry_account_id` -> `registry_accounts.id`
- `data_sources.asset_id` -> `assets.id`
- `data_sources.registry_id` -> `registries.id`
- `data_sources.registry_project_id` -> `registry_projects.id`
- `generation_data.asset_id` -> `assets.id`
- `generation_data.source_id` -> `data_sources.id`
- `rate_data.asset_id` -> `assets.id`
- `rate_data.source_id` -> `data_sources.id`
- `energy_certificates.registry_id` -> `registries.id`
- `energy_certificates.registry_project_id` -> `registry_projects.id`
- `energy_certificates.asset_id` -> `assets.id`
- `energy_certificates.owner_account_id` -> `registry_accounts.id`
- `auth_credentials.organization_id` -> `organizations.id`
- `credential_bindings.credential_id` -> `auth_credentials.id`
- `credential_bindings.registry_account_id` -> `registry_accounts.id`
- `certificates.party_id` -> `parties.id`
- `cert_gateway_challenges.organization_id` -> `organizations.id`
- `cert_gateway_challenges.created_by_user_id` -> `users.id`
- `crawl_task_tokens.organization_id` -> `organizations.id`
- `crawl_task_tokens.certificate_id` -> `certificates.id`
- `crawl_task_tokens.created_by_user_id` -> `users.id`
- `plants.owner_organization_id` -> `organizations.id`
- `plants.created_by_organization_id` -> `organizations.id`
- `sync_runs.organization_id` -> `organizations.id`
- `sync_runs.plant_id` -> `plants.id`
- `sync_run_items.sync_run_id` -> `sync_runs.id`
- `sync_run_items.asset_id` -> `assets.id`
- `sync_run_items.registry_project_id` -> `registry_projects.id`
- `sync_run_items.data_source_id` -> `data_sources.id`
- `generation_records.plant_id` -> `plants.id`
- `rate_records.plant_id` -> `plants.id`
- `rec_records.plant_id` -> `plants.id`
- `documents.plant_id` -> `plants.id`

## 구조의 이유

### 1. `organization` 중심 모델을 둔 이유

- 사업자, 총판, 대리점, 시공사, 유지보수사를 하나의 추상화로 표현할 수 있다.
- 로그인/권한/데이터 소유권을 같은 축에서 평가할 수 있다.
- `business_no`는 외부 식별자이고, `organization.id`는 내부 canonical key라서 둘을 분리할 수 있다.

### 2. `asset_id`와 외부 식별자를 분리한 이유

- `asset_id`는 내부 비즈니스 로직과 권한 모델이 의존하는 canonical facility ID다.
- `facility_code`, I-REC project ID, utility meter ID 같은 값은 외부 시스템 식별자다.
- 외부 설비 코드 변경, 중복 이슈, 소스별 키 차이를 내부 식별자에 전파하지 않기 위해 분리한다.
- 새 `asset_id`는 KR RPS `facility_code` 원문을 ID로 사용하지 않는다. KR RPS 경로는 기존 `asset_profiles`/`registry_projects` 매핑을 먼저 해석하고, 없으면 동일 facility의 동시 최초 sync가 같은 asset으로 수렴하도록 provider-scoped opaque ID를 발급한다. 호환 `plants.id`도 같은 이유로 짧은 외부 ID가 아니라 32 hex prefix 이상의 deterministic opaque key를 쓴다.
- 국가/시장별 외부 ID는 `asset_profiles.external_id` 또는 `registry_projects.external_project_id`에 저장한다.
- 기존 `plants`/`plant_id`는 한국 RPS 호환 경로로 남아 있지만 `facility_code`와 같은 값으로 취급하지 않는다. 글로벌 권한과 신규 데이터 모델의 기준은 `asset_id`다.
- 같은 `plants.id`가 다른 조직에서 들어오면 기존 plant 소유권을 덮어쓰지 않고 거부한다.
- `sync_runs`, 레코드, 문서는 호환 `plant_id`에 연결하고, 글로벌 권한과 신규 데이터 모델은 `asset_id`를 기준으로 전개한다.

### 3. 권한 원본과 전개 테이블을 분리한 이유

- `role_bindings`는 감사 가능한 원본 권한만 저장한다.
- `effective_*` 테이블은 hot path 조회 최적화용이다.
- 조직별 명시 binding, 그룹 membership, 계약 상태까지 매번 런타임 join으로 계산하면 목록/권한 체크가 느려진다.
- `list_visible_plants()` 같은 API를 빠르게 만들기 위해 전개 캐시가 필요하다.

### 4. `agreements`를 별도 테이블로 둔 이유

- 외부 조직 권한의 근거를 binding row 안에 묻어두지 않기 위해서다.
- 계약 종료, 만료, 해지 같은 상태 변화를 권한과 분리해 관리할 수 있다.
- 계약 상태가 바뀌면 권한 전개만 다시 계산하면 된다.
- 글로벌 모델에서는 계약이 `asset` 접근 권한의 근거가 되며, registry account나 credential 사용은 추가 실행 조건으로 검증한다.

### 5. registry/profile/source 계층을 분리한 이유

- 하나의 물리 발전 설비가 여러 registry, utility, PPA, O&M 시스템에 다른 ID로 등록될 수 있다.
- `assets`는 물리 설비의 공통 속성만 가진다.
- `asset_profiles`는 국가/시장별 식별자와 제도별 보조 정보를 담는다.
- `registry_projects`는 registry 내부 project/device/meter와 `asset`의 매핑이다.
- 서비스 출시 전 브레이킹 스키마 변경이므로 `ensure_schema()`는 기존 `plants` 행을 `assets`, `asset_profiles`, `registry_projects`, `data_sources`로 backfill하지 않는다.
- 한국 RPS 생성/수집 경로가 처음부터 `plants` 호환 행과 canonical `assets`/profile/registry/source/certificate 데이터를 함께 써야 한다. 오래된 dev DB는 재생성하거나 명시적으로 reseed한다.
- 실제 외부 계정 ID가 없으면 `registry_accounts`는 만들지 않고 `registry_projects.registry_account_id`를 NULL로 둔다.
- 기존 `registry_projects.registry_account_id`가 있으면 sync 실행 조직의 `business_no`로 owner registry account를 덮어쓰지 않는다. 새 registry account를 만들 때만 `organization_id + external_account_id` 조합으로 식별한다.
- `data_sources`와 `generation_data`는 발전량 데이터가 하나의 truth에서만 온다는 가정을 제거한다.
- `generation_data.metric_type`은 canonical `value`의 의미를 국가/adapter가 명시하는 필드다. 예를 들어 한국 RPS adapter는 `generation_records.generation_kwh`/원천 `GELEC_QTY`를 `gross_generation_kwh`로 저장한다. 순수 발전량, 계통 송전량, 정산 확정량, 수입 계량값이 같은 `value` 컬럼에 섞이지 않도록 adapter는 metric code를 고정해야 한다.

### 6. `energy_certificates`를 별도로 둔 이유

- REC, I-REC, LGC, STC 등은 registry별 serial number와 소유/상태 추적이 핵심이다.
- 한국 `rec_records`는 한국 RPS 신청/발급 화면에 가까운 호환 데이터이고, 글로벌 인증서의 canonical 모델은 `energy_certificates`다.
- 한국 RPS 호환 projection은 `application_no`가 plant별로만 유니크할 수 있으므로 `serial_number`를 `facility_code:application_no` 형태의 KR RPS namespace 값으로 저장한다.
- 인증서 거래, 이전, retirement 같은 상태는 registry account와 연결해서 추적해야 한다.

### 7. `auth_credentials`, `certificates`, `cert_gateway_challenges`, `crawl_task_tokens`를 분리한 이유

- `auth_credentials`는 공동인증서, OAuth, API key, portal login을 모두 수용하는 신규 credential 추상화다.
- 한국 공동인증서를 저장하면 한국 RPS `certificate` credential도 `auth_credentials`에 함께 기록하고, registry account가 확인되는 시점에 `credential_bindings`로 연결한다.
- registry project가 특정 account에 묶인 뒤에는 저장형 KR RPS credential 사용도 해당 account의 `credential_bindings`를 통과해야 한다.
- 공동인증서 subject에 사업자번호가 없으면 이미 연결된 legal party나 명시적으로 검증된 조직 business number만 사용할 수 있다. 요청 payload의 `business_no`만으로 새 legal party를 bootstrap하지 않는다.
- self-signed/test 인증서는 기본 운영 설정에서 `auth_credentials`로 저장하지 않는다. 기본 설정은 proof bundle의 intermediate DER chain과 `CERTD_CERT_TRUST_BUNDLE_PATH`로 지정된 PEM CA bundle 또는 시스템 X.509 trust store 기준으로 인증서 체인을 검증하며, local/test fixture만 `allow_untrusted_certificates` 설정을 명시적으로 켠다.
- 저장형 인증서, 일회성 증명 challenge, 저장형 작업 토큰은 수명과 보안 성격이 다르다.
- `certificates`는 한국 공동인증서 호환 경로의 장기 보관 대상이고 암호화 저장이 필요하다.
- `cert_gateway_challenges`는 아주 짧은 수명과 1회 사용이 핵심이다. payload hash는 사용자가 승인한 origin/scope/certificate fingerprint가 서명 대상과 분리되지 않도록 묶는다.
- `crawl_task_tokens`는 저장형 인증서로 작업을 위임할 때만 쓰는 짧은 수명 capability다.
- 이 셋을 분리해야 만료, 재사용 방지, 감사 추적을 명확히 할 수 있다.

### 8. 인증서 blob에 비밀번호 대신 `wrapped_dek`를 쓰는 이유

- 사용자 원래 인증서 비밀번호를 서버에 저장하지 않기 위해서다.
- 저장형도 per-certificate 랜덤 DEK로 암호화하고, 그 DEK만 KMS-style master key로 감싼다.
- 나중에 실제 cloud KMS로 바꿔도 상위 모델이 유지된다.

### 9. 수집 결과를 `assets -> registry/source -> records/documents`로 나누는 이유

- `assets`는 현재 상태의 물리 설비 마스터 정보다.
- `asset_profiles`/`registry_projects`는 외부 시스템별 등록 상태다.
- `sync_runs`는 수집 실행 이력이다.
- `generation_data`, `rate_data`, `energy_certificates`는 글로벌 canonical 데이터다.
- `generation_data`는 `asset_id + source_id + metric_type + period` 기준으로 유일하다. 국가별 adapter가 여러 발전량 metric을 제공하면 같은 source/period에 여러 row로 저장한다.
- `generation_records`, `rate_records`, `rec_records`, `documents`는 한국 RPS 호환 조회용 materialized state다. 이 테이블의 `plant_id`는 모두 내부 `plants.id`를 가리키며 `facility_code`를 대체 key로 쓰지 않는다.
- full sync는 한국 RPS 호환 테이블뿐 아니라 해당 `data_source`/`registry_project` 범위의 `generation_data`, `rate_data`, `energy_certificates`도 replace한다.
- full sync replace는 새 projection을 먼저 upsert하고 그 뒤 stale row를 prune한다. stale row 삭제는 transaction 안의 조건부 delete로 수행해 overlapping full sync에서도 완료 snapshot 기준으로 수렴한다.
- full sync는 전체 facility/source snapshot일 때만 prune할 수 있으므로 `startMonth`/`endMonth`가 있는 bounded full sync는 service layer에서 거부한다.
- sync 성공 materialization은 `plants`, canonical asset projection, 호환 projection, document rows, `sync_run_items`, `sync_runs` 완료 상태를 하나의 DB transaction으로 커밋한다. 문서 object upload는 transaction 전에 준비하고 stale object 삭제는 DB commit 이후에만 수행한다.
- `sync_run_items`는 facility/source 단위 write 범위와 row count를 남겨 canonical projection과 호환 projection의 추적 지점을 만든다.
- collector 결과 검증이나 materialization 중 실패가 나면 대상 facility의 item도 `failed`로 남긴다.
- 실행 이력과 현재 상태를 분리해야 재수집, delta/full, 실패 복구, 문서 교체를 다룰 수 있다.

### 10. `documents`를 blob 본문이 아니라 메타데이터 테이블로 둔 이유

- 실제 파일은 object storage나 artifact 디렉터리에 두고 DB에는 참조만 두는 편이 단순하다.
- 해시와 object key를 저장하면 중복 감지, 무결성 확인, 재다운로드 판단이 가능하다.
- 문서 조회와 파일 전달 책임을 분리할 수 있다.

### 11. 물리 FK를 적극적으로 두지 않은 이유

- 현재 저장소는 hiqlite/SQLite 기반 embedded 운영과 앱 레벨 무결성 검증을 전제로 한다.
- 권한 전개, 일시적 재계산, seed/복구를 앱에서 제어하는 편이 더 단순하다.
- 이번 REPass v2 스키마는 출시 전 브레이킹 변경으로 처리하므로, 기존 plant-only 데이터를 자동 migration/backfill하지 않는다.
- 대신 스키마 테스트와 repository 메서드가 관계 일관성을 보장하는 방향으로 설계되어 있다.

### 12. `sync_runs.plant_id`를 nullable로 둔 이유

- 현재는 단일 plant 실행이 많지만, 배치형 전체 수집으로 확장할 여지를 남기기 위해서다.
- 글로벌 모델에서는 sync 실행 단위가 특정 한국 RPS plant가 아니라 asset, registry, data source, 또는 전체 계정이 될 수 있다.
- facility를 지정하지 않고 시작한 sync도 collector가 facility를 확정한 뒤에는 `plant_id`를 내부 plant row ID로 갱신한다. 실패 전 write target이 확정되지 않은 경우에는 `sync_run_items.registry_project_id`와 `data_source_id`를 NULL로 둘 수 있다.

## 현재 문서 기준으로 특히 주의할 점

- `asset_id`가 글로벌 내부 canonical facility key다.
- `plant_id`는 한국 RPS 호환 row의 내부 key다. 외부 설비 코드는 `facility_code`다.
- `facility_code`는 외부 식별자이자 profile/registry mapping 필드다.
- `organization_id`와 `business_no`는 서로 다른 값이다.
- 조직 간 asset/plant 접근은 상하위 상속이 아니라 `agreements + role_bindings`의 명시 조합으로만 생긴다.
- `sync_run_items`는 현재 facility 단위 write 이력을 남기며, account-wide/batch sync가 늘어나면 item별 실패와 재시도 단위가 된다.
