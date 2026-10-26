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
- `POST /api/v1/me/ensure`는 가입 전 신원 연결 계약이다. 이 경로는 이전 `POST /api/v1/me/materialize`를 대체한다. 카탈로그 Action ID `user.materialize`는 배포된 권한 참조와 호환되도록 유지하지만, 경로·표시 이름·설명은 ensure 계약을 설명해야 한다. 자동 조직 가입이나 자동 권한 부여로 바꾸지 않는다.
- 앱 trust 정책 분리, 전용 마이그레이션 HTTP API의 CLI 이전은 각각 데이터·운영 계약을 검토한 후 별도로 진행한다.

## 검증 기준

검토 대상은 이 브랜치의 변경 전체다. 범위·정확성·운영 순서·복잡도, API 호환성, 저장 데이터 무결성, 권한 경계를 각각 확인한다. 교차 조직 접근 거절, 생략 필드 보존, 중복 생성 시 불변, 미검증 앱 거절, 구성원 제거와 재가입 후 이전 권한 부활 방지를 테스트한다. 실제 배포 검증과 로컬 테스트 결과는 구분한다.

## 현재 추가 경로

아래 경로는 voter에서 처리한다. `GET /api/v1/orgs`는 Bearer만으로 인증하고 조직 헤더 없이 자신이 활성 구성원인 조직을 반환한다. `admin` 경로는 master key 또는 해당 관리 Action을 가진 플랫폼 사용자가 호출하며, 조직 경로는 URL 조직과 인증 조직의 일치 및 업무 권한을 확인한다.

| 요청 | 입력과 효과 |
|---|---|
| `POST /api/v1/admin/orgs` | `id`, `name`, `ownerUserId`. 기존 활성 사용자에게 새 조직의 canonical owner 소속을 생성한다. IdP 계정은 만들지 않는다. 중복 조직 ID는 409다. |
| `GET /api/v1/orgs` | 현재 사용자의 활성 소속 조직 목록. 조직 비활성 상태도 조회 결과에 보일 수 있다. |
| `GET /api/v1/orgs/{organizationId}` | 조직 프로필을 조회한다. |
| `PATCH /api/v1/orgs/{organizationId}` | `name`만 수정한다. |
| `PATCH /api/v1/admin/providers/{providerId}` | `name`, `description`, `active` 중 전달한 항목만 수정한다. |
| `PUT /api/v1/admin/orgs/{organizationId}/apps/{applicationId}/identity-binding` | `clientId`로 기존 IdP 클라이언트를 검증하여 연결한다. 빈 값·null은 거부한다. |
| `PATCH /api/v1/orgs/{organizationId}/groups/{groupId}` | `name`, `description`을 부분 수정한다. |
| `PATCH /api/v1/orgs/{organizationId}/agreements/{agreementId}` | `title`, `agreementType`, `referenceNo`, `status`, `startsAt`, `endsAt`을 부분 수정한다. 상태·기간 변경은 grantor만 허용한다. 당사자 조직은 변경할 수 없다. |
| `PATCH /api/v1/orgs/{organizationId}/apps/{applicationId}` | `name`을 수정하거나 identity binding 전 미연결 앱의 `redirectUri`를 설정한다. `redirectUri`는 create와 같은 URL 검증을 거치며 null로 지울 수 없다. ready/bound·platform-managed 앱에서는 변경할 수 없다. IdP client·secret, application type, 소유 조직·trust와 기타 메타데이터는 변경하지 않는다. |
| `PUT /api/v1/orgs/{organizationId}/apps/{applicationId}/active` | 필수 boolean `active`로 이용 상태를 설정한다. 비활성화 시 기존 활성 동의를 철회하며 재활성화로 동의가 복원되지 않는다. |
| `DELETE /api/v1/orgs/{organizationId}/apps/{applicationId}` | 앱의 동의·동의 Action·요청 Action·redirect manifest를 함께 제거한다. IdP 클라이언트는 삭제하지 않는다. |
| `PATCH /api/v1/orgs/{organizationId}/roles/{roleId}` | 조직 역할의 `name`, `description`을 부분 수정한다. |
| `DELETE /api/v1/orgs/{organizationId}/roles/{roleId}` | Binding이 없을 때 역할과 Action 연결을 제거한다. Binding이 하나라도 있으면 409다. |
| `GET /api/v1/orgs/{organizationId}/members` | 비활성 기록을 포함한 조직 소속 목록을 조회한다. |
| `DELETE /api/v1/orgs/{organizationId}/members/{userId}` | 소속을 inactive로 남기고 개인 Binding·그룹 소속·앱 동의를 철회한다. owner 제거는 409이며 먼저 owner PUT 경로로 소유권을 이전해야 한다. |
| `DELETE /api/v1/orgs/{organizationId}/app-grants/{grantId}` | 조직 관리자가 특정 동의를 철회한다. 이미 철회된 동의에도 204이며 다른 조직의 동의에는 적용할 수 없다. |

