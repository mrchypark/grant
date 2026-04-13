# Authorization Model Implementation Plan

> Superseded on 2026-04-10. Current implementation does not use `organization_edges` or `organization_closure`; organization-to-organization plant access is now modeled only through explicit `agreements + role_bindings`.

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** `cert`의 기존 `business` 중심 권한 모델을 `organization + role_bindings + effective_*` 구조로 교체하고, plant 목록/단건 권한 체크를 전개 테이블 기반으로 전환한다.

**Architecture:** 원본 규칙은 `actions`, `roles`, `role_actions`, `role_bindings`, `agreements`에 저장하고, 조회는 `effective_organization_plant_actions`, `effective_user_plant_actions`, `effective_user_plant_visibility`만 사용한다. `organization.*` 권한은 런타임 직접 평가로 처리하고, `plant.*` 권한만 plant 전개 테이블로 내린다.

**Tech Stack:** Rust, hiqlite, SQLite, axum, serde, tokio

---

### Task 1: 도메인 모델을 `organization` 중심으로 재정의

**Files:**
- Modify: `/Users/cypark/Documents/work/cert/crates/domain/src/lib.rs`
- Test: `/Users/cypark/Documents/work/cert/crates/domain/tests/model_roundtrip.rs`
- Test: `/Users/cypark/Documents/work/cert/crates/domain/tests/model_smoke.rs`

**Step 1: Write the failing test**

- `Business`, `BusinessMembership`, `BusinessRole` 중심 roundtrip 테스트를 `Organization`, `OrganizationMembership`, `Group`, `Agreement`, `RoleBinding` 중심으로 교체한다.
- `Plant`가 `owner_organization_id`와 `created_by_organization_id`를 가지는 직렬화 테스트를 추가한다.
- `Action`이 `visibility_granting`을 포함하는지 테스트를 추가한다.

**Step 2: Run test to verify it fails**

Run: `cargo test -p domain -- --nocapture`

Expected: 기존 도메인 구조체 이름과 필드가 달라 컴파일 또는 assertion 실패

**Step 3: Write minimal implementation**

- `Business*` 타입을 제거하거나 deprecated 처리한다.
- 아래 타입을 추가한다.
  - `Organization`
  - `OrganizationMembership`
  - `Group`
  - `Agreement`
  - `Action`
  - `Role`
  - `RoleRule`
  - `RoleBinding`
- `Plant`를 새 owner/creator 모델에 맞게 재정의한다.

**Step 4: Run test to verify it passes**

Run: `cargo test -p domain -- --nocapture`

Expected: PASS

**Step 5: Commit**

```bash
git add crates/domain/src/lib.rs crates/domain/tests/model_roundtrip.rs crates/domain/tests/model_smoke.rs
git commit -m "refactor: redefine domain authorization model"
```

### Task 2: 스키마를 `organization + agreement + role_bindings + effective_*`로 교체

**Files:**
- Modify: `/Users/cypark/Documents/work/cert/crates/infra/src/database.rs`
- Test: `/Users/cypark/Documents/work/cert/crates/infra/tests/database_schema.rs`

**Step 1: Write the failing test**

- 새 스키마 생성 후 다음 테이블이 존재하는지 검증하는 테스트를 추가한다.
  - `organizations`
  - `organization_memberships`
  - `groups`
  - `group_memberships`
  - `agreements`
  - `actions`
  - `roles`
  - `role_rules`
  - `role_actions`
  - `role_bindings`
  - `effective_organization_plant_actions`
  - `effective_user_plant_actions`
  - `effective_user_plant_visibility`
- 기존 `businesses`, `business_memberships` 기반 스키마에 더 이상 의존하지 않는지 확인하는 테스트를 넣는다.

**Step 2: Run test to verify it fails**

Run: `cargo test -p infra database_schema -- --nocapture`

Expected: 새 테이블 부재 또는 기존 스키마 assertion 실패

**Step 3: Write minimal implementation**

- `create schema` SQL 블록을 새 모델로 교체한다.
- 핵심 인덱스를 같이 추가한다.
  - `effective_organization_plant_actions (organization_id, plant_id, action_id)`
  - `effective_user_plant_actions (user_id, organization_id, plant_id, action_id)`
  - `effective_user_plant_visibility (user_id, organization_id, plant_id)`
- `plants` 테이블을 `owner_organization_id`, `created_by_organization_id` 기준으로 재정의한다.

**Step 4: Run test to verify it passes**

Run: `cargo test -p infra database_schema -- --nocapture`

Expected: PASS

**Step 5: Commit**

```bash
git add crates/infra/src/database.rs crates/infra/tests/database_schema.rs
git commit -m "feat: add authorization schema and effective tables"
```

