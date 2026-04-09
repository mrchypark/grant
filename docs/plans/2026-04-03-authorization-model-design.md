# Authorization Model Design

> Superseded on 2026-04-10. The current implementation removed `organization_edges` and `organization_closure`; organization-to-organization plant access now exists only when an explicit agreement-backed role binding exists.

목표는 `cert`의 사업자/발전소 권한 모델을 실제 운영 구조에 맞게 다시 정의하는 것이다.

핵심 전제는 다음과 같다.

- `organization`이 사업자 조직의 기본 단위다.
- 발전사업자, 총판, 대리점, 시공사, 유지보수사는 모두 `organization`이다.
- `group`은 조직 내부 팀이다. 대리점이나 총판은 `group`으로 표현하지 않는다.
- `plant`는 특정 시점에 하나의 소유 조직(`owner organization`)에 귀속된다.
- 사용자는 여러 조직에 속할 수 있지만, 모든 권한 평가는 `active organization` 컨텍스트 기준으로 한다.

## 조직과 발전소

권장 모델은 `account`를 별도 테넌트로 두지 않고 `organization` 중심으로 간다.

- `organizations`
- `users`
- `organization_memberships`
- `groups`
- `group_memberships`
- `plants`

조직 간 plant 접근은 상하위 관계로 상속하지 않는다. 총판, 대리점, 시공사, 유지보수사 모두 별도 `organization`이고, 접근은 각 조직이 가진 명시 계약과 role binding으로만 생긴다.

발전소는 다음 두 축을 가진다.

- `owner_organization_id`: 현재 발전사업자
- `created_by_organization_id`: 최초 생성 조직, 예를 들면 시공사

이렇게 두면 "시공사가 생성 후 발전사업자에게 이관하고, 이후에도 권한을 유지"하는 흐름을 표현할 수 있다.

## 계약과 권한 근거

권한의 기간과 근거는 `role_bindings`에 직접 넣지 않고 `agreements`로 분리한다.

- `agreements`
  - `agreement_type`
  - `grantor_organization_id`
  - `grantee_organization_id`
  - `status`
  - `title`
  - `reference_no`
  - `starts_at`
  - `ends_at`

외부 조직에 대한 발전소 권한은 가능하면 항상 `agreement`를 근거로 둔다. 계약 종료 시 관련 binding을 비활성화하고 전개 테이블을 다시 계산한다.

운영 규칙:

- `agreement`의 `status`, `starts_at`, `ends_at` 변경은 즉시 권한 전개 재계산을 트리거해야 한다.
- `plant` 전개는 현재 시점에 유효한 `agreement`만 반영한다.
- 따라서 종료되거나 만료된 계약에 기대고 있던 외부 조직 권한은 `effective_*`에서 사라져야 한다.

이렇게 하면 `role_bindings`는 현재 유효한 권한 원본만 담당하고, 계약/감사/업무 근거는 `agreements`가 담당한다.

## 액션과 롤

액션은 두 스코프로 나눈다.

- `organization.*`
- `plant.*`

`organization` 스코프는 발전소 생성이나 조직 관리처럼 특정 발전소가 아직 없거나 plant 집합 전체에 대해 동작하는 권한을 의미한다.

예:

- `organization.plant.create`
- `organization.plant.create_for_owner`
- `organization.member.manage`
- `organization.group.manage`
- `organization.role.manage`
- `organization.permission.manage`

`plant` 스코프는 특정 plant에 대한 권한이다.

예:

- `plant.profile.read`
- `plant.profile.write`
- `plant.telemetry.read`
- `plant.alert.read`
- `plant.document.read`
- `plant.document.write`
- `plant.report.read`
- `plant.report.write`
- `plant.rec.read`
- `plant.rate.read`
- `plant.sync.execute`
- `plant.device.read`
- `plant.device.control`
- `plant.certificate.use`
- `plant.permission.manage`
- `plant.ownership.transfer`

`plant.read` 같은 루트 read는 의미가 모호하므로 사용하지 않는다. 액션은 항상 무엇을 읽거나 수정하는지 드러내야 한다.

`organization.*` 액션은 `plant` 전개 테이블로 내리지 않는다. 이 권한들은 수가 적고 조직 스코프에서만 평가되므로, `organization` resource binding을 직접 조회하는 런타임 평가 경로로 처리한다.

액션 정의 테이블은 다음과 같다.

- `actions`
  - `code`
  - `name`
  - `description`
  - `risk_level`
  - `visibility_granting`

- `roles`
  - 시스템 공통 role 또는 조직 전용 role

- `role_rules`
  - wildcard rule 정의용

- `role_actions`
  - wildcard를 실제 action으로 전개한 결과

wildcard는 정의용 문법으로만 사용한다. 런타임은 항상 `role_actions`만 사용한다.

허용하는 예시는 다음과 같다.

- `plant.*`
- `plant.*.read`
- `plant.document.*`

비허용:

- `*.read`
- `plant.**`
- 임의 정규식

## 권한 부여 원본

`role_bindings`는 통합 테이블로 유지한다. 전개 테이블을 별도로 둘 것이므로, 원본 규칙은 표현력 중심으로 설계해도 된다.