앱의 `first_party=1`이면 조직 API를 통한 활성 상태 변경과 삭제는 403이다. 배포 enforcer가 참조하는 앱은 삭제할 수 없으며 409다. 시스템 역할은 조직 소유 역할이 아니므로 조직 수정·삭제 API의 대상이 아니다. 역할 삭제는 Binding을 암묵적으로 지우지 않는다.

구성원 제거는 row 삭제 대신 inactive 기록을 보존한다. 재가입 시 이전 개인 Binding·그룹 소속·동의는 복원되지 않는다. 다만 현재 조직 전체에 부여된 권한은 활성 구성원이 되면 다시 적용된다. 따라서 재가입자가 모든 권한을 잃은 상태라고 단정해서는 안 된다.

메타데이터 PATCH는 생략 값을 보존하고 `description`, `referenceNo`, 기간처럼 nullable인 문자열의 null은 값을 지운다. 필수 이름·상태·boolean의 null, 알 수 없는 필드와 소유권·식별자 변경은 거부한다. 빈 PATCH는 400이다. 서로 다른 필드를 갱신할 때 전체 row를 덮어쓰지 않으며, 같은 필드의 동시 수정은 마지막 트랜잭션 값이 남는다. 이 PATCH에 ETag 보호가 추가된 것은 아니다. 기존 그룹·Agreement PUT은 호환성을 위해 유지하며 전체 교체 계약을 바꾸지 않는다.

## IdP 연결의 검증 범위

새 앱은 Rauthy와 Goauthy 모드에서 `unprovisioned`, client ID 없음으로 저장한다. Rauthy는 기존 client의 ID·이름·활성 상태·종류·redirect manifest·flow·public PKCE를 읽기 전용으로 검증한다. Goauthy는 설정된 application과 introspection/expected client ID가 모두 일치하는 confidential bootstrap client만 지원하며, 기존 Basic 자격 증명으로 `GET /oidc/client-configuration`을 읽는다. 정확한 redirect 목록, 허용 scope와 flow, resource를 포함한 `audience`, 정확히 `[resource]`인 `default_aud`, 응답 원문의 SHA-256 ETag와 `Cache-Control: no-store`를 검증한다. Grant는 외부 클라이언트를 생성하거나 갱신하지 않는다.

`GRANT_RAUTHY_CLIENT_READ_API_KEY`는 Rauthy 연결 검증에만 필요하며 없으면 해당 요청은 503이다. Goauthy는 기존 introspection client 자격 증명을 사용하며 별도 client-read API key를 요구하지 않는다. PocketBase 모드는 이전처럼 client ID 없이 ready이며 이 연결 API를 사용하지 않는다.

검증 결과는 검증한 로컬 manifest·소속·상태가 여전히 같을 때만 원자적으로 저장한다. 다른 앱에 연결된 client ID, 준비된 앱의 다른 ID로 재연결, 비활성 조직·앱은 거절한다. 실패 시 ready로 바꾸지 않는다. 기존 검증 완료 데이터와 `rauthy_client_id` 컬럼은 보존하며 이름만 일반화하는 DB 변경은 수행하지 않는다.