### Task 3: bootstrap 데이터와 기본 action catalog를 시드

**Files:**
- Modify: `/Users/cypark/Documents/work/cert/crates/infra/src/database.rs`
- Test: `/Users/cypark/Documents/work/cert/crates/infra/tests/database_seed.rs`

**Step 1: Write the failing test**

- bootstrap 후 기본 사용자와 기본 조직이 생성되는지 검증한다.
- `actions`에 최소 action catalog가 시드되는지 검증한다.
- `plant.profile.read`가 `visibility_granting=true`로 들어가는지 검증한다.

**Step 2: Run test to verify it fails**

Run: `cargo test -p infra database_seed -- --nocapture`

Expected: seed 데이터 없음 또는 action catalog 미존재

**Step 3: Write minimal implementation**

- `ensure_bootstrap_user()`를 `ensure_bootstrap_principals()` 또는 동등한 초기화 함수로 확장한다.
- 기본 조직/기본 admin membership/action catalog seed를 추가한다.
- 초기 action catalog는 설계 문서 기준으로 넣는다.

**Step 4: Run test to verify it passes**

Run: `cargo test -p infra database_seed -- --nocapture`

Expected: PASS

**Step 5: Commit**

```bash
git add crates/infra/src/database.rs crates/infra/tests/database_seed.rs
git commit -m "feat: seed bootstrap organization and action catalog"
```

### Task 4: 원본 binding 조회/저장 API를 infra 계층에 추가

**Files:**
- Modify: `/Users/cypark/Documents/work/cert/crates/infra/src/database.rs`
- Modify: `/Users/cypark/Documents/work/cert/crates/infra/src/service.rs`
- Test: `/Users/cypark/Documents/work/cert/crates/infra/tests/authorization_bindings.rs`

**Step 1: Write the failing test**

- `organization` subject binding을 plant resource에 저장/조회하는 테스트를 추가한다.
- `group`/`user` subject binding을 같은 테이블에서 저장/조회하는 테스트를 추가한다.
- `organization` resource binding으로 `organization.plant.create` role을 저장/조회하는 테스트를 추가한다.

**Step 2: Run test to verify it fails**

Run: `cargo test -p infra authorization_bindings -- --nocapture`

Expected: binding 저장 API 부재

**Step 3: Write minimal implementation**

- `save_role_binding`, `list_role_bindings_for_subject`, `list_role_bindings_for_resource` 같은 최소 API를 추가한다.
- `agreement_id`, `state`, `subject_organization_id`, `resource_type` 제약을 앱 레벨에서 검증한다.
- `organization.*`는 plant 전개 대상이 아니라 런타임 평가 대상임을 반영한 조회 API를 추가한다.

**Step 4: Run test to verify it passes**

Run: `cargo test -p infra authorization_bindings -- --nocapture`

Expected: PASS

**Step 5: Commit**

```bash
git add crates/infra/src/database.rs crates/infra/src/service.rs crates/infra/tests/authorization_bindings.rs
git commit -m "feat: persist authorization role bindings"
```

### Task 5: 조직 명시 권한 전개기 구현

**Files:**
- Modify: `/Users/cypark/Documents/work/cert/crates/infra/src/database.rs`
- Modify: `/Users/cypark/Documents/work/cert/crates/infra/src/service.rs`
- Test: `/Users/cypark/Documents/work/cert/crates/infra/tests/authorization_expansion.rs`

**Step 1: Write the failing test**

- 각 조직은 자기 자신의 계약과 binding으로만 plant 권한을 얻는 테스트를 추가한다.
- 계약이 종료되면 해당 조직의 plant action만 사라지고, 다른 조직의 명시 계약은 유지되는지 검증한다.
- 실제 사용자 action은 소속 조직의 명시 plant action 상한 안에서만 전개된다는 테스트를 추가한다.

**Step 2: Run test to verify it fails**

Run: `cargo test -p infra authorization_expansion -- --nocapture`

Expected: explicit agreement/effective organization 전개 미구현

**Step 3: Write minimal implementation**

- `role_bindings(subject_type=organization, resource_type=plant)`를 `role_actions`와 join해서 `effective_organization_plant_actions`를 채운다.
- 조직 상속은 두지 않고, 명시된 organization binding만 전개한다.

**Step 4: Run test to verify it passes**

Run: `cargo test -p infra authorization_expansion -- --nocapture`

Expected: PASS

**Step 5: Commit**

```bash
git add crates/infra/src/database.rs crates/infra/src/service.rs crates/infra/tests/authorization_expansion.rs
git commit -m "feat: expand organization plant actions"
```

### Task 6: 사용자/그룹 권한 전개와 visibility 전개 구현

