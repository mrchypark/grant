# Database Schema

이 문서는 현재 [`crates/infra/src/database.rs`](../crates/infra/src/database.rs) 기준의 실제 스키마를 설명한다.

핵심 원칙은 다음과 같다.

- `organization`이 테넌트 겸 권한 평가의 기본 단위다.
- `plant_id`가 내부 canonical 식별자다.
- `facility_code`는 외부 RPS 설비 식별자다.
- 권한 원본과 조회 최적화 결과를 분리한다.
- 공동인증서 작업 경계는 일반 로그인과 분리한다.
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
- `resource_type` > 리소스 타입 (`organization`, `plant`)
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
- `payload_hash` > challenge payload 해시
- `facility_code` > 현재는 선택 입력, 향후 메타/스코프용
- `expires_at` > 만료 시각
- `consumed_at` > 1회 사용 완료 시각
- `created_by_user_id` > 요청 사용자 ID
- `created_at` > 생성 시각

### `crawl_task_tokens` > 저장형 인증서용 1회성 작업 토큰

- `id` > 토큰 식별자
- `organization_id` > 요청 조직 ID
- `certificate_id` > 저장 인증서 ID
- `token_hash` > 작업 토큰 해시
- `scope_json` > 작업 스코프 JSON
- `expires_at` > 만료 시각
- `used_at` > 사용 완료 시각
- `created_by_user_id` > 요청 사용자 ID
- `created_at` > 생성 시각

### `plants` > 내부 발전소 마스터

- `id` > 내부 canonical plant ID
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
- `plant_id` > 실행 대상 plant ID
- `mode` > `full` 또는 `delta`
- `status` > 실행 상태
- `started_at` > 시작 시각
- `finished_at` > 종료 시각
- `duration_ms` > 소요 시간
- `error_message` > 실패 이유

### `generation_records` > 월별 발전량 데이터

- `plant_id` > plant ID
- `month` > 기준 월
- `generation_kwh` > 총 발전량
- `self_consumption_kwh` > 자가소비량
- `supply_kwh` > 공급량
- `application_deadline` > 신청 마감일
- `updated_at` > 갱신 시각

### `rate_records` > 월별 요율/단가 데이터

- `plant_id` > plant ID
- `month` > 기준 월
- `self_rate` > 자사 요율
- `national_average` > 전국 평균
- `nearby_average` > 인근 평균
- `nearby_highest` > 인근 최고값
- `published` > 공시 여부
- `source_control_flag` > 소스 제어 플래그
- `updated_at` > 갱신 시각

### `rec_records` > REC 신청/발급 이력

- `plant_id` > plant ID
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
- `plant_id` > plant ID
- `category` > 문서 카테고리
- `attachment_code` > 외부 첨부 코드
- `file_class` > 원본 분류
- `file_sec_name` > 세부 분류명
- `file_name` > 파일명
- `required` > 필수 제출 문서 여부
- `source_sha256` > 원본 해시
- `output_sha256` > 변환/저장 결과 해시
- `object_key` > blob/object storage 키
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
- `role_bindings.resource_id` -> `organizations.id` 또는 `plants.id`, `resource_type`에 따라 달라짐
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
- `certificates.organization_id` -> `organizations.id`
- `cert_gateway_challenges.organization_id` -> `organizations.id`
- `cert_gateway_challenges.created_by_user_id` -> `users.id`
- `crawl_task_tokens.organization_id` -> `organizations.id`
- `crawl_task_tokens.certificate_id` -> `certificates.id`
- `crawl_task_tokens.created_by_user_id` -> `users.id`
- `plants.owner_organization_id` -> `organizations.id`
- `plants.created_by_organization_id` -> `organizations.id`
- `sync_runs.organization_id` -> `organizations.id`
- `sync_runs.plant_id` -> `plants.id`
- `generation_records.plant_id` -> `plants.id`
- `rate_records.plant_id` -> `plants.id`
- `rec_records.plant_id` -> `plants.id`
- `documents.plant_id` -> `plants.id`

## 구조의 이유

### 1. `organization` 중심 모델을 둔 이유

- 사업자, 총판, 대리점, 시공사, 유지보수사를 하나의 추상화로 표현할 수 있다.
- 로그인/권한/데이터 소유권을 같은 축에서 평가할 수 있다.
- `business_no`는 외부 식별자이고, `organization.id`는 내부 canonical key라서 둘을 분리할 수 있다.

### 2. `plant_id`와 `facility_code`를 분리한 이유

