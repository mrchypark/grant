# certd 권한 모델

이 문서는 현재 코드와 스키마가 기대하는 조직, 그룹, 발전소 권한 전개 구조를 정리한다.

## 기본 엔티티

- `organization`
  - 사업자 조직의 기본 단위다.
  - 발전사업자, 총판, 대리점, 시공사, 유지보수사를 모두 포함한다.
- `group`
  - 조직 내부 팀 단위다.
  - 외부 조직 관계 표현에는 쓰지 않는다.
- `plant`
  - 하나의 `owner_organization_id`에 귀속된다.
  - `created_by_organization_id`로 최초 생성 조직을 기록한다.

## 권한 규칙

- `organization.*`
  - 조직 스코프 권한이다.
  - 예: `organization.plant.create`
  - 전개 테이블이 아니라 런타임 직접 평가로 처리한다.
- `plant.*`
  - 특정 발전소에 대한 권한이다.
  - 예: `plant.profile.read`, `plant.sync.execute`
  - 전개 결과를 `effective_*` 테이블로 조회한다.

## 목록 노출 규칙

- plant 목록은 `effective_user_plant_visibility`만 본다.
- 현재 1차 구현 기준으로 visibility를 만드는 read 권한은 `plant.profile.read` 하나다.
- `plant.document.read` 같은 다른 read 권한만으로는 plant 목록에 자동 노출되지 않는다.

즉, 문서 접근 권한이 있더라도 `plant.profile.read`가 없으면 목록에서 발전소가 보이지 않을 수 있다.

## 전개 테이블

권한 모델의 핵심은 아래 전개 결과에 있다.

- `organization_closure`
  - 조직 계층 전개
- `effective_organization_plant_actions`
  - 조직 기준 plant action 전개
- `effective_user_plant_actions`
  - 사용자 기준 plant action 전개
- `effective_user_plant_visibility`
  - 사용자 기준 plant 목록 노출 전개

테이블 정의와 제약은 [database-schema.md](./database-schema.md)를 기준으로 본다.

## 코드와의 연결

- write/create/deactivate 같은 조직 변경 작업은 voter 라우트에서만 허용된다.
- read 경로는 전개 결과를 이용해 plant 조회와 문서 목록 노출을 결정한다.
- learner 노드는 read/authz 보조 경로로 쓰일 수 있지만, write/admin surface는 열지 않는다.

## 문서 갱신 규칙

아래가 바뀌면 이 문서와 [database-schema.md](./database-schema.md)를 함께 갱신해야 한다.

- 조직/그룹/발전소 관계 의미
- canonical key 의미
- `effective_*` 전개 방식
- plant visibility 생성 규칙