**Files:**
- Modify: `/Users/cypark/Documents/work/cert/crates/infra/src/database.rs`
- Modify: `/Users/cypark/Documents/work/cert/crates/infra/src/service.rs`
- Test: `/Users/cypark/Documents/work/cert/crates/infra/tests/authorization_visibility.rs`

**Step 1: Write the failing test**

- 사용자가 그룹 role만으로 plant action을 얻는 테스트를 추가한다.
- 사용자 직접 role override가 동작하는 테스트를 추가한다.
- `plant.profile.read`가 있을 때만 `effective_user_plant_visibility`에 row가 생기고, `plant.document.read`만 있을 때는 목록에 노출되지 않는 테스트를 추가한다.

**Step 2: Run test to verify it fails**

Run: `cargo test -p infra authorization_visibility -- --nocapture`

Expected: effective user action/visibility 전개 미구현

**Step 3: Write minimal implementation**

- `effective_user_plant_actions` 전개기 추가
- `group_memberships`, `organization_memberships`, `role_bindings(subject_type=group|user)`를 합쳐 사용자 권한을 계산
- `effective_user_plant_visibility`는 `plant.profile.read` 기준으로만 생성
- 카디널리티를 줄이기 위해 1차 구현은 `plant.profile.read`와 고빈도/고위험 action만 전개 대상으로 제한한다

**Step 4: Run test to verify it passes**

Run: `cargo test -p infra authorization_visibility -- --nocapture`

Expected: PASS

**Step 5: Commit**

```bash
git add crates/infra/src/database.rs crates/infra/src/service.rs crates/infra/tests/authorization_visibility.rs
git commit -m "feat: expand effective user plant visibility"
```

### Task 7: plant 목록과 단건 조회를 새 권한 경로로 전환

**Files:**
- Modify: `/Users/cypark/Documents/work/cert/crates/api/src/lib.rs`
- Modify: `/Users/cypark/Documents/work/cert/crates/infra/src/service.rs`
- Modify: `/Users/cypark/Documents/work/cert/crates/infra/src/database.rs`
- Test: `/Users/cypark/Documents/work/cert/crates/api/tests/plant_authorization.rs`

**Step 1: Write the failing test**

- 현재 조직 컨텍스트에서 허용된 plant만 목록에 나오는 API 테스트를 추가한다.
- 허용되지 않은 plant의 상세 조회가 거부되는 테스트를 추가한다.
- `organization.plant.create` 같은 organization scope 권한은 런타임 직접 평가 경로로 처리되는 테스트를 추가한다.

**Step 2: Run test to verify it fails**

Run: `cargo test -p api plant_authorization -- --nocapture`

Expected: 기존 business_no 기반 목록/조회 경로가 새 권한 모델과 불일치

**Step 3: Write minimal implementation**

- `list_plants()`를 business_no 직접 조회 대신 `effective_user_plant_visibility` 기반으로 교체한다.
- `get_plant()`는 `effective_user_plant_actions`에서 `plant.profile.read`를 확인하도록 바꾼다.
- API 입력에 `active organization` 전달 방식(헤더/컨텍스트/테스트 더블)을 추가한다.
- 기존 `/api/v1/businesses/{businessNo}/plants` 경로는 브레이킹 체인지 허용 범위 안에서 조직 중심 API로 바꾸거나 호환 레이어를 최소화한다.

**Step 4: Run test to verify it passes**

Run: `cargo test -p api plant_authorization -- --nocapture`

Expected: PASS

**Step 5: Commit**

```bash
git add crates/api/src/lib.rs crates/infra/src/service.rs crates/infra/src/database.rs crates/api/tests/plant_authorization.rs
git commit -m "feat: authorize plant list and detail via effective tables"
```

### Task 8: 조직 스코프 권한 경로와 생성 흐름 전환

**Files:**
- Modify: `/Users/cypark/Documents/work/cert/crates/api/src/lib.rs`
- Modify: `/Users/cypark/Documents/work/cert/crates/infra/src/service.rs`
- Modify: `/Users/cypark/Documents/work/cert/crates/infra/src/database.rs`
- Test: `/Users/cypark/Documents/work/cert/crates/api/tests/organization_authorization.rs`

**Step 1: Write the failing test**

- `organization.plant.create`가 있는 사용자만 plant 생성 가능한 테스트를 추가한다.
- `organization.plant.create_for_owner`가 없는 경우 타 조직 owner 지정이 거부되는 테스트를 추가한다.
- 생성 후 `plants.owner_organization_id`와 `created_by_organization_id`가 올바르게 저장되는 테스트를 추가한다.

**Step 2: Run test to verify it fails**

Run: `cargo test -p api organization_authorization -- --nocapture`