내부 PEP 서비스는 application이 아니다. 배포는 `GRANT_HTTP_CAPABILITY_PEP_CLIENT_ID`로 정확한 PEP identity를 고정하고, master 전용 `/ops/providers/{providerId}/enforcers/{pepClientId}`로 provider binding을 등록한다. 요청 body에는 `lifecycle`과 선택적 `validUntil`만 있으며 `applicationId`는 허용하지 않는다. 기존 user/application management checks는 `GRANT_MANAGEMENT_APPLICATION_ID`를 사용한다. `GRANT_HTTP_CAPABILITY_PEP_APPLICATION_ID`는 이 application ID의 deprecated alias일 뿐 PEP identity fallback이 아니며, 새 설정과 값이 다르면 시작을 거부한다. 일반 사용자 앱은 기존 읽기 전용 verifier, trust, consent 절차를 그대로 따른다.

이 문서는 코드 계약이다. IED 반영 여부는 별도의 배포·실기능 검증 기록으로 확인해야 한다.

## 배포 전환과 공개 범위의 제한

이전 검증 기록의 IED 관리 카탈로그는 generation 4, 51개 capability다. 이 브랜치의 생성 결과는 69개다. 이 작업에서 라이브 상태나 이미지 pin을 변경하지 않았으며, 적용 시 먼저 실제 활성 generation과 digest를 조회해야 한다. 다음 generation을 5로 고정해서는 안 된다.

배포 전에는 기존 Provider Action, 관리 앱의 Action, 관련 역할의 Action과 enforcer Binding을 읽어 보존한다. 새 정의만 등록하고 필요한 관리 앱·역할에 명시적으로 부여한다. 전체 집합을 교체해야 한다면 관련 없는 항목까지 보존하여 병합하고 GET으로 읽은 ETag를 If-Match로 보낸다. 412·428 또는 결과 불명 오류가 발생하면 현재 상태를 다시 조회하고 조건을 새로 계산한다. 쓰기를 무조건 재시도하지 않는다.

검토한 새 세대를 stage한 뒤 generation·digest·capability를 읽어 확인하고, 실제 기존 세대에 대한 compare-and-swap으로 활성화한다. 관리 Exposure의 digest 설정도 일치시킨다. 전환 중에는 private voter 제어 경로를 유지하며 master key를 Exposure로 전달하지 않는다. 신규 경로의 허용·거절, 교차 조직 차단, 삭제·철회 이후의 실제 접근 차단을 확인해야 배포 완료로 기록할 수 있다.

배포 저장소의 bootstrap은 빈 DB용이며 Provider 생성과 catalog의 초기 활성화를 전제로 한다. 기존 IED DB에 다시 실행하지 않는다. NoPVC와 GCS 및 분리 배포를 유지하고 MinIO를 추가하지 않는다. 현재 PocketBase 인증 profile은 제거한 Rauthy 프록시를 사용하지 않으므로 변경하지 않는다. 향후 Rauthy profile은 Gateway에서 IdP로 직접 라우팅해야 한다.

Grant와 관리 Exposure는 정해진 네 온보딩 경로에 한해 조직 헤더 없이 요청을 처리한다. 아래 온보딩 절차의 앱·인증·Action 조건을 충족해야 하며, 일반 조직 관리와 데이터 API의 조직 문맥 요구는 유지한다.

## Agreement 시간과 감사 기록

새 Agreement 생성·PUT·PATCH는 RFC3339 입력을 파싱하고 UTC의 소수점 이하 정확히 9자리 고정 폭 문자열로 저장한다. 시작이 종료 이후이면 거절한다. 판정 시각도 같은 형식으로 만들어 시작·종료 경계를 포함해 비교하며 여유 시간을 추가하지 않는다. 이 검사는 사용자·그룹·조직 주체의 세 Binding 경로에 모두 적용한다.

기존 row는 일괄 변경하지 않는다. 비정규 기존 기간은 유효한 offset 시각이나 소수부 없는 UTC 문자열이라도 판정에서 거절한다. 권한 있는 grantor가 명시적인 PATCH로 정규화해야 다시 사용할 수 있다. 기간이 없는 null은 계속 무제한 경계이며 이 정책은 Agreement에 한정된다.

플랫폼 조직 생성이 기록하는 초기 owner Binding의 `created_by_user_id`는 지정된 owner가 아니라 실제 운영 행위자를 기록한다. 소유자와 작업자를 혼동하지 않는다. master key 호출의 행위자는 기존 운영 context 규약을 따른다.

