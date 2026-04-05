# TrailBase Auth Integration Design

목표는 `certd`의 기존 `organization` 기반 권한 모델을 유지하면서, 사용자 로그인과 세션 관리는 TrailBase를 원천으로 사용하는 것이다.

핵심 원칙은 다음과 같다.

- TrailBase는 `AuthN`만 담당한다.
- `certd + hiqlite`는 `AuthZ`와 도메인 데이터를 계속 담당한다.
- 사용자는 TrailBase에서 로그인하고, `certd`는 TrailBase auth bridge가 검증한 결과로 사용자를 식별한다.
- `certd`는 토큰 식별 결과를 기준으로 로컬 사용자와 조직 membership을 찾고, 기존 `effective_*` 전개 테이블로 권한을 판정한다.
- TrailBase와 `certd`는 별도 바이너리/별도 프로세스로 배포한다.

이 설계는 TrailBase의 기본 auth UI를 우선 사용하고, 나중에 필요하면 custom UI로 전환하는 것을 전제로 한다.

## TrailBase를 사용하는 이유

TrailBase 공식 문서 기준으로 다음이 현재 요구와 맞는다.

- 기본 auth UI를 제공한다: `/_/auth/login`
- custom UI도 가능하다
- short-lived JWT와 stateful refresh token을 함께 사용한다
- 다른 백엔드가 auth token을 검증할 수 있는 구조를 지향한다

참고:

- https://trailbase.io/documentation/auth/
- https://www.trailbase.io/getting-started/starting-up
- https://www.trailbase.io/getting-started/first-ui-app

즉 1차 버전에서는 TrailBase의 기본 UI와 auth API를 그대로 활용하고, `certd`는 별도 비즈니스 API로 유지하는 것이 가장 자연스럽다.

## 시스템 경계

### TrailBase

TrailBase는 다음만 담당한다.

- `users` auth collection
- 로그인, 로그아웃
- 이메일/비밀번호 인증
- 향후 OAuth, MFA, 이메일 검증
- 기본 auth UI
- 사용자 생명주기 관리
- 내부용 auth bridge endpoint

TrailBase는 조직, 발전소, 계약, 권한, 문서, 수집 상태를 저장하지 않는다.

### certd + hiqlite

`certd`는 다음을 계속 담당한다.

- `organizations`
- `organization_memberships`
- `groups`
- `agreements`
- `actions`
- `roles`
- `role_bindings`
- `effective_*`
- `plants`
- 수집기 실행
- 산출물/PDF 메타데이터

즉 `TrailBase = AuthN`, `certd = AuthZ + domain` 으로 명확히 분리한다.

## 배포 토폴로지

TrailBase와 `certd`는 하나의 바이너리로 합치지 않는다. 각각 별도 서비스로 띄운다.

권장 구성:

- `trailbase`
  - auth UI
  - auth API
  - `/api/certd/introspect`
- `certd`
  - public domain API
  - authorization engine
  - collector
- `minio`
  - backup/object storage

통신 방향:

- 브라우저 → TrailBase
  - 로그인, 로그아웃, 세션 갱신
- 브라우저 → certd
  - domain API 호출
- certd → TrailBase
  - bearer token 검증용 내부 introspection 호출

즉 사용자는 TrailBase에서 인증되고, 도메인 호출은 `certd`로 들어오며, `certd`가 필요할 때만 TrailBase auth bridge에 내부적으로 사용자 진위를 묻는다.

## 사용자 식별 모델

권한과 도메인 모델은 TrailBase의 사용자 row를 직접 foreign key로 물지 않는다. 대신 `shadow user + identity mapping`을 둔다.

권장 테이블:

- `users`
  - `id`
  - `display_name`
  - `email`
  - `status`
  - `created_at`
  - `updated_at`

- `user_identities`
  - `id`
  - `user_id`
  - `provider`
  - `external_user_id`
  - `email_snapshot`
  - `display_name_snapshot`
  - `last_seen_at`
  - unique `(provider, external_user_id)`

여기서:

- `provider = 'trailbase'`
- `external_user_id = TrailBase users.id`

이 구조를 두는 이유는 다음과 같다.

- 인증 원천을 바꿔도 도메인 user id를 유지할 수 있다.
- 권한/감사/조직 membership이 auth provider에 종속되지 않는다.
- 나중에 Google, SSO, 다른 IdP가 붙어도 동일 패턴으로 확장할 수 있다.

## 인증 흐름

정상 흐름은 다음과 같다.

1. 사용자가 TrailBase 기본 UI 또는 auth API로 로그인한다.
2. 클라이언트는 TrailBase auth token을 받는다.
3. 클라이언트는 `certd` 호출 시 `Authorization: Bearer <trailbase_token>`를 보낸다.
4. `certd` auth middleware가 TrailBase auth bridge의 introspection endpoint를 호출한다.
5. 검증 결과에서 TrailBase user id를 확인한다.
6. `user_identities`에서 로컬 `user_id`를 찾는다.
7. 없으면 최소 프로필로 `users`/`user_identities`를 자동 upsert 한다.
8. 요청의 `active organization` 컨텍스트를 확인한다.
9. 권한은 `effective_*` 전개 테이블에서 판정한다.

즉 인증 성공이 곧 권한 허용을 의미하지는 않는다. TrailBase는 "누구인지"만 알려주고, `certd`는 "무엇을 할 수 있는지"를 판단한다.

## 토큰 검증 방식

선택지는 세 가지가 있다.

### 1. TrailBase API를 매 요청 호출

- 장점: 단순하다
- 단점: 요청당 네트워크 비용이 크고, TrailBase 장애가 바로 `certd` 인증 장애가 된다

