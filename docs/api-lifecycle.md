# API 책임과 수명주기 정비

## 범위

Grant는 조직 소속, 역할과 자원별 권한 부여, 앱의 권한 범위와 사용자 동의를 관리한다. IdP는 로그인, 클라이언트 생성, 로그인 설정과 자격 증명을 관리한다. Exposure는 등록된 HTTP 카탈로그에 따라 요청을 검사하고 고정된 upstream으로 전달한다.

이번 변경은 IdP 프록시 제거, 기존 IdP 클라이언트의 검증된 연결, Provider 생성과 수정 분리, 조직·앱·구성원·역할의 관리 수명주기를 대상으로 한다. 실시간 구독, 외부 개발자용 실행기 배포, PatchData/PatchAuth 변경은 포함하지 않는다.

## 변경 원칙

- 앱 등록은 Grant 권한 객체만 생성한다. Grant가 IdP 클라이언트 ID나 비밀 키를 발급하지 않는다.
- IdP 클라이언트 연결은 플랫폼 운영자만 요청할 수 있으며, 설정된 어댑터가 기존 클라이언트를 검증한 뒤 원자적으로 반영한다. 검증 실패 시 사용할 수 있는 앱으로 전환하지 않는다.
- Provider POST는 생성만 수행한다. 중복 ID는 409이며 기존 데이터를 덮어쓰지 않는다. 메타데이터 수정은 PATCH로 분리한다.
- 메타데이터 PATCH는 생략한 값을 보존한다. 관계 하나를 설정하는 PUT과 조건부 전체 집합 교체는 유지한다.
- 구성원 제거는 해당 조직의 개인 권한, 그룹 소속, 앱 동의를 함께 철회한다. 재가입으로 이전 개인 권한이 복원되지는 않지만 현재 조직 전체 권한은 다시 적용될 수 있다.
- 다른 조직의 앱·역할·구성원을 수정할 수 없다. 시스템 역할은 조직 관리자가 변경할 수 없다.
- 삭제와 권한 철회는 관련 참조를 원자적으로 처리하거나 명확한 충돌 오류로 거절한다.
- 새 관리 경로는 Grant의 관리 카탈로그에도 등록한다. 카탈로그 생성만으로 실제 앱이나 사용자에게 권한을 부여하지 않는다.

## 호환성과 적용 순서

`/auth`, `/auth/v1`, `/auth/v1/*`는 Grant에서 제거한다. IdP 트래픽은 Gateway에서 IdP로 직접 라우팅한다. 기존 인증 토큰 검증은 유지한다.

기존 수동 `provisioning/verify` 경로는 명시적인 `identity-binding`으로 교체한다. 기존에 검증된 연결 데이터는 보존한다. 기존 DB 컬럼 이름의 변경이나 대규모 데이터 이관은 별도 작업으로 분리한다.

Provider 등록을 반복해서 upsert로 사용하던 호출자는 생성의 409를 확인한 뒤 명시적인 PATCH를 수행하도록 변경해야 한다. 다른 오류를 중복 등록으로 간주하면 안 된다.

Grant 변경을 검증한 후 관리 카탈로그를 새 세대로 생성하고, 운영자가 신규 관리 Action을 필요한 앱·역할에 명시적으로 부여한다. 그 다음 카탈로그 활성화와 Exposure의 digest 설정을 함께 적용한다. 이전 catalog의 삭제된 경로가 동작한다고 가정하면 안 된다.

## 유지하거나 후속으로 검토할 사항

- 기존 `authz`와 `application-authz`의 단건·배치·필터 API는 별도 소비자 조사 없이 제거하지 않는다. 현재 Exposure는 HTTP catalog/decide를 사용한다.
- `me/materialize`는 현재 가입 전 신원 연결 계약이므로 호환성을 유지한다. 자동 조직 가입이나 자동 권한 부여로 바꾸지 않는다.
- 앱 trust 정책 분리, 업종별 Agreement 분류의 일반화, 전용 마이그레이션 HTTP API의 CLI 이전은 각각 데이터·운영 계약을 검토한 후 별도로 진행한다.