구성원 제거는 관리자 소속도 `membership_role=member`, `status=inactive`로 내린다. 재가입이 관리자 지위를 복원하지 않도록 하며 owner는 제거 대상이 아니다. 앱 Action 집합의 ETag는 앱 ID·조직 ID·생성 시각도 포함하므로 앱 삭제 후 같은 ID로 다시 생성한 경우 이전 ETag로 교체할 수 없다. GET의 공개 Action 배열 구조는 그대로다.

grantee의 전체 PUT은 보존한 기간이라도 정규화 때문에 판정 의미가 달라지는 기존 row에 대해 403이다. grantor가 기간을 먼저 정규화해야 한다. grantee는 메타데이터 PATCH를 계속 사용할 수 있으며 이 경로는 저장된 기간을 변경하지 않는다.

## Agreement 분류 일반화

`agreementType`은 서비스가 정하는 분류 메타데이터다. 기존 필드명과 기존 분류값을 유지하면서 `maintenance-contract`, `공동 운영` 같은 새로운 분류도 받는다. 생성·PUT·PATCH의 규칙은 동일하다. 앞뒤 공백을 제거한 뒤 1–128자 UTF-8 문자열이어야 하며 제어 문자는 허용하지 않는다. PATCH에서 생략하면 기존 값을 유지하고 null이나 빈 문자열은 거절한다.

분류 변경은 권한의 허용·거절을 바꾸지 않는다. 실제 위임 권한은 기존처럼 당사자 조직, 활성 상태, 유효기간 및 역할 Binding으로 결정된다. 상태·기간 변경의 grantor 전용 제한도 유지한다. 분류는 기존 메타데이터 수정과 같이 권한 있는 당사자 조직이 수정할 수 있다. 이 변경에는 DB 마이그레이션이나 기존 값 치환이 없다.

## 조직 소유권 이전

canonical owner는 활성 `organization_memberships.membership_role=owner`다. 여러 owner를 지원하는 `GET /api/v1/orgs/{organizationId}/owners`는 `{"organizationId":"…","owners":[{"userId":"…"}]}`와 ETag를 반환한다. GET도 활성 owner만 호출할 수 있고 `organization.profile.read`가 필요하다. `PUT`과 `DELETE /api/v1/orgs/{organizationId}/owners/{userId}`는 body 없이 `If-Match`를 요구하며 최신 owner envelope와 ETag를 반환한다. 변경은 활성 owner와 `organization.permission.manage`가 필요하다. 누락 tag는 428, stale tag는 412, 마지막 활성 owner 제거는 409다. PUT으로 기존 owner를 다시 추가하는 것은 idempotent이며 ETag를 바꾸지 않는다.

기존 `GET/PUT /api/v1/orgs/{organizationId}/owner`는 단일 owner 조직에만 호환된다. 여러 활성 owner가 있으면 409로 거절하며 잘못된 단일 owner 이전을 성공 처리하지 않는다. 일반 metadata PATCH로 owner를 바꿀 수 없다.

legacy 이전은 현재 단일 owner에게 관리 권한이 있어야 하고 두 사용자 모두 활성 사용자이자 해당 활성 조직의 활성 구성원이어야 한다. 대상의 자동 가입이나 사용자 생성은 하지 않는다. `If-Match`가 없으면 428, 변경된 ETag이면 412, 현재 owner가 아닌 호출자 또는 관리 권한이 철회된 owner이면 403이다. 같은 사용자에게 이전하거나 대상이 존재하지 않거나 비활성인 경우는 400이다. owner가 없거나 여러 명인 조직은 409로 거절한다.

owner 판정은 membership에서 계산하며 owner 전용 system-role binding은 authority source가 아니다. demotion은 membership만 변경하고 무관한 기존 delegated binding은 보존한다. role/binding 편집으로 owner를 만들거나 owner 관리 권한을 제거할 수 없다. owner 권한은 해당 조직의 관리 action에만 적용되며 플랫폼 wildcard 또는 application admission을 부여하지 않는다.

## 조직 범위 Operable 사용자 정의 Action