### 2. certd가 TrailBase JWT를 로컬 검증

- 장점: 빠르다
- 단점: TrailBase의 토큰 정책, 공개키, 무효화 정책을 `certd`가 강하게 알아야 한다

### 3. certd가 TrailBase introspection endpoint를 호출

- 장점: TrailBase를 진짜 인증 원천으로 유지할 수 있다
- 장점: user disable, 세션 종료, 향후 정책 변경을 TrailBase 쪽에서 일관되게 관리할 수 있다
- 단점: TrailBase에 내부용 endpoint가 추가로 필요하다

권장안은 3번이다.

### 구현된 introspection endpoint

TrailBase auth bridge 서비스에 내부 전용 endpoint를 하나 추가한다.

- `POST /api/certd/introspect`

입력:

- `Authorization: Bearer <trailbase_token>`
- 내부 호출 인증용 shared secret 헤더 `x-certd-internal-secret`

출력:

- `200 OK`
  - `valid = true`
  - `trailbase_user_id`
  - `email`
  - `display_name`
  - `verified`
  - `active`
  - `expires_at`
- 또는 `401/403`

이 endpoint는 공개 API가 아니라 `certd -> TrailBase` 내부 호출용이다. 현재 구현은 [`crates/trailbase-auth-bridge`](/Users/cypark/Documents/work/cert/crates/trailbase-auth-bridge) 에 있다.

### 권장 introspection 응답

`certd`가 필요로 하는 최소 응답은 다음과 같다.

- `valid`
- `trailbase_user_id`
- `email`
- `display_name`
- `verified`
- `active`
- `expires_at`

이 응답만으로 `certd`는 로컬 principal 매핑과 organization membership 확인을 수행할 수 있다.

## 캐시 정책

토큰 검증은 요청마다 TrailBase에 왕복하지 않도록 짧게 캐시한다.

권장:

- 메모리 캐시
- 키: auth token hash
- TTL: 15~60초

캐시를 짧게 가져가는 이유:

- 로그아웃
- 사용자 비활성화
- 세션 회수

같은 상태 변경이 비교적 빨리 반영되어야 하기 때문이다.

장기 저장은 권장하지 않는다. 인증 세션은 hiqlite 영속 데이터가 아니라 프로세스 메모리 캐시로 취급한다.

## active organization 컨텍스트

TrailBase는 사용자 인증만 담당하므로, "현재 어느 조직으로 일하는가"는 `certd`가 결정해야 한다.

권장 방식:

- 요청 헤더 또는 세션 보조 정보로 `X-Organization-Id`
- `certd`가 이 값으로 `organization_memberships` 확인
- 이후 `effective_user_plant_actions` / `effective_user_plant_visibility`를 `(user_id, organization_id)` 기준으로 조회

즉 같은 사람이 여러 조직에 속해 있어도, 매 요청은 항상 하나의 활성 조직 문맥에서 평가된다.

## TrailBase에 저장할 것과 저장하지 않을 것

TrailBase에 저장:

- auth user
- 최소 프로필
- 세션/refresh token

TrailBase에 저장하지 않음:

- organizations
- organization memberships
- groups
- roles
- role bindings
- effective authorization tables
- plants
- agreements
- collector artifacts

이렇게 해야 TrailBase 교체나 custom UI 전환 시 영향 범위가 작다.

## 권장 API 경계

### TrailBase

- `/_/auth/login`
- `/_/auth/logout`
- `/api/auth/v1/login`
- `/api/auth/v1/token`
- 내부용 introspection endpoint

### certd

- `/api/v1/organizations/...`
- `/api/v1/plants/...`
- `/api/v1/documents/...`
- `/api/v1/admin/backups`

`certd`는 TrailBase 세션을 직접 발급하지 않는다. 인증 토큰을 입력으로만 받는다.

## 구현 상태

이미 반영된 변경은 다음과 같다.

1. `certd`는 TrailBase bearer token만 받는다
- 헤더 기반 사용자 식별 모드는 제거됐다
- `Authorization: Bearer <token>` + `x-organization-id`만 허용한다

2. `user_identities` 테이블이 추가됐다
- TrailBase 사용자와 local user를 연결한다
- `provider = 'trailbase'`

3. auth middleware가 TrailBase introspection을 사용한다
- `CERTD_TRAILBASE_BASE_URL`
- `CERTD_TRAILBASE_INTROSPECT_PATH`
- `CERTD_TRAILBASE_INTERNAL_SECRET`
- `CERTD_TRAILBASE_CACHE_TTL_SECONDS`

4. 기존 권한 엔진을 그대로 재사용한다
- `organization.*`는 organization binding 직접 평가
- `plant.*`는 `effective_*` 전개 테이블 사용

5. TrailBase auth bridge는 독립 바이너리다
- 경로: [`crates/trailbase-auth-bridge`](/Users/cypark/Documents/work/cert/crates/trailbase-auth-bridge)
- 환경 변수:
  - `TRAILBASE_CERTD_INTERNAL_SECRET`
  - `TRAILBASE_DATA_DIR`
  - `TRAILBASE_ADDRESS`
  - `TRAILBASE_ADMIN_ADDRESS`

## 설계 요약

최종 구조는 다음 한 문장으로 요약된다.

TrailBase는 로그인과 세션의 원천이고, `certd`는 TrailBase 토큰으로 사용자를 식별한 뒤 로컬 `user_identities`를 통해 principal을 매핑하고, hiqlite의 `organization` 기반 권한 모델로 실제 접근 권한을 판정한다.
TrailBase와 `certd`는 별도 바이너리로 배포되며, `certd`는 내부 introspection 호출만으로 TrailBase와 통신한다.