- `role_bindings`
  - `id`
  - `subject_type`
  - `subject_id`
  - `subject_organization_id`
  - `resource_type`
  - `resource_id`
  - `role_id`
  - `agreement_id` nullable
  - `state`
  - `created_by_user_id`
  - `created_at`
  - `updated_at`

`subject_type`은 다음 셋만 허용한다.

- `organization`
- `group`
- `user`

`resource_type`은 1차 구현에서 다음 둘을 허용한다.

- `organization`
- `plant`

이 테이블의 의미는 다음과 같다.

- `organization` binding:
  - 외부 조직 또는 소유 조직이 어떤 plant 또는 organization scope에 대해 어떤 role을 가지는지 표현
- `group` binding:
  - 조직 내부 그룹 권한
- `user` binding:
  - 조직 내부 사용자 예외 권한

`subject_organization_id`는 항상 조직 문맥을 명시한다. 사용자가 여러 조직에 속할 수 있기 때문에 이 컬럼이 있어야 권한을 안전하게 해석할 수 있다.

외부 조직에 대한 plant 권한은 조직 binding으로 부여하고, 내부 사용자/그룹 권한은 같은 plant에 대해 더 좁은 role로 다시 분배한다. 내부 권한은 조직이 plant에 대해 가진 상한 권한을 넘을 수 없다.

## 조회 최적화와 전개 테이블

권한 원본을 그대로 조인해서 `list plants`를 풀면 느려진다. 특히 조직별 명시 binding, 그룹 membership, user override, role-action 전개가 한 번에 들어가면 hot path가 복잡해진다.

따라서 권한 정의와 조회를 분리한다.

원본 규칙:

- `organization_memberships`
- `group_memberships`
- `role_actions`
- `role_bindings`

전개 결과:

- `effective_organization_plant_actions`
- `effective_user_plant_actions`
- `effective_user_plant_visibility`

### `effective_organization_plant_actions`

조직 기준으로 어떤 plant에 어떤 action을 가지는지 전개한다.

- `organization_id`
- `plant_id`
- `action_id`
- `updated_at`

입력:

- `role_bindings(subject_type=organization)`
- `role_actions`

### `effective_user_plant_actions`

현재 조직 컨텍스트에서, 어떤 사용자가 어떤 plant에 어떤 action을 가지는지 전개한다.

- `user_id`
- `organization_id`
- `plant_id`
- `action_id`
- `updated_at`

입력:

- `effective_organization_plant_actions`
- `role_bindings(subject_type=group|user)`
- `group_memberships`
- `organization_memberships`
- `role_actions`

### `effective_user_plant_visibility`

plant 목록 조회용 전개 테이블이다.

- `user_id`
- `organization_id`
- `plant_id`
- `updated_at`

생성 규칙:

- `effective_user_plant_actions`
- join `actions.visibility_granting = true`
- 한 plant에 대해 visibility action이 하나라도 있으면 row 생성

즉 `plant.list` 액션을 별도로 두지 않는다. 목록에 보이는지 여부는 visibility 전개 결과로 결정한다.

1차 구현에서 visibility는 `plant.profile.read` 하나만 기준으로 삼는다. 즉 어떤 사용자에게 다른 read action이 있더라도 `plant.profile.read`가 없으면 plant 목록에는 노출하지 않는다.

이는 visibility를 UI 편의가 아니라 discovery 권한으로 보기 때문이다. 예를 들어 문서 작업이나 telemetry 조회 권한이 있더라도, plant 자체 존재를 목록에 노출할지 여부는 별도로 더 보수적으로 제어한다.

## 권한 평가 방식

단건 권한 체크는 다음 쿼리로 끝나야 한다.

```sql
SELECT 1
FROM effective_user_plant_actions
WHERE user_id = ?
  AND organization_id = ?
  AND plant_id = ?
  AND action_id = ?;
```

plant 목록 조회는 다음 쿼리로 끝나야 한다.

```sql
SELECT p.*
FROM effective_user_plant_visibility v
JOIN plants p ON p.id = v.plant_id
WHERE v.user_id = ?
  AND v.organization_id = ?
ORDER BY p.name;
```

즉 실제 API hot path는 원본 권한 규칙을 직접 조인하지 않고 전개 결과만 조회한다.

## 로그와 이력

로그성 기록은 외부 시스템으로 보낸다고 가정한다. 따라서 현재 설계에는 다음을 포함하지 않는다.

- 액션 실행 이력 테이블
- 권한 변경 감사 로그 테이블

현재 DB는 최신 상태와 전개 결과만 보유한다.

## 권장 구현 순서

1. `organization`, `group`, `plant`, `agreement`, `action`, `role` 스키마 정리
2. `role_bindings` 도입
3. `effective_organization_plant_actions` 전개 구현
4. `effective_user_plant_actions` 전개 구현
5. `effective_user_plant_visibility` 전개 구현
6. 목록 API와 단건 권한 체크를 전개 테이블 기반으로 전환

## 결론

이 설계의 핵심은 다음 한 문장으로 요약된다.

정의는 정규화하고, 조회는 전개한다.

권한 원본은 `role_bindings`와 `role_actions`가 담당하고, 실제 목록 및 액션 판정은 `effective_*` 전개 테이블이 담당한다. 이렇게 해야 명시 계약 기반 조직 접근과 발전소별 세밀한 권한 모델을 동시에 유지하면서도 `user -> visible plants` 조회 성능을 확보할 수 있다.