`POST /api/v1/orgs/{organizationId}/providers/{providerId}/actions`는 `operable` provider에 한해 `code`, `name`, `description`, `riskLevel`을 받는다. `PATCH .../actions/{actionId}`는 `name`, `description`, `riskLevel`만 받고 DELETE는 본문이 없다. 세 mutation 모두 active owner와 `organization.permission.manage`가 필요하다. `code`는 `custom.org-<organization-id의 소문자 SHA-256 64자리 hex>.<feature>.<definition-id>.<operation>` 형식이고, DB ID는 `operable.` + code로 생성한다. 이 namespace 검증은 create/edit/delete transaction에서 조직 owner 재검사와 함께 수행된다. built-in 및 role/application/active HTTP catalog가 참조하는 action은 409로 보호한다. 이 경로는 provider 관리 UI/API나 platform action, application admission을 제공하지 않는다.

## 조직 없는 온보딩과 조직 생성

관리 Exposure를 통과하는 조직 없는 요청은 다음 네 경로로 한정한다.

1. `POST /api/v1/me/ensure`: body 없이 Bearer로 호출한다. 검증된 외부 신원에 대응하는 내부 사용자와 `user_identities` 매핑이 없으면 생성하고, 있으면 프로필을 갱신하지 않고 같은 사용자를 재사용한다. 성공 응답은 `{"userId":"..."}`만 포함한다. 기존 내부 사용자가 비활성이면 403이다. 조직 소속, 역할, Binding, 동의는 생성하거나 부여하지 않는다.
2. `GET /api/v1/me`: 현재 내부 사용자와 소속 조직을 조회할 뿐 사용자·매핑을 만들거나 프로필을 쓰지 않는다. 매핑이 없으면 `LOCAL_USER_NOT_PROVISIONED` 코드와 함께 404를 반환한다.
3. `GET /api/v1/orgs`: 현재 사용자의 조직을 조회한다. 소속이 없으면 빈 목록이다.
4. `POST /api/v1/orgs`: `{"id":"새 조직 ID","name":"조직 이름"}`으로 본인 조직을 만든다. owner는 검증된 현재 사용자로 결정한다. `ownerUserId`는 입력받지 않으며 기존 ID는 409다.

이 예외는 무인증 공개가 아니다. 정확한 `grant-management` Provider·메서드·경로·Action과 `service/grant` 자원, `forward-user` 요청 계약을 확인한다. 설정된 관리 앱은 active/ready 및 first-party/auto-approve 상태여야 한다. 앱의 조직과 Provider도 활성 상태여야 하며, 해당 Action을 앱에 명시적으로 등록하고 일치하는 활성 enforcer로 요청해야 한다. IdP가 검증한 token client와 앱의 연결도 일치해야 한다. 일반 앱이나 다른 경로로 이 예외를 확장하지 않는다.

PDP, `GET /api/v1/me`, 일반 인증 조회는 provider·issuer·collection·subject를 포함한 외부 신원 digest와 기존 매핑을 조회만 한다. 이 경로들은 consistency 수준과 관계없이 내부 사용자 생성, 매핑 생성, 프로필 갱신을 수행하지 않는다. 실제 `POST /api/v1/me/ensure`만 사용자와 매핑을 반영한다. 이미 존재하는 비활성 사용자는 최초 방문자로 취급하지 않고 거절한다. 이메일이나 외부 subject와 내부 ID의 우연한 일치로 다른 사용자를 채택하지 않는다.

`GET /api/v1/me`는 `consistency=strong`, `local`, `prefer-strong`을 지원한다. consistency는 조회 결과의 신선도를 선택하며 사용자나 매핑의 생성을 허용하지 않는다. 다른 경로는 기존처럼 명시적 consistency override로 `strong`만 허용한다. `local` 조회의 `LOCAL_USER_NOT_PROVISIONED` 404는 복제 지연 때문에 발생할 수도 있으므로, ensure 직후 최신 사용자 정보가 필요하면 `GET /api/v1/me?consistency=strong`을 호출한다.

본인 조직 생성은 기본 Grant owner 역할뿐 아니라 보호된 `role-system-management-owner`의 새 조직 범위 Binding을 만든다. 이 역할에는 카탈로그에 등록된 조직 관리 Action만 포함되며 플랫폼 관리나 다른 조직의 권한은 포함되지 않는다. 필요한 Action이 누락되면 조직 생성 전체를 거절한다. 기존 owner에게 새 Action을 무조건 부여하는 배포 스크립트로 이 동작을 대체해서는 안 된다.