Expected: organization scope 권한 경로 미구현

**Step 3: Write minimal implementation**

- `organization` resource binding을 직접 조회하는 런타임 authorization 함수 추가
- plant 생성 서비스에서 owner/create-for-owner 정책 적용
- 생성 직후 최소 role binding 또는 전개 재계산 트리거 연결

**Step 4: Run test to verify it passes**

Run: `cargo test -p api organization_authorization -- --nocapture`

Expected: PASS

**Step 5: Commit**

```bash
git add crates/api/src/lib.rs crates/infra/src/service.rs crates/infra/src/database.rs crates/api/tests/organization_authorization.rs
git commit -m "feat: enforce organization-scoped plant creation permissions"
```

### Task 9: 브리지/인증서/수집 경로를 organization 모델에 맞게 이식

**Files:**
- Modify: `/Users/cypark/Documents/work/cert/crates/infra/src/service.rs`
- Modify: `/Users/cypark/Documents/work/cert/crates/infra/src/database.rs`
- Modify: `/Users/cypark/Documents/work/cert/crates/infra/src/bridge.rs`
- Modify: `/Users/cypark/Documents/work/cert/crates/api/src/lib.rs`
- Test: `/Users/cypark/Documents/work/cert/crates/infra/tests/collector_authorization.rs`

**Step 1: Write the failing test**

- 인증서 저장이 business_no가 아니라 owner organization 기준으로 연결되는 테스트를 추가한다.
- sync 실행 시 active organization이 해당 plant에 `plant.sync.execute` 권한이 있어야 하는 테스트를 추가한다.

**Step 2: Run test to verify it fails**

Run: `cargo test -p infra collector_authorization -- --nocapture`

Expected: 기존 business_no 중심 collector 경로와 새 organization 모델 불일치

**Step 3: Write minimal implementation**

- certificate 저장/로드 키를 organization 중심으로 전환
- bridge 호출 직전 plant owner organization과 certificate 연결 경로 정리
- sync 실행 권한을 `effective_user_plant_actions` 기반으로 연결

**Step 4: Run test to verify it passes**

Run: `cargo test -p infra collector_authorization -- --nocapture`

Expected: PASS

**Step 5: Commit**

```bash
git add crates/infra/src/service.rs crates/infra/src/database.rs crates/infra/src/bridge.rs crates/api/src/lib.rs crates/infra/tests/collector_authorization.rs
git commit -m "refactor: move collector authorization to organization model"
```

### Task 10: 문서와 운영 가이드 갱신

**Files:**
- Modify: `/Users/cypark/Documents/work/cert/README.md`
- Modify: `/Users/cypark/Documents/work/cert/docs/plans/2026-04-03-authorization-model-design.md`

**Step 1: Update docs**

- organization/group/plant/agreement/action/role/binding/effective table 관계를 README에 요약한다.
- `plant.profile.read`만 visibility를 부여한다는 정책을 명시한다.
- `organization.*`는 런타임 직접 평가, `plant.*`는 전개 테이블 기반이라는 운영 규칙을 적는다.

**Step 2: Verify docs against code**

Run: `cargo test --workspace -- --nocapture`

Expected: PASS

**Step 3: Commit**

```bash
git add README.md docs/plans/2026-04-03-authorization-model-design.md
git commit -m "docs: describe authorization model implementation"
```

### Task 11: 전체 검증 및 마이그레이션 리허설

**Files:**
- Modify: `/Users/cypark/Documents/work/cert/crates/infra/tests/authorization_integration.rs`

**Step 1: Write the failing test**

- 총판 > 대리점 > plant grant > group role > user visibility 전체 흐름을 한 번에 검증하는 통합 테스트를 추가한다.
- `agreement` 종료 시 binding 비활성화 후 visibility/action 전개가 제거되는 테스트를 추가한다.

**Step 2: Run test to verify it fails**

Run: `cargo test -p infra authorization_integration -- --nocapture`

Expected: 통합 흐름 미구현 또는 일부 경계 실패

**Step 3: Write minimal implementation**

- 누락된 재계산 경로, cleanup, partial rebuild 유틸을 보완한다.
- full rebuild와 partial rebuild 명령을 내부 API 또는 서비스 함수로 정리한다.

**Step 4: Run test to verify it passes**

Run: `cargo test -p infra authorization_integration -- --nocapture`

Expected: PASS

**Step 5: Final verification**

Run: `cargo test --workspace -- --nocapture`

Expected: PASS

**Step 6: Commit**

```bash
git add crates/infra/tests/authorization_integration.rs crates/infra/src/database.rs crates/infra/src/service.rs
git commit -m "test: verify authorization expansion end-to-end"
```