## 검증 기준

검토 대상은 이 브랜치의 변경 전체다. 범위·정확성·운영 순서·복잡도, API 호환성, 저장 데이터 무결성, 권한 경계를 각각 확인한다. 교차 조직 접근 거절, 생략 필드 보존, 중복 생성 시 불변, 미검증 앱 거절, 구성원 제거와 재가입 후 이전 권한 부활 방지를 테스트한다. 실제 배포 검증과 로컬 테스트 결과는 구분한다.

## 현재 추가 경로

아래 경로는 voter에서 처리한다. `GET /api/v1/orgs`는 Bearer만으로 인증하고 조직 헤더 없이 자신이 활성 구성원인 조직을 반환한다. `admin` 경로는 master key 또는 해당 관리 Action을 가진 플랫폼 사용자가 호출하며, 조직 경로는 URL 조직과 인증 조직의 일치 및 업무 권한을 확인한다.

| 요청 | 입력과 효과 |
|---|---|
| `POST /api/v1/admin/orgs` | `id`, `name`, `ownerUserId`. 기존 활성 사용자에게 새 조직의 owner 소속과 `role-system-bootstrap-owner`의 해당 조직 Binding을 함께 생성한다. IdP 계정은 만들지 않는다. 중복 조직 ID는 409다. |
| `GET /api/v1/orgs` | 현재 사용자의 활성 소속 조직 목록. 조직 비활성 상태도 조회 결과에 보일 수 있다. |
| `GET /api/v1/orgs/{organizationId}` | 조직 프로필을 조회한다. |
| `PATCH /api/v1/orgs/{organizationId}` | `name`만 수정한다. |
| `PATCH /api/v1/admin/providers/{providerId}` | `name`, `description`, `active` 중 전달한 항목만 수정한다. |
| `PUT /api/v1/admin/orgs/{organizationId}/apps/{applicationId}/identity-binding` | `clientId`로 기존 IdP 클라이언트를 검증하여 연결한다. 빈 값·null은 거부한다. |
| `PATCH /api/v1/orgs/{organizationId}/groups/{groupId}` | `name`, `description`을 부분 수정한다. |
| `PATCH /api/v1/orgs/{organizationId}/agreements/{agreementId}` | `title`, `agreementType`, `referenceNo`, `status`, `startsAt`, `endsAt`을 부분 수정한다. 상태·기간 변경은 grantor만 허용한다. 당사자 조직은 변경할 수 없다. |
| `PATCH /api/v1/orgs/{organizationId}/apps/{applicationId}` | `name`, `description`만 수정한다. IdP 설정·소유 조직·trust는 변경하지 않는다. |
| `PUT /api/v1/orgs/{organizationId}/apps/{applicationId}/active` | 필수 boolean `active`로 이용 상태를 설정한다. 비활성화 시 기존 활성 동의를 철회하며 재활성화로 동의가 복원되지 않는다. |
| `DELETE /api/v1/orgs/{organizationId}/apps/{applicationId}` | 앱의 동의·동의 Action·요청 Action·redirect manifest를 함께 제거한다. IdP 클라이언트는 삭제하지 않는다. |
| `PATCH /api/v1/orgs/{organizationId}/roles/{roleId}` | 조직 역할의 `name`, `description`을 부분 수정한다. |
| `DELETE /api/v1/orgs/{organizationId}/roles/{roleId}` | Binding이 없을 때 역할과 Action 연결을 제거한다. Binding이 하나라도 있으면 409다. |
| `GET /api/v1/orgs/{organizationId}/members` | 비활성 기록을 포함한 조직 소속 목록을 조회한다. |
| `DELETE /api/v1/orgs/{organizationId}/members/{userId}` | 소속을 inactive로 남기고 개인 Binding·그룹 소속·앱 동의를 철회한다. owner 제거는 409이며 소유권 이전은 별도 절차다. |
| `DELETE /api/v1/orgs/{organizationId}/app-grants/{grantId}` | 조직 관리자가 특정 동의를 철회한다. 이미 철회된 동의에도 204이며 다른 조직의 동의에는 적용할 수 없다. |