관리 앱이 다른 조직에 속하더라도 정확한 조직 관리 카탈로그 경로에 한해서는 대상 조직의 활성 구성원 여부와 해당 조직의 세부 RBAC를 검사하여 처리한다. 앱의 신뢰·명시적 Action 등록과 외부 신원 검사는 계속 필요하다. 일반 Provider·데이터 API, 플랫폼 관리 API 및 다른 앱에 대한 조직 간 허용으로 확장하지 않는다. 조직 생성 이후의 관리 요청에는 대상 조직 헤더를 사용한다.

이 절은 구현 계약이다. 새 이미지와 카탈로그가 실제 IED에 반영되었거나 라이브 테스트를 통과했다는 뜻은 아니다.

## 온보딩 배포 전 점검과 기존 관리 앱 갱신

이미지를 교체하거나 69개 카탈로그만 생성해도 온보딩을 사용할 수 있는 것은 아니다. 배포 전 다음 상태가 함께 준비되어야 한다.

- `grant-management` Provider와 새 카탈로그의 모든 Action 정의가 등록되어 있어야 한다. 보호된 조직 관리 owner 역할을 구성하는 Action이 하나라도 없으면 본인 조직 생성은 원자적으로 거절된다. 신원 반영과 조직 생성은 별개이므로 로컬 사용자만 존재하고 조직은 없는 상태가 정상적으로 남을 수 있다.
- 설정된 관리 앱에 네 온보딩 Action(`user.materialize`, `user.read`, `organization.list`, `organization.create`)과 제공할 조직 관리 Action을 `grant-management.` 접두사로 명시적으로 등록해야 한다. 앱 등록 자체가 Action 부여를 대신하지 않는다.
- user/application policy를 담당하는 관리 앱은 active/ready 및 first-party/auto-approve 상태여야 하고 그 앱 조직도 활성 상태여야 한다. 앱에 필요한 management Action과 사용자 consent는 기존처럼 검증한다.
- HTTP capability service identity는 이 관리 앱과 별도로 `GRANT_HTTP_CAPABILITY_PEP_CLIENT_ID`로 고정하고 Provider별로 등록한다. PocketBase 인증 설정의 application ID는 기존 managed user app을 계속 가리킨다. PocketBase 모드는 client ID 없이 ready인 기존 계약을 유지하며 Rauthy identity-binding을 호출하지 않는다. Rauthy로 전환할 경우 검증된 client ID와 token client의 일치가 필요하다.

기존 first-party/auto-approve 앱의 Action 변경은 일반 Action 수정 API에서 차단된다. 따라서 기존 앱에 새 Action을 추가할 때는 private Grant 제어 경로와 운영자 인증을 확보하고, 현재 trust·Action 집합·ETag를 저장한 뒤 작업한다. master 권한의 trust API로 해당 관리 앱만 일시적으로 untrusted로 바꾸고, 앱 조직의 관리 권한을 가진 Bearer로 현재 집합에 새 Action을 병합하여 조건부 교체한다. master key만으로 조직 앱 Action API를 호출하는 것은 아니다. 교체 결과를 읽어 확인한 후 원래의 trusted 상태를 복원한다. 이 구간에서는 공개 관리 경로가 거절될 수 있으므로 전환 시간을 계획한다. 기존 관계없는 Action을 제거하지 않고, 실패나 결과 불명 시 저장 상태를 확인하여 복구하며 무조건 재시도하지 않는다.

새 조직의 보호된 owner Binding은 자기 조직의 관리 권한만 제공한다. 기존 조직에는 자동으로 해당 Binding을 추가하지 않으며, 기존에 운영자가 부여한 역할은 별도로 검토한다. 새 조직을 만들었다는 이유로 플랫폼 운영 API나 다른 조직의 API를 호출할 수는 없다.

현재 deploy 저장소의 bootstrap은 빈 DB용이며 카탈로그/Action 배열을 입력에서 읽는다. 51·66·69 같은 개수를 하드코딩하지 않지만, 과거 파일을 그대로 입력하면 신규 Action과 경로가 누락된다. 기존 IED에 bootstrap을 다시 실행하지 말고 검토한 신규 정의와 카탈로그로 명시적인 갱신 절차를 사용한다. 이 문서 작성 과정에서는 deploy 저장소, 라이브 DB, 활성 세대, 이미지 pin을 변경하지 않았다.