- `plant_id`는 내부 비즈니스 로직과 권한 모델이 의존하는 canonical ID다.
- `facility_code`는 RPS 외부 시스템과 연동할 때 쓰는 외부 키다.
- 외부 설비 코드 변경, 중복 이슈, 소스별 키 차이를 내부 식별자에 전파하지 않기 위해 분리한다.
- 따라서 `sync_runs`, 레코드, 문서는 계속 `plant_id`에 연결하는 것이 맞다.

### 3. 권한 원본과 전개 테이블을 분리한 이유

- `role_bindings`는 감사 가능한 원본 권한만 저장한다.
- `effective_*` 테이블은 hot path 조회 최적화용이다.
- 조직별 명시 binding, 그룹 membership, 계약 상태까지 매번 런타임 join으로 계산하면 목록/권한 체크가 느려진다.
- `list_visible_plants()` 같은 API를 빠르게 만들기 위해 전개 캐시가 필요하다.

### 4. `agreements`를 별도 테이블로 둔 이유

- 외부 조직 권한의 근거를 binding row 안에 묻어두지 않기 위해서다.
- 계약 종료, 만료, 해지 같은 상태 변화를 권한과 분리해 관리할 수 있다.
- 계약 상태가 바뀌면 권한 전개만 다시 계산하면 된다.

### 5. `certificates`, `cert_gateway_challenges`, `crawl_task_tokens`를 분리한 이유

- 저장형 인증서, 일회성 증명 challenge, 저장형 작업 토큰은 수명과 보안 성격이 다르다.
- `certificates`는 장기 보관 대상이고 암호화 저장이 필요하다.
- `cert_gateway_challenges`는 아주 짧은 수명과 1회 사용이 핵심이다.
- `crawl_task_tokens`는 저장형 인증서로 작업을 위임할 때만 쓰는 짧은 수명 capability다.
- 이 셋을 분리해야 만료, 재사용 방지, 감사 추적을 명확히 할 수 있다.

### 6. 인증서 blob에 비밀번호 대신 `wrapped_dek`를 쓰는 이유

- 사용자 원래 인증서 비밀번호를 서버에 저장하지 않기 위해서다.
- 저장형도 per-certificate 랜덤 DEK로 암호화하고, 그 DEK만 KMS-style master key로 감싼다.
- 나중에 실제 cloud KMS로 바꿔도 상위 모델이 유지된다.

### 7. 수집 결과를 `plants -> sync_runs -> records/documents`로 나눈 이유

- `plants`는 현재 상태의 마스터 정보다.
- `sync_runs`는 수집 실행 이력이다.
- `generation_records`, `rate_records`, `rec_records`, `documents`는 현재 조회용 materialized state다.
- 실행 이력과 현재 상태를 분리해야 재수집, delta/full, 실패 복구, 문서 교체를 다룰 수 있다.

### 8. `documents`를 blob 본문이 아니라 메타데이터 테이블로 둔 이유

- 실제 파일은 object storage나 artifact 디렉터리에 두고 DB에는 참조만 두는 편이 단순하다.
- 해시와 object key를 저장하면 중복 감지, 무결성 확인, 재다운로드 판단이 가능하다.
- 문서 조회와 파일 전달 책임을 분리할 수 있다.

### 9. 물리 FK를 적극적으로 두지 않은 이유

- 현재 저장소는 hiqlite/SQLite 기반 embedded 운영과 앱 주도 migration을 전제로 한다.
- 권한 전개, 일시적 재계산, seed/복구/호환 migration을 유연하게 처리하려면 앱 레벨 제어가 더 단순하다.
- 대신 스키마 테스트와 repository 메서드가 관계 일관성을 보장하는 방향으로 설계되어 있다.

### 10. `sync_runs.plant_id`를 nullable로 둔 이유

- 현재는 단일 plant 실행이 많지만, 배치형 전체 수집으로 확장할 여지를 남기기 위해서다.
- 레거시 dev DB에 `facility_code`만 있던 과거 스키마와도 호환 migration을 적용할 수 있다.

## 현재 문서 기준으로 특히 주의할 점

- `plant_id`가 내부 canonical key다.
- `facility_code`는 외부 식별자이자 메타/매핑 필드다.
- `organization_id`와 `business_no`는 서로 다른 값이다.
- 조직 간 plant 접근은 상하위 상속이 아니라 `agreements + role_bindings`의 명시 조합으로만 생긴다.
- 현재 스키마에는 `sync_run_items` 같은 batch 상세 테이블은 아직 없다.