앱의 `first_party=1`이면 조직 API를 통한 활성 상태 변경과 삭제는 403이다. 배포 enforcer가 참조하는 앱은 삭제할 수 없으며 409다. 시스템 역할은 조직 소유 역할이 아니므로 조직 수정·삭제 API의 대상이 아니다. 역할 삭제는 Binding을 암묵적으로 지우지 않는다.

구성원 제거는 row 삭제 대신 inactive 기록을 보존한다. 재가입 시 이전 개인 Binding·그룹 소속·동의는 복원되지 않는다. 다만 현재 조직 전체에 부여된 권한은 활성 구성원이 되면 다시 적용된다. 따라서 재가입자가 모든 권한을 잃은 상태라고 단정해서는 안 된다.

메타데이터 PATCH는 생략 값을 보존하고 `description`, `referenceNo`, 기간처럼 nullable인 문자열의 null은 값을 지운다. 필수 이름·상태·boolean의 null, 알 수 없는 필드와 소유권·식별자 변경은 거부한다. 빈 PATCH는 400이다. 서로 다른 필드를 갱신할 때 전체 row를 덮어쓰지 않으며, 같은 필드의 동시 수정은 마지막 트랜잭션 값이 남는다. 이 PATCH에 ETag 보호가 추가된 것은 아니다. 기존 그룹·Agreement PUT은 호환성을 위해 유지하며 전체 교체 계약을 바꾸지 않는다.

## IdP 연결의 검증 범위

새 앱은 Rauthy 모드에서 `unprovisioned`, client ID 없음으로 저장한다. 운영자가 IdP에 별도로 만든 client ID를 플랫폼 관리 API에 전달한다. 현재 연결 검증 어댑터는 Rauthy만 지원하며 설정된 origin의 client read API로 ID·이름·활성 상태·클라이언트 종류·redirect manifest·flow·public PKCE를 확인한다. Grant는 외부 클라이언트를 생성하거나 갱신하지 않는다.

`GRANT_RAUTHY_CLIENT_READ_API_KEY`는 연결 검증에만 필요하다. 키가 없으면 해당 요청은 503이며 정상 토큰 검증을 위한 시작 조건은 아니다. Goauthy 등 지원하지 않는 모드의 연결 요청은 409로 거절하고 Rauthy로 대체 호출하지 않는다. PocketBase 모드는 이전처럼 client ID 없이 ready이며 이 연결 API를 사용하지 않는다.

검증 결과는 검증한 로컬 manifest·소속·상태가 여전히 같을 때만 원자적으로 저장한다. 다른 앱에 연결된 client ID, 준비된 앱의 다른 ID로 재연결, 비활성 조직·앱은 거절한다. 실패 시 ready로 바꾸지 않는다. 기존 검증 완료 데이터와 `rauthy_client_id` 컬럼은 보존하며 이름만 일반화하는 DB 변경은 수행하지 않는다. 새로운 API 이름이 Goauthy 지원을 의미하지 않는다.

이 문서는 코드 계약이다. IED 반영 여부는 별도의 배포·실기능 검증 기록으로 확인해야 한다.

## 배포 전환과 공개 범위의 제한

이전 검증 기록의 IED 관리 카탈로그는 generation 4, 51개 capability다. 이 브랜치의 생성 결과는 66개다. 이 작업에서 라이브 상태나 이미지 pin을 변경하지 않았으며, 적용 시 먼저 실제 활성 generation과 digest를 조회해야 한다. 다음 generation을 5로 고정해서는 안 된다.

배포 전에는 기존 Provider Action, 관리 앱의 Action, 관련 역할의 Action과 enforcer Binding을 읽어 보존한다. 새 정의만 등록하고 필요한 관리 앱·역할에 명시적으로 부여한다. 전체 집합을 교체해야 한다면 관련 없는 항목까지 보존하여 병합하고 GET으로 읽은 ETag를 If-Match로 보낸다. 412·428 또는 결과 불명 오류가 발생하면 현재 상태를 다시 조회하고 조건을 새로 계산한다. 쓰기를 무조건 재시도하지 않는다.

검토한 새 세대를 stage한 뒤 generation·digest·capability를 읽어 확인하고, 실제 기존 세대에 대한 compare-and-swap으로 활성화한다. 관리 Exposure의 digest 설정도 일치시킨다. 전환 중에는 private voter 제어 경로를 유지하며 master key를 Exposure로 전달하지 않는다. 신규 경로의 허용·거절, 교차 조직 차단, 삭제·철회 이후의 실제 접근 차단을 확인해야 배포 완료로 기록할 수 있다.

배포 저장소의 bootstrap은 빈 DB용이며 Provider 생성과 catalog의 초기 활성화를 전제로 한다. 기존 IED DB에 다시 실행하지 않는다. NoPVC와 GCS 및 분리 배포를 유지하고 MinIO를 추가하지 않는다. 현재 PocketBase 인증 profile은 제거한 Rauthy 프록시를 사용하지 않으므로 변경하지 않는다. 향후 Rauthy profile은 Gateway에서 IdP로 직접 라우팅해야 한다.

Grant에 직접 요청하는 `GET /api/v1/orgs`는 조직 헤더 없이 Bearer만 사용한다. 관리 Exposure를 통한 요청은 현재 `me`와 같은 기존 서비스 권한 정책의 조직 문맥을 요구한다. 따라서 이 추가만으로 조직이 없는 사용자가 Exposure에서 최초 조직을 탐색·가입할 수 있다고 설명해서는 안 된다. 조직 생성은 여전히 플랫폼 운영자의 명시적 작업이다.

## Agreement 시간과 감사 기록

새 Agreement 생성·PUT·PATCH는 RFC3339 입력을 파싱하고 UTC의 소수점 이하 정확히 9자리 고정 폭 문자열로 저장한다. 시작이 종료 이후이면 거절한다. 판정 시각도 같은 형식으로 만들어 시작·종료 경계를 포함해 비교하며 여유 시간을 추가하지 않는다. 이 검사는 사용자·그룹·조직 주체의 세 Binding 경로에 모두 적용한다.

기존 row는 일괄 변경하지 않는다. 비정규 기존 기간은 유효한 offset 시각이나 소수부 없는 UTC 문자열이라도 판정에서 거절한다. 권한 있는 grantor가 명시적인 PATCH로 정규화해야 다시 사용할 수 있다. 기간이 없는 null은 계속 무제한 경계이며 이 정책은 Agreement에 한정된다.

플랫폼 조직 생성이 기록하는 초기 owner Binding의 `created_by_user_id`는 지정된 owner가 아니라 실제 운영 행위자를 기록한다. 소유자와 작업자를 혼동하지 않는다. master key 호출의 행위자는 기존 운영 context 규약을 따른다.

구성원 제거는 관리자 소속도 `membership_role=member`, `status=inactive`로 내린다. 재가입이 관리자 지위를 복원하지 않도록 하며 owner는 제거 대상이 아니다. 앱 Action 집합의 ETag는 앱 ID·조직 ID·생성 시각도 포함하므로 앱 삭제 후 같은 ID로 다시 생성한 경우 이전 ETag로 교체할 수 없다. GET의 공개 Action 배열 구조는 그대로다.

grantee의 전체 PUT은 보존한 기간이라도 정규화 때문에 판정 의미가 달라지는 기존 row에 대해 403이다. grantor가 기간을 먼저 정규화해야 한다. grantee는 메타데이터 PATCH를 계속 사용할 수 있으며 이 경로는 저장된 기간을 변경하지 않는다.
