# 현재 Grant API 안내서

> **2026-10-05 배포 변경:** 현재 경로는 본문 없는 `POST /api/v1/me/ensure`이며, `/me/materialize`는 404를 반환한다. `GET /me`는 조회만 하고, ensure 이전에는 `404 LOCAL_USER_NOT_PROVISIONED`를 반환한다. Ensure는 내부 신원만 준비하며 조직 가입이나 권한을 만들지 않는다. 관리 catalog는 generation 6이다. 아래 표와 runtime 파일은 10월 4일의 관측 기록으로 보존한다. 현재 계약과 실제 호출 결과는 10월 5일 검증 기록을 참고한다.

기준일은 **2026-10-04**이다. 이 안내서는 런타임 담당자가 저장한 live 증거와 **배포 이미지의 커밋 `16020fe78bc4a1908f8cc105f0a0c728f5bbce4e`**의 라우트·핸들러·서비스를 기준으로 작성했다. 현재 작업 공간의 HEAD `3d9d4018bfc044421caffba4001901052c1569a3`는 배포 버전과 다르므로 현행 API의 기준으로 사용하지 않는다. 이 문서 작성자는 클러스터를 실행하거나 호출하지 않았다.

[Live management catalog](runtime/catalog-management.json)의 `.catalog.capabilities`에는 Provider **`grant-management`, generation 5의 활성 계약 69개**가 있다. 관측 시각은 20:37:03 KST이다. [이미지와 커밋 비교](runtime/grant-commit-comparison.json)는 20:43:36 KST에 배포 커밋을 확인했다. 관리 계약 69개는 배포 커밋의 HTTP fixture에 모두 포함된다. 배포 HTTP 경계는 총 **84개 method/path**이며 나머지 15개는 health, PDP, 운영용 경로다. 별도 private runtime 경로 3개는 이 84개에 포함하지 않는다.

**Provider 액션 관리자 GET, 활성 catalog 관리자 GET, 조직별 Provider 후보 액션 GET은 현재 활성 69개 계약에 모두 포함된다.** 런타임 담당자는 관리자 GET 두 개를 voter에 master 인증으로 호출하여 성공을 확인했다. [과거 등록 문서](../../registration-api-management.md)의 “IED 미배포” 문장은 당시 작업 상태이며 현재 live 제공 상태를 설명하지 않는다. 조직 후보 GET의 catalog 포함과 관리자 GET의 실제 성공을 구분하며, 모든 경로의 모든 사용자·변경 요청이 실측되었다고 확대하지 않는다. Catalog에 활성 계약이 있어도 개별 사용자는 권한·앱·소속·동의 조건을 통과해야 한다.

## 1. 왜 Grant가 필요한가

로그인은 “누구인가”를 확인한다. Grant는 “그 사람이 어느 조직의 어떤 리소스에서 어떤 작업을 어떤 앱으로 할 수 있는가”를 확인한다. 로그인만 검사하면 다른 조직의 데이터까지 열리거나 사용자가 동의하지 않은 앱이 업무 권한을 이용할 수 있다.

Provider는 서비스를, action은 작업을, role은 작업의 묶음을 뜻한다. Binding은 그 역할을 특정 사용자·그룹·조직과 정확한 리소스에 연결한다. 예를 들어 `patch-data.plant.read`를 역할에 넣는 것과 `(patch-data, plant, <PLANT_ID>)`에 그 역할을 적용하는 것은 서로 다른 설정이다. `resourceId:"*"`도 전체 리소스를 뜻하지 않는다. 앱의 액션 선택과 사용자 동의 역시 리소스 binding을 대신하지 않는다.

PDP인 Grant는 권한을 판단하고, PEP인 Exposure는 판단에 맞춰 실제 요청을 전달하거나 차단한다. 현재 management 계약은 `request.authorization:"forward-user"`를 사용한다. 따라서 PEP가 허용한 뒤에도 Grant 관리 핸들러가 사용자·조직·관리 권한을 확인한다. Grant가 업무 Provider의 origin이나 service token을 catalog에 저장하는 구조는 아니다.

## 2. 인증, 노드 역할과 공통 사용법

모든 예시의 `<...>`는 자리표시자다. 실제 사용자 token, master key, service credential을 문서에 적지 않는다. `/api/v1/orgs/{organizationId}` 예시에서는 `{organizationId}`를 `<ORG_ID>`로 바꾼다. 표의 요청 번호는 아래 상세 절의 공통 DTO 예시를 가리킨다. 응답 예시는 구조 설명이며 운영 환경의 실측 응답 원문이 아니다.

| 표기 | 누가 어떤 조건으로 호출하는가 |
|---|---|
| B | `Authorization: Bearer <USER_BEARER>`로 검증된 외부 신원을 사용한다. |
| U | B와 `X-Organization-ID: <ORG_ID>`를 사용한다. 로컬 사용자·소속이 활성이고 선택 조직과 path 조직이 일치해야 한다. 각 기능의 조직 관리 action도 검사한다. |
| P | 플랫폼 관리 경로다. 직접 voter 호출에는 `X-Grant-Master-Key: <MASTER_KEY>`를 사용할 수 있다. 키를 보내지 않은 경우 서버에 설정된 management 조직의 B+조직 문맥과 해당 `grant-management.*` platform 액션의 앱/RBAC/동의를 검사한다. 조직 관리자가 임의 조직 role만으로 플랫폼 운영자가 되지는 않는다. |
| M | 운영 전용 master 경로다. `X-Grant-Master-Key`가 필요하며 플랫폼 사용자 인증의 일반 대체 경로가 아니다. |
| A | PEP가 `X-Grant-App-Auth-Key: <APP_AUTH_KEY>`를 사용한다. 설정된 PEP application과 Provider 연결도 확인한다. |
| A+B | 판정용 서비스 키와 실제 최종 사용자의 Bearer를 함께 사용한다. 일반 업무 판정은 조직 문맥도 필요하다. 배포된 v2에는 엄격한 관리 onboarding 예외가 있으므로 모든 decide 요청에 조직 소속이 선행해야 한다고 단정하지 않는다. |

69개 관리 계약은 **voter 관리 API**로 연결된다. Learner는 전역 HTTP 검사에서 `/healthz`, `/readyz`, v2 catalog/decide만 제공하며 일반 조회까지 포함한 나머지 등록 경로는 503으로 막는다. 라우트 등록의 `voter=false`만 보고 learner가 조직 목록을 제공한다고 생각하면 안 된다.

조직 핸들러의 관리 action은 `(grant, organization, <ORG_ID>)`에서 검사한다. 프로필/owner 읽기는 `organization.profile.read`, 앱은 `organization.application.manage`, 그룹은 `organization.group.manage`, 구성원은 `organization.member.manage`, 역할·binding·협약·조직 변경은 `organization.permission.manage`다. Exposure의 `grant-management.*` catalog action은 이 내부 핸들러의 관리 action과 구분한다. 본인 consent에는 조직 앱 관리 action이 필요하지 않다.

```sh
curl '<MANAGEMENT_BASE_URL>/api/v1/orgs/<ORG_ID>/groups' \
  -H 'Authorization: Bearer <USER_BEARER>' \
  -H 'X-Organization-ID: <ORG_ID>'
```

`<MANAGEMENT_BASE_URL>`은 실제로 허용된 Exposure 관리 진입점 또는 직접 voter 주소로 정한다. URL이 같아 보여도 Exposure의 catalog 검사와 직접 Grant 핸들러 인증은 서로 다른 단계다. 네트워크 접근 여부는 [런타임 요약](runtime/runtime-summary.md)을 따른다.

공개 관리자 API도 사용자 Bearer로 호출한다. 예를 들어 Provider 액션 조회는 다음과 같다.

```sh
curl '<EXPOSURE_MANAGEMENT_URL>/api/v1/admin/providers/<PROVIDER_ID>/actions' \
  -H 'Authorization: Bearer <PLATFORM_USER_BEARER>' \
  -H 'X-Organization-ID: <CONFIGURED_MANAGEMENT_ORG_ID>'
```

이 live 계약의 `forwardHeaders`는 `x-organization-id`이고 인증 모드는 `forward-user`다. Exposure가 `X-Grant-Master-Key`를 전달하는 공개 호출 절차가 아니다. 먼저 PEP/PDP가 `grant-management.admin.provider.actions.read`와 `(grant-management, platform, grant)`를 검사하고, 직접 Grant 핸들러도 master 헤더가 없을 때 `AuthorizePlatformManagement`로 설정된 관리 조직·사용자·앱·해당 액션을 확인한다. **이 대체 인증은 표의 P로 표시한 공개 관리자 계약 14개 모두에 적용된다.** 정확히 배포 registry에서 `Platform:true`인 method/path를 대상으로 하며, `/api/v1/admin`라는 접두사만으로 모든 운영 경로에 적용하지 않는다. 다른 조직을 선택하면 403, 플랫폼 사용자 인증이 설정되지 않으면 503이다. 런타임 담당자가 수행한 voter 직접 master GET 성공은 이 공개 Bearer 경로의 모든 사용자에게 권한이 있음을 증명하지 않는다. `/ops/*`와 bootstrap, master v1 RBAC는 별도의 내부 운영 인증을 유지한다.

JSON body API는 `Content-Type: application/json` 또는 지원하는 `+json` media type을 요구한다. Body는 2 MiB 한계가 있다. Content-Type 오류는 415, 크기 초과는 413, 깨진 JSON은 400, 필수값 누락·null·중복·타입 오류는 보통 422다. PATCH는 생략한 필드를 유지하고 nullable 설명에 null을 보내면 지우며, immutable/unknown field를 거부한다. 빈 PATCH는 400이다. Body 검사가 인증보다 먼저 실행되는 경로도 있다.

GET과 `/api/v1/authz/*`의 선택적 `?consistency=strong`은 정확히 한 번만 사용할 수 있다. 다른 값/중복은 400이다. 관리 변경·동의·새 snapshot 조회는 strong 경계를 사용한다. 키 누락/오류는 401, 조직·권한·앱 조건 불일치는 403, 대상 부재는 보통 404, 참조·동시 변경 충돌은 흔히 409다. 모든 DB assertion 오류가 같은 4xx로 변환되는 것은 아니므로 underlying 500/503도 무시하지 않는다. 오류는 public API에서 보통 `{"error":"<설명>"}`다. 204와 backup 202는 body가 없다. 판정 API의 HTTP 200은 `allowed:true`를 보장하지 않는다.

### 동시 편집 보호가 필요한 이유

두 운영자가 오래된 액션 목록을 편집하면 한 사람의 변경이 사라질 수 있다. 배포 버전은 전체 action 교체와 동의·owner 이전에 ETag를 사용한다.

- Provider/role/app의 action GET은 Action 배열과 `ETag: "<OPAQUE_ETAG>"`를 반환한다. 전체 PUT에는 이 값을 그대로 `If-Match`로 보낸다. 필수 헤더가 없으면 428, 오래된 값이면 412다.
- 본인 consent GET은 grant/actions 객체와 ETag를 반환하며 활성 동의가 없으면 404다. 최초 PUT은 `If-None-Match: *`, 기존 동의 교체는 `If-Match: "<ETAG>"`를 사용한다. 두 조건 헤더를 함께 보내거나 별표 외 If-None-Match를 쓰면 400이다.
- Owner GET의 ETag로 owner PUT을 한다. 단일 role/app 액션 PUT/DELETE는 전체 목록을 보내지 않는 별도 작업이며 body와 If-Match를 요구하지 않는다.

```sh
curl -X PUT '<MANAGEMENT_BASE_URL>/api/v1/orgs/<ORG_ID>/apps/<APP_ID>/actions' \
  -H 'Authorization: Bearer <USER_BEARER>' -H 'X-Organization-ID: <ORG_ID>' \
  -H 'Content-Type: application/json' -H 'If-Match: "<OPAQUE_ETAG>"' \
  --data '{"actionIds":["<ACTION_ID>"]}'
```

근거는 배포 커밋의 `internal/api/http.go`, `routes_action_mutations.go`, `routes_ownership.go`, `internal/grant/store_action_mutations.go:113`이다. 정확한 소스 링크는 마지막 절에 모았다.

## 3. 현재 활성 관리 계약 69개의 전체 method/path

아래는 live catalog 순서이며 전부 `enabled:true`, `accessMode:authorized`다. 모든 항목은 배포 HTTP fixture에도 있다. 목적 열은 각 기능이 없을 때 생기는 사람·운영·보안 문제를 설명한다. “목록”은 body 없이 GET하며 배열을 반환하고 “상세”는 body 없이 GET하며 객체를 반환한다. 204는 빈 응답이다.

| 번호 | 메서드 | 전체 경로 | 직접 핸들러 인증 | 기능과 근본적인 필요 | 요청·응답 안내 |
|---|---|---|---|---|---|
| 01 | GET | `/api/v1/me` | B | 내 신원·조직을 확인해 외부 ID와 내부 사용자를 혼동하지 않는다. | 4.1 / 200 내 정보 |
| 02 | POST | `/api/v1/me/materialize` | B | 외부 신원을 내부 ID로 등록하되 가입 권한을 자동 부여하지 않는다. | 4.1 / body 없음, 200 |
| 03 | GET | `/api/v1/providers` | 없음 | 서비스 정의를 찾게 하여 action을 잘못된 Provider에 설정하지 않는다. | 4.2 / 200 배열 |
| 04 | GET | `/api/v1/orgs` | B | 내가 속한 조직을 찾아 다른 조직을 선택하는 실수를 줄인다. | 4.3 / 200 배열 |
| 05 | POST | `/api/v1/orgs` | B | 기존 조직의 관리자 없이 본인 소유 조직을 시작할 수 있게 한다. | 4.3 / 자기 생성 DTO, 201 |
| 06 | GET | `/api/v1/orgs/{organizationId}/owner` | U | 책임자를 확인하고 이전에 필요한 최신 상태를 읽는다. | 4.3 / 200 객체+ETag |
| 07 | PUT | `/api/v1/orgs/{organizationId}/owner` | U·현 owner | 책임자 변경 때 owner 부재·중복과 동시 이전을 막는다. | 4.3 / ownerUserId+If-Match, 200 |
| 08 | GET | `/api/v1/orgs/{organizationId}` | U | 선택 조직의 실제 프로필을 확인한다. | 4.3 / 200 객체 |
| 09 | PATCH | `/api/v1/orgs/{organizationId}` | U | 조직 이름이 현실과 달라져 관리 대상을 혼동하지 않게 한다. | 4.3 / name, 200 |
| 10 | POST | `/api/v1/admin/orgs` | P | 운영자가 명시한 사용자에게 새 조직 책임을 지정한다. | 4.3 / 관리자 생성 DTO, 201 |
| 11 | GET | `/api/v1/orgs/{organizationId}/apps` | U | 조직에 어떤 앱이 등록되었는지 확인한다. | 4.4 / 200 배열 |
| 12 | POST | `/api/v1/orgs/{organizationId}/apps` | U | 앱과 조직의 소유 관계를 정해 임의 client의 접근을 막는다. | 4.4 / 앱 생성 DTO, 201 |
| 13 | GET | `/api/v1/orgs/{organizationId}/apps/{applicationId}` | U | 앱의 준비·활성·동의 정책을 확인한다. | 4.4 / 200 객체 |
| 14 | PATCH | `/api/v1/orgs/{organizationId}/apps/{applicationId}` | U | 앱 설명을 수정하면서 보안·소유 필드를 함께 덮어쓰지 않는다. | 4.4 / name·description, 200 |
| 15 | PUT | `/api/v1/orgs/{organizationId}/apps/{applicationId}/active` | U | 문제 앱을 데이터 삭제 없이 정지한다. | 4.4 / active, 200 |
| 16 | DELETE | `/api/v1/orgs/{organizationId}/apps/{applicationId}` | U | 더 이상 사용하지 않는 앱과 관련 권한 연결을 정리한다. | 4.4 / body 없음, 204 |
| 17 | GET | `/api/v1/orgs/{organizationId}/apps/{applicationId}/actions` | U | 앱이 행사할 수 있는 작업의 상한을 확인한다. | 4.5 / 200 배열+ETag |
| 18 | PUT | `/api/v1/orgs/{organizationId}/apps/{applicationId}/actions` | U | 앱 작업 범위를 한 번에 교체하며 오래된 편집을 막는다. | 4.5 / actionIds+If-Match, 200 |
| 19 | PUT | `/api/v1/orgs/{organizationId}/apps/{applicationId}/actions/{actionId}` | U | 다른 선택을 덮어쓰지 않고 작업 하나를 추가한다. | 4.5 / body 없음, 204 |
| 20 | DELETE | `/api/v1/orgs/{organizationId}/apps/{applicationId}/actions/{actionId}` | U | 앱의 불필요한 작업과 그 동의 연결을 회수한다. | 4.5 / body 없음, 204 |
| 21 | GET | `/api/v1/orgs/{organizationId}/app-grants` | U | 조직의 사용자 동의 내역을 조사한다. | 4.6 / 200 배열 |
| 22 | DELETE | `/api/v1/orgs/{organizationId}/app-grants/{grantId}` | U | 관리자가 위험한 앱 동의를 조직 범위 안에서 철회한다. | 4.6 / body 없음, 204 |
| 23 | GET | `/api/v1/orgs/{organizationId}/apps/{applicationId}/consent` | U | 사용자가 자신의 허용 범위를 확인한다. | 4.6 / 200 객체+ETag, 없으면 404 |
| 24 | PUT | `/api/v1/orgs/{organizationId}/apps/{applicationId}/consent` | U | 로그인만으로 앱이 모든 사용자 권한을 이용하지 못하게 한다. | 4.6 / actionIds+조건 헤더, 200 |
| 25 | DELETE | `/api/v1/orgs/{organizationId}/apps/{applicationId}/consent` | U | 사용자가 앱 접근 동의를 스스로 철회한다. | 4.6 / body 없음, 204 |
| 26 | GET | `/api/v1/orgs/{organizationId}/groups` | U | 팀 단위 권한 주체를 찾는다. | 4.7 / 200 배열 |
| 27 | POST | `/api/v1/orgs/{organizationId}/groups` | U | 사람마다 권한을 복사하는 대신 팀을 만든다. | 4.7 / 그룹 DTO, 201 |
| 28 | GET | `/api/v1/orgs/{organizationId}/groups/{groupId}` | U | 권한 적용 대상 팀을 확인한다. | 4.7 / 200 객체 |
| 29 | PUT | `/api/v1/orgs/{organizationId}/groups/{groupId}` | U | 팀 이름·설명을 완전한 수정 요청으로 맞춘다. | 4.7 / name·description, 200 |
| 30 | PATCH | `/api/v1/orgs/{organizationId}/groups/{groupId}` | U | 한 필드 수정으로 다른 팀 설명을 잃지 않는다. | 4.7 / 일부 필드, 200 |
| 31 | DELETE | `/api/v1/orgs/{organizationId}/groups/{groupId}` | U | 쓰지 않는 팀을 참조가 없는 상태에서 정리한다. | 4.7 / body 없음, 204 |
| 32 | GET | `/api/v1/orgs/{organizationId}/groups/{groupId}/members` | U | 팀 권한의 적용 대상자를 확인한다. | 4.8 / 200 배열 |
| 33 | PUT | `/api/v1/orgs/{organizationId}/groups/{groupId}/members/{userId}` | U | 조직 안의 활성 구성원만 팀에 넣는다. | 4.8 / body 없음, 200 |
| 34 | DELETE | `/api/v1/orgs/{organizationId}/groups/{groupId}/members/{userId}` | U | 업무 이동 때 팀 기반 접근을 회수한다. | 4.8 / body 없음, 204 |
| 35 | PUT | `/api/v1/orgs/{organizationId}/members/{userId}` | U | 신원 등록과 별개로 조직 가입을 승인한다. | 4.8 / body 없음, 200 |
| 36 | GET | `/api/v1/orgs/{organizationId}/members` | U | 조직 소속과 상태를 조사한다. | 4.8 / 200 배열 |
| 37 | DELETE | `/api/v1/orgs/{organizationId}/members/{userId}` | U | 퇴사자의 팀·개인 권한·동의를 함께 회수한다. | 4.8 / body 없음, 204 |
| 38 | GET | `/api/v1/orgs/{organizationId}/agreements` | U | 조직 간 위임 근거를 찾는다. | 4.9 / 200 배열 |
| 39 | POST | `/api/v1/orgs/{organizationId}/agreements` | U·grantor | 누가 누구에게 위임했는지와 기간을 기록한다. | 4.9 / 협약 DTO, 201 |
| 40 | GET | `/api/v1/orgs/{organizationId}/agreements/{agreementId}` | U | 현재 계약 조건을 확인한다. | 4.9 / 200 객체 |
| 41 | PUT | `/api/v1/orgs/{organizationId}/agreements/{agreementId}` | U | 계약 내용을 교체하되 당사자를 바꾸지 않는다. | 4.9 / 전체 협약 DTO, 200 |
| 42 | PATCH | `/api/v1/orgs/{organizationId}/agreements/{agreementId}` | U | 제목·기간 등의 부분 수정으로 다른 조건을 보존한다. | 4.9 / 일부 필드, 200 |
| 43 | GET | `/api/v1/orgs/{organizationId}/roles` | U | 업무 역할을 재사용하고 중복 정의를 줄인다. | 4.10 / 200 배열 |
| 44 | POST | `/api/v1/orgs/{organizationId}/roles` | U | 작업을 업무 책임별로 묶는다. | 4.10 / 역할 DTO, 201 |
| 45 | PATCH | `/api/v1/orgs/{organizationId}/roles/{roleId}` | U | 이름·설명을 고치면서 역할의 소유·code를 보존한다. | 4.10 / name·description, 200 |
| 46 | DELETE | `/api/v1/orgs/{organizationId}/roles/{roleId}` | U | 사용하지 않는 역할을 dangling binding 없이 정리한다. | 4.10 / body 없음, 204 |
| 47 | PUT | `/api/v1/orgs/{organizationId}/roles/{roleId}/actions` | U | 업무 작업 묶음을 최신 snapshot 조건으로 교체한다. | 4.10 / actionIds+If-Match, 200 |
| 48 | GET | `/api/v1/orgs/{organizationId}/roles/{roleId}/actions` | U | 역할의 실제 작업 목록과 편집 ETag를 확인한다. | 4.10 / 200 배열+ETag |
| 49 | PUT | `/api/v1/orgs/{organizationId}/roles/{roleId}/actions/{actionId}` | U | 다른 역할 작업을 보존하며 하나를 추가한다. | 4.10 / body 없음, 204 |
| 50 | DELETE | `/api/v1/orgs/{organizationId}/roles/{roleId}/actions/{actionId}` | U | 불필요한 업무 작업을 역할에서 회수한다. | 4.10 / body 없음, 204 |
| 51 | GET | `/api/v1/orgs/{organizationId}/bindings` | U | 실제 리소스 권한의 근거를 확인한다. | 4.11 / 200 배열 |
| 52 | POST | `/api/v1/orgs/{organizationId}/bindings` | U | 역할을 정확한 사람·팀·조직과 리소스에 한정한다. | 4.11 / binding DTO, 201 |
| 53 | DELETE | `/api/v1/orgs/{organizationId}/bindings/{bindingId}` | U | 역할 정의를 남기고 해당 리소스 접근만 회수한다. | 4.11 / body 없음, 204 |
| 54 | POST | `/api/v1/orgs/{organizationId}/deactivate` | U | 조직 전체 접근을 데이터 삭제 없이 중단한다. | 4.3 / body 없음, 200 |
| 55 | POST | `/api/v1/orgs/{organizationId}/reactivate` | 복구 U | 비활성 조직의 정당한 관리자가 복구할 수 있게 한다. | 4.3 / body 없음, 200 |
| 56 | POST | `/api/v1/admin/providers` | P | 서비스 ID를 중복 생성하지 않고 등록한다. | 4.2 / Provider DTO, 201 |
| 57 | PATCH | `/api/v1/admin/providers/{providerId}` | P | 서비스 정보·활성을 부분 변경한다. | 4.2 / name·description·active, 200 |
| 58 | PUT | `/api/v1/admin/providers/{providerId}/actions` | P | 전체 작업 정의를 최신 상태 조건으로 교체한다. | 4.2 / actions+If-Match, 200 |
| 59 | GET | `/api/v1/admin/providers/{providerId}/actions` | P | 실제 정의와 변경용 ETag를 확인한다. | 4.2 / 200 배열+ETag, live 직접 성공 |
| 60 | POST | `/api/v1/admin/providers/{providerId}/actions` | P | 기존 작업을 덮어쓰지 않고 새 작업을 추가한다. | 4.2 / 단일 Action DTO, 201 |
| 61 | PATCH | `/api/v1/admin/providers/{providerId}/actions/{actionId}` | P | 의미·위험도·앱 부여 설정을 ID 보존하에 수정한다. | 4.2 / 일부 Action 필드, 200 |
| 62 | DELETE | `/api/v1/admin/providers/{providerId}/actions/{actionId}` | P | 폐기 작업과 권한 연결을 활성 catalog 참조 보호하에 제거한다. | 4.2 / body 없음, 204 |
| 63 | GET | `/api/v1/orgs/{organizationId}/providers/{providerId}/actions` | U | 앱에 선택 가능한 활성 서비스 작업만 보여 준다. | 4.2 / 200 후보 배열, live 계약 포함 |
| 64 | GET | `/api/v1/admin/providers/{providerId}/http-capability-active` | P | PEP 설정과 독립적으로 현재 활성 계약을 확인한다. | 4.12 / 200 객체, live 직접 성공 |
| 65 | POST | `/api/v1/admin/providers/{providerId}/http-capability-generations` | P | 검토할 불변 HTTP 계약을 실행 포인터와 분리해 저장한다. | 4.12 / stage DTO, 201 |
| 66 | POST | `/api/v1/admin/providers/{providerId}/http-capability-active` | P | 동시 운영자가 서로의 활성화를 덮어쓰지 않게 한다. | 4.12 / CAS DTO, 200 |
| 67 | PUT | `/api/v1/admin/orgs/{organizationId}/apps/{applicationId}/trust` | P | 일반 앱의 동의 우회를 막고 승인된 내부 앱만 신뢰한다. | 4.13 / trusted, 200 |
| 68 | PUT | `/api/v1/admin/orgs/{organizationId}/apps/{applicationId}/identity-binding` | P | IdP client의 실제 설정을 대조해 앱 신원을 연결한다. | 4.13 / clientId, 200 |
| 69 | PUT | `/api/v1/admin/users/{userId}/active` | P | 계정 침해·퇴사 때 남은 binding보다 먼저 공통 접근을 정지한다. | 4.13 / active, 200 |

Catalog action ID·resource extractor·body/query/header 한계의 원문은 [live catalog](runtime/catalog-management.json)에 있다. 이 표의 P는 Provider `grant-management`, resource `(platform, grant)`를 사용한다. U의 관리 catalog는 path 조직을 `(organization, <ORG_ID>)`에 결합한다. 사용자·조직 목록 등 서비스 계약은 `(service, grant)`를 사용한다. 일부 onboarding은 엄격한 전용 app admission 예외로 처리하며 일반 서비스 binding이나 조직 소속을 자동 생성하는 의미가 아니다.

## 4. 관리 API의 요청·응답·거부와 활용

### 4.1 내 신원 확인과 등록: 01–02

내 정보 GET은 body가 없고 B로 호출한다. 200 응답의 주요 필드는 `provider,externalUserId,user,organizations,activeOrganizationId`다. PocketBase는 `issuer,collectionId`를 포함할 수 있다. 조직 헤더를 보냈다면 자신의 활성 조직이어야 하며 아니면 403이다. 비활성 로컬 사용자도 거부된다. 기본 consistency에 따라 신원 동기화가 일어날 수 있으므로 GET을 항상 무변경이라고 해석하지 않는다.

Materialize POST는 body 없이 자신의 검증된 identity를 로컬 ID로 등록한다. 조직을 선택한 호출의 응답 예시는 `{"userId":"<USER_ID>","organizationId":"<ORG_ID>","membershipStatus":"none"}`다. 배포 버전에는 조직 미선택 onboarding도 있으므로 신규 사용자에게 membership을 먼저 요구하는 순환 절차를 만들지 않는다. 이 기능은 owner나 membership을 자동 부여하지 않는다. 신원은 PocketBase의 canonical issuer/collection/record namespace로 구분하며 이메일이 같다고 합치지 않는다. 잘못된 token·비활성 신원·허용되지 않은 onboarding 앱은 인증/권한 오류로 거부된다.

### 4.2 Provider와 action 정의: 03, 56–63

Provider GET은 직접 Grant에서는 인증 없이 전체 Provider 배열을 반환한다. 비활성 항목도 관리 목적으로 포함되며 사용자별 업무 허용 목록은 아니다. Exposure 경로에는 별도의 활성 관리 catalog 권한 검사가 있다.

Provider POST body는 `{"id":"<PROVIDER_ID>","name":"<NAME>","description":null,"active":true}`다. **배포 버전은 생성 전용이며 기존 ID는 409다.** PATCH `{"name":"새 서비스 이름","active":false}`로 수정한다. ID를 바꾸는 필드는 없고 비활성화된 Provider는 권한 평가의 활성 조건을 통과하지 못한다. 빈 필수 문자열·빈 PATCH·잘못된 immutable 필드는 거부된다.

Action 생성·전체 교체에 사용하는 단일 정의 예시는 다음과 같다.

```json
{"id":"patch-data.plant.read","code":"plant.read","name":"발전소 읽기","description":"허용된 발전소를 조회합니다.","riskLevel":"high","visibilityGranting":false,"grantableToApplications":true,"consentLabel":"발전소 조회","consentDescription":"앱의 발전소 조회를 허용합니다."}
```

Action POST는 이 객체 하나를 보내 201 Action 객체를 받는다. 전체 PUT은 `{"actions":[<ACTION 객체>]}` 구조와 최신 관리자 GET의 If-Match를 보내 200 Action 배열을 받는다. 꺾쇠 객체 표기는 대입 설명이며 그대로 실행하는 JSON이 아니다. 최대 999개이고 `[]`는 전체 제거다. `grant` 외 Provider의 ID는 필요하면 `<provider>.<code>`로 정규화한다. risk는 low/medium/high/critical이며 자동 권한 점수가 아니다. `visibilityGranting:true`는 지원하지 않는다.

Action PATCH 예시는 `{"name":"운영 정보 조회","grantableToApplications":false}`다. name/description/riskLevel/visibilityGranting/grantableToApplications/consentLabel/consentDescription만 변경할 수 있고 id/code/provider는 immutable이다. 앱 부여를 false로 바꾸면 기존 앱 선택·동의 연결을 제거하지만 역할 연결은 보존한다. DELETE는 body 없이 해당 action과 role/app/consent 연결을 제거하며 204다. 활성 catalog가 해당 action을 참조하면 409로 막는다. 제거하려면 참조를 없앤 catalog를 먼저 활성화해야 한다.

관리자 GET은 비활성 Provider도 읽고 Action 배열과 ETag를 반환한다. Provider 부재는 404, action 부재는 빈 배열이다. 조직 후보 GET은 U와 앱 관리 action이 필요하며 활성 Provider의 `grantableToApplications:true`만 반환한다. 비활성/없는 Provider는 404다. 후보 노출은 사용자 리소스 권한을 뜻하지 않는다. 두 관리자 GET의 live 성공과 조직 후보 계약의 live 포함은 문서 첫머리의 증거를 따른다.

### 4.3 조직 생성·프로필·owner·정지/복구: 04–10, 54–55

조직 목록 GET은 B로 자신의 활성 membership이 있는 조직 배열을 반환한다. 선택 조직 헤더 없이 목록을 찾을 수 있다. 프로필 GET은 U와 profile.read가 필요하며 `{"id":"<ORG_ID>","name":"<NAME>"}` 같은 Organization 객체를 받는다. 선택적 partyId/businessNo/inactivatedAt은 있을 때 포함한다. PATCH `{"name":"새 조직 이름"}`은 permission.manage를 요구하며 다른 조직·소유자·사업자 ID를 변경하지 않는다.

자기 조직 POST는 `{"id":"<NEW_ORG_ID>","name":"새 조직"}`만 받는다. 검증된 본인이 owner가 되고 bootstrap 관리 binding과 해당 조직의 management-owner binding을 같은 작업에서 마련한다. 관리자 조직 POST는 `{"id":"<NEW_ORG_ID>","name":"새 조직","ownerUserId":"<EXISTING_ACTIVE_USER_ID>"}`를 받는다. 명시한 활성 로컬 사용자가 owner가 되며 호출자가 암묵적으로 owner가 되지 않는다. 각각 201 Organization이다. 중복 조직·invalid owner·참조 전제 변경은 400/409 등으로 거부된다. 플랫폼 액션은 자기 조직 생성으로 얻는 조직 관리 권한과 다르다.

Owner GET은 U/profile.read로 `{"organizationId":"<ORG_ID>","ownerUserId":"<USER_ID>"}`와 ETag를 받는다. Owner가 정확히 한 명인 상태를 요구하며 아니면 409다. PUT `{"ownerUserId":"<NEW_OWNER_ID>"}`와 If-Match는 현 owner 본인과 permission.manage를 요구한다. 새 owner도 활성 사용자이자 같은 활성 조직의 활성 구성원이어야 한다. 현 owner와 같으면 400, 현 owner가 아니면 403, 헤더 없음은 428, 상태 변경은 412다. 이전 owner는 member가 되고 onboarding owner 관리 binding은 기존 상태를 보존하여 옮긴다. 무관한 개인 업무 binding을 모두 새 owner에게 복사하는 기능은 아니다.

Deactivate POST는 U/permission.manage, body 없음으로 `inactivatedAt`이 있는 200 Organization을 받는다. Reactivate POST는 body 없이 비활성 조직을 복구한 객체를 받는다. 복구에서는 조직이 inactive인 경우만 예외로 열고 신원·활성 membership·동일 조직 관리 권한을 검사한다. PocketBase 조직 헤더는 path와 일치해야 한다. 일반 inactive gate를 우회하여 다른 조직을 복구하는 기능이 아니다. 권한 없으면 403이며 소속과 데이터는 삭제하지 않는다.

### 4.4 앱 등록·수정·활성·삭제: 11–16

U/application.manage로 앱 목록 배열 또는 상세 객체를 읽는다. 다른 조직/없는 앱은 404다. 생성 body는 다음과 같다.

```json
{"id":"<APP_ID>","slug":"<APP_SLUG>","name":"운영 포털","description":"운영 담당자용 앱입니다.","applicationType":"confidential","appType":"web","active":true,"redirectUri":"https://app.example.invalid/callback"}
```

201 응답은 평탄한 Application 객체이며 `id,organizationId,slug,name,applicationType,appType,firstParty,consentPolicy,active,provisioningStatus,createdByUserId`와 선택적 redirectUri 등이 있다. 일반 앱은 firstParty:false/explicit다. PocketBase는 ready이며 Rauthy client ID가 없다. Rauthy는 unprovisioned로 만들고 배포 버전에서는 client를 임의 자동 생성·예약한 것으로 안내하지 않는다. Client 연결은 4.13을 사용한다.

ApplicationType은 confidential/public, appType은 web/mobile/server/internal이다. Redirect는 최대 2048 byte의 absolute HTTPS 또는 explicit-port loopback-IP HTTP이고 query/fragment/userinfo/wildcard를 거부한다. 생략은 빈 manifest다. 앱 목록에는 redirectUri가 항상 포함되는 것이 아니다.

PATCH `{"name":"새 포털 이름","description":null}`은 메타데이터만 바꾼다. PUT active `{"active":false}`는 앱을 정지하고 활성 동의를 revoke한다. true로 복구해도 과거 동의를 자동 복구하지 않는다. First-party 앱의 조직 lifecycle 변경은 403이다. DELETE는 일반 앱의 grant/actions/redirect 등의 연결을 정리하지만 플랫폼 관리 앱은 403, deployment enforcer가 참조하면 409, 보안 mutation lease가 충돌해도 409다. 없는 앱은 404다. 클라이언트 secret 발급·IdP client 삭제 API라고 해석하지 않는다.

### 4.5 앱 작업 범위: 17–20

U/application.manage로 Action 배열+ETag를 읽고 전체 PUT `{"actionIds":["<ACTION_ID>"]}`에 If-Match를 보낸다. 200 선택 배열이며 최대 999개, `[]`는 전체 해제다. Provider 비활성/미등록·앱 부여 불가 action은 거부하고 trusted 앱의 액션 변경은 403, 보안 변경 중은 409다.

한 action만 추가할 때는 body 없이 `/actions/<ACTION_ID>`에 PUT, 제거할 때는 DELETE하여 204를 받는다. 중복 추가는 존재 상태를 유지하며, 제거는 그 action의 사용자 동의 연결도 제거한다. 한 항목 변경은 다른 액션을 덮어쓰지 않는다. 이 앱 범위는 사용자 RBAC·리소스 binding을 만들지 않는다. 다른 앱/조직 scope는 404, action·최대 개수·동시 참조 전제는 오류로 거부된다.

### 4.6 사용자 동의와 조직 관리자의 동의 철회: 21–25

조직 app-grants GET과 `/app-grants/<GRANT_ID>` DELETE는 U/application.manage다. GET은 조직의 ApplicationGrant 배열로 userId/applicationId/status/consentedAt와 철회 정보 등을 반환한다. DELETE는 같은 조직에 속한 해당 grant를 revoke하고 204다. 없는/다른 조직 grant는 404다. 이것은 다른 사람의 동의를 새로 작성하는 기능이 아니다.

본인의 `/apps/<APP_ID>/consent` GET/PUT/DELETE는 U로 현재 사용자를 사용한다. GET은 `{"grant":<GRANT 객체>,"actions":[<ACTION 객체>]}`와 ETag를 반환하며 활성 동의 부재는 404다. 최초 PUT은 `If-None-Match: *`, 기존 교체는 If-Match와 `{"actionIds":["<ACTION_ID>"]}`를 보내 같은 구조의 200 응답을 받는다. 1–999개이며 앱이 선택한 액션 안에서만 허용한다. inactive 앱은 403, auto_approve 앱에 explicit 동의를 쓰면 400, stale 조건은 412, 조건 없음은 428이다. DELETE는 204로 본인의 활성 동의를 revoke한다.

로그인과 동의는 다르다. 동의가 남아 있어도 사용자 binding을 잃으면 판정은 불허한다. Trusted 앱도 앱 action과 사용자 RBAC가 필요하다. 관리자가 목록을 볼 권한과 사용자가 자기 동의를 쓸 권한은 분리된다.

### 4.7 그룹: 26–31

U/group.manage로 목록 배열·상세 객체를 확인한다. POST body `{"id":"<GROUP_ID>","name":"운영팀","description":"현장 운영 담당자입니다."}` → 201 Group이며 `organizationId`는 path에서 정한다. PUT은 `{"name":"현장 운영팀","description":null}`로 이름과 설명을 바꾸고 PATCH `{"name":"현장 운영팀"}`는 생략 필드를 유지한다. 응답은 수정된 200 Group이다. name은 빈 값/null을 허용하지 않으며 unknown/immutable 필드는 거부된다.

DELETE는 body 없이 204이고 그룹 membership도 제거한다. Group role binding이 하나라도 남아 있으면 disabled 참조도 포함하여 409로 막는다. 먼저 binding을 정리해야 한다. 다른 조직/없는 그룹은 404다. 팀 단위 관리는 사람마다 권한 복사본을 남기는 문제를 줄이지만 그룹을 만들기만 해서는 권한이 생기지 않는다.

### 4.8 조직 소속과 그룹 구성원: 32–37

그룹 구성원 경로는 U/group.manage다. GET은 `{"groupId":"<GROUP_ID>","organizationMembershipId":"<MEMBERSHIP_ID>","userId":"<USER_ID>"}` 항목의 배열이다. PUT `/members/<USER_ID>`는 body 없이 활성 사용자·활성 조직의 active membership을 확인하여 200 GroupMember를 반환한다. 조건 부재는 403이며 재추가는 중복 연결을 만들지 않는다. DELETE는 204로 그룹 연결만 제거하며 개인/조직 binding은 별도로 남을 수 있다.

조직 members 경로는 U/member.manage다. PUT `/members/<USER_ID>`는 body 없이 활성 로컬 사용자에 member/active 소속을 만들어 200 OrganizationMembership을 반환한다. GET은 membershipRole/status 등을 포함한 전체 소속 배열이다. Owner/admin 승격 API는 아니다.

DELETE 조직 member는 body 없이 204를 반환한다. **실제 배포 구현은 membership row를 물리 삭제하지 않고 member/inactive로 바꾸며**, 그 사용자의 group membership, 해당 조직 개인 role binding을 지우고 활성 앱 동의를 revoke한다. 재가입이 과거 개인 권한과 동의를 자동 복구하지 않게 하기 위해서다. Owner 제거는 409이며 먼저 owner 이전을 해야 한다. 없는 membership은 404다. 사용자의 다른 조직 membership이나 전역 identity는 삭제하지 않는다.

### 4.9 조직 간 협약: 38–42

U/permission.manage로 grantor 또는 grantee인 협약만 목록·상세 조회한다. 다른 조직 범위는 403, 부재는 404다. POST와 전체 PUT의 DTO 예시는 다음과 같다.

```json
{"id":"<AGREEMENT_ID>","agreementType":"운영 위임","grantorOrganizationId":"<GRANTOR_ORG_ID>","granteeOrganizationId":"<GRANTEE_ORG_ID>","status":"active","title":"발전소 운영 계약","referenceNo":"<REFERENCE_NO>","startsAt":"2026-10-01T00:00:00Z","endsAt":"2027-10-01T00:00:00Z"}
```

POST는 grantor 조직만 가능하며 201 Agreement다. PUT의 ID는 path와 같고 당사자 조직은 불변이다. PATCH `{"title":"변경 계약명"}`이나 `{"endsAt":"2027-12-31T00:00:00Z"}`는 일부 필드만 바꾸어 200 객체를 반환한다. 수임 조직은 상태/시작/종료 권한 조건을 바꿀 수 없으며 403이다.

배포 버전의 agreementType은 고정 enum이 아니라 **1–128자의 제어문자 없는 UTF-8 분류 문자열**이다. 이 분류는 표시 메타데이터이며 권한 규칙을 선택하지 않는다. 상태는 draft/active/expired/terminated다. 날짜는 RFC3339로 검사하여 고정 폭 UTC로 정규화하고 시작이 종료보다 늦으면 400이다. null은 경계 없음이다. 잘못된 날짜를 무기한으로 처리하지 않는다.

협약은 위임 근거이며 그 row만으로 권한이 생기지 않는다. 연결된 binding과 evaluator의 실제 협약 검사를 함께 사용한다. 일반적인 업무 관계나 같은 발전소 ID를 권한 상속으로 확대하지 않는다. 이 목록에 협약 DELETE는 없다.

### 4.10 역할과 역할 액션: 43–50

U/permission.manage로 role 목록은 같은 조직과 전역 역할을 함께 반환한다. POST `{"id":"<ROLE_ID>","code":"plant-reader","name":"발전소 조회 담당","description":null}` → 201 Role이다. PATCH `{"name":"현장 조회 담당","description":null}` → 200이며 조직·ID·code를 바꾸지 않는다. 조직 소유 role만 수정·삭제할 수 있고 전역/다른 조직 role은 해당 수정 범위에서 404다.

Role action GET은 Action 배열+ETag이며 읽을 수 있는 전역 role도 포함한다. 전체 PUT `{"actionIds":["patch-data.plant.read"]}`+If-Match는 최대 999개, 중복 정리, `[]` 해제를 지원한다. 단일 `/actions/<ACTION_ID>` PUT/DELETE는 body 없이 204다. 앱 부여 가능 여부는 role-action 등록을 대신 제한하는 조건이 아니다. 미등록 action·변경 전제는 오류이며 전체 교체의 stale ETag는 412/누락은 428이다.

Role DELETE는 body 없이 204지만 role binding이 남아 있으면 409다. 먼저 적용 대상을 회수해야 한다. Role 생성만으로 리소스 권한이 생기지 않으며 보존된 field-view 테이블이 있다고 현재 field mask 관리 API가 제공되는 것도 아니다.

### 4.11 리소스 binding: 51–53

U/permission.manage로 subjectOrganizationId가 선택 조직인 binding 배열을 읽는다. 생성 body는 다음과 같다.

```json
{"id":"<BINDING_ID>","subjectType":"user","subjectId":"<USER_ID>","subjectOrganizationId":"<ORG_ID>","providerId":"patch-data","resourceType":"plant","resourceId":"<PLANT_ID>","roleId":"<ROLE_ID>","agreementId":null,"state":"active"}
```

201 RoleBinding은 요청에 생성 사용자/시각 등을 추가한다. Subject는 user/group/organization, state는 active/disabled다. Organization subject의 ID는 subjectOrganizationId와 같아야 한다. 활성 Provider/조직, role scope, subject 참조를 확인하며 path와 subject 조직 불일치는 403이다. `grant/organization`의 다른 조직 리소스 binding에는 organization subject와 유효한 agreement가 필요하다. 업무 레코드가 실제 upstream에 있는지 Grant가 조회하는 기능은 아니다.

DELETE `/bindings/<BINDING_ID>`는 body 없이 204며 다른 조직/없는 binding은 404다. Role·사용자·업무 레코드는 남는다. Role을 모든 발전소에 자동 적용하는 wildcard·hierarchy·업무 relation 상속은 없다. 일부 assertion 실패의 status는 underlying 오류 변환을 따르므로 모든 참조 오류를 403/409로 고정하지 않는다.

### 4.12 HTTP catalog 등록·활성화·조회: 64–66

이 기능이 없으면 action의 의미와 실제 HTTP method/path가 어긋나고 작성 중인 계약이 실행되거나 운영자끼리 포인터를 덮어쓸 수 있다. P 인증으로 stage를 작성하고 PEP를 별도 운영 경로에 등록한 뒤 CAS로 활성화한다. 기본 설정만으로 자동 활성화되지 않는다.

Stage POST는 `schemaVersion:1,generation:<NEXT>,expectedLatestGeneration:<CURRENT_OR_NULL>,capabilities:[...],catalogDigest:"sha256:<HEX_64>"`와 선택적 responseResources를 받는다. Capability는 capabilityId/version/enabled/accessMode/method/pathTemplate/actionId/resource/query/request/response/contractDigest를 갖는다. 예를 들어 authorized GET `/api/collections/plants/records/{plantId}`는 plant.read와 pathSegment plantId extractor를 결합한다. Public_authn은 action/resource 없이 선언하며 decide하지 않는다. Extractor는 pathSegment/queryScalar/jsonPointer/constant만 지원한다.

201 응답은 `staged,providerId,generation,catalogDigest,totalCount,enabledCount,disabledCount`다. Generation은 1부터 연속 증가하고 immutable이다. 동일 bytes 재시도는 staged:false일 수 있으며 같은 번호에 다른 내용은 409다. Stage는 active pointer를 바꾸지 않는다. Digest는 구현의 canonical contract/catalog 알고리즘으로 계산하며 파일 전체 hash로 대체하지 않는다. 계약 형식·중복/겹침·digest는 422 등으로 거부하고 존재하지 않는 action은 400이다.

Activation POST body는 다음 구조다.

```json
{"expectedGeneration":null,"expectedCatalogDigest":null,"targetGeneration":1,"targetCatalogDigest":"sha256:<CATALOG_HEX_64>"}
```

최초만 expected 두 필드를 null로 쓰며 이후 현재 active 번호/digest를 함께 보낸다. 200은 activationReceiptId/activationSequence/fromGeneration/toGeneration/catalogDigest/activatedAt/activatedBy를 포함한다. 유효한 stage와 active/ready 앱에 연결된 active·미만료 PEP가 필요하며 stale CAS/action 참조/PEP 부재는 409다. 이전 stage generation으로 rollback해도 같은 CAS를 사용한다.

관리 GET은 P로 body 없이 활성 catalog와 generation/digest/activation 정보·count를 조회한다. **현재 management catalog는 generation 5다.** PEP lease 필드는 없고 Provider 부재 또는 active catalog 부재는 404다. 이 GET은 실제 voter master 호출 성공이 확인되었다. Snapshot의 TTL은 당시 관측값이므로 저장된 catalog의 expiresAt을 오늘의 새 receipt처럼 재사용하지 않는다.

### 4.13 Trust·IdP identity 연결·사용자 정지: 67–69

Trust PUT body `{"trusted":true}` → 200 Application에서 firstParty:true/auto_approve를 확인한다. false는 해제한다. 승격은 기존 활성 explicit grant를 revoke한다. 일반 조직 관리자가 trusted 앱 action을 바꾸어 동의 우회를 넓히지 못하게 한다. Trust가 사용자 RBAC·앱 액션을 대신하지 않으며 앱 scope 부재는 404, security lease 충돌은 409다.

Identity-binding PUT body `{"clientId":"<IDP_CLIENT_ID>"}` → 200 Application이다. Rauthy client 실제 ID/name/enabled/confidential/redirect/flows/challenges를 로컬 앱 계약과 대조한 뒤 연결하고 ready로 만든다. Ready 앱의 다른 client로 재연결은 409, inactive 앱은 409, client read 설정 부재/통신 실패는 503이다. **현재 PocketBase 모드에서는 Rauthy identity-binding hook의 해당 기능이 활성 사용 경로가 아니며 409로 거부된다.** Catalog에 경로가 있어도 인증 공급자 전제가 맞아야 한다. Secret을 발급하거나 IdP client를 자동 생성하는 기능은 아니다.

사용자 active PUT `{"active":false}` → 200 User의 `id,displayName,active:false` 등을 확인한다. true는 다시 활성화한다. 부재는 404다. Grant local 계정을 정지하며 외부 IdP token을 직접 취소하지 않는다. 이후 strong/fenced 판정의 active 조건이 접근을 막는다. 남아 있는 짧은 receipt의 유효기간은 별도 계약에 따른다.

## 5. 배포 커밋의 추가 HTTP 경로 15개

69개 관리 계약과 아래 15개를 합하면 배포 fixture의 **84개**다. [배포 경로 TSV](grant-deployed-routes.tsv)는 integrator가 결합할 수 있도록 method/path/commit/source를 저장했다. 이 표는 active management catalog의 공개 계약으로 추가하지 않는다.

| 메서드 | 전체 경로 | 인증·노드 | 필요와 요청·응답·제한 |
|---|---|---|---|
| GET | `/healthz` | 없음·L+V | 프로세스 생존을 확인해 불필요한 재시작을 줄인다. Body 없음, 200 `{"status":"ok"}`이며 DB 정상까지 보장하지 않는다. |
| GET | `/readyz` | 없음·L+V | 준비되지 않은 노드로 요청을 보내지 않는다. Body 없음, 200/503 health 객체의 status/readReady/writeReady/runtimeMode/defaultReadConsistency를 읽는다. Learner writeReady는 false다. |
| POST | `/api/v1/authz/check` | M·V | 내부 운영자가 지정 사용자 한 명의 exact RBAC를 확인한다. 5.1 check body, 200 allowed/reason. 앱 문맥은 없다. |
| POST | `/api/v1/authz/batch-check` | M·V | 여러 판정의 왕복을 줄인다. `{"checks":[<CHECK>]}`, 200 results 배열. 최대 1000개. |
| POST | `/api/v1/authz/filter` | M·V | 받은 후보 중 허용 ID만 추린다. 5.1 filter body, 200 resource_ids 배열. 최대 1000개. |
| POST | `/api/v1/application-authz/check` | A+B+조직·V | 사용자 권한뿐 아니라 앱 범위·동의를 확인한다. 5.1 앱 check, 200 allowed/reason. |
| POST | `/api/v1/application-authz/batch-check` | A+B+조직·V | 여러 앱 판정을 묶어 호출한다. checks 배열, 200 results. 최대 1000개. |
| POST | `/api/v1/application-authz/filter` | A+B+조직·V | 앱이 사용할 수 있는 후보 ID만 추린다. action/resources, 200 resource_ids. 최대 1000개. |
| GET | `/api/v2/http-capabilities/catalog/{providerId}` | A·L+V | PEP가 PDP와 같은 활성 HTTP 계약을 쓰게 한다. Body 없음, 200 catalog와 30초 lease. PEP 설정 부재 503, 미활성/만료 등 403. |
| POST | `/api/v2/http-capabilities/decide` | A+B·L+V | 허용 판단을 원래 HTTP 요청 증거에 결합해 재사용을 막는다. 5.2 evidence body, 200 allow/echo와 5초 receipt. |
| POST | `/api/v1/admin/bootstrap/owner` | M+B+조직·V | 최초 관리 사용자가 없을 때 검증 신원과 owner를 연결한다. Body 없음, 201 userId/organizationId. 앱은 별도 등록한다. |
| POST | `/ops/backups` | M·V | 장애 후 권한 상태 복구 자료를 만든다. Body 없음, snapshot 생성 이후 202 빈 body. Job ID/poll API는 없다. |
| GET | `/ops/backups` | M·V | 실제 보관된 논리 snapshot을 확인한다. Body 없음, 200 `{"scope":"logical-snapshots","items":[...]}`. 목록은 restore 검증이 아니다. |
| PUT | `/ops/providers/{providerId}/enforcers/{pepClientId}` | M·V | 증거를 제출하는 PEP의 Provider·앱 신원을 제한한다. applicationId/lifecycle/validUntil, 200 PEP 객체. |
| POST | `/ops/migrations/patch-data-authority` | M·V | 기존 소속·발전소 권한을 partial state 없이 canonical snapshot으로 옮긴다. 5.3 manifest, 200 import receipt. |

### 5.1 사용자 RBAC와 앱 판정의 공통 사용법

Check의 예시는 다음과 같다. Subject는 user만 지원하고 orgId/org_id 중 정확히 하나를 보내며 resource.view는 보내지 않는다.

```json
{"subject":{"type":"user","id":"<USER_ID>","orgId":"<ORG_ID>"},"action":"patch-data.plant.read","resource":{"provider":"patch-data","type":"plant","id":"<PLANT_ID>"}}
```

앱 check는 subject를 생략하여 `{"action":"patch-data.plant.read","resource":{"provider":"patch-data","type":"plant","id":"<PLANT_ID>"}}`로 보낸다. 사용자와 앱은 검증 token/client 또는 서버 설정으로 결정하며 임의 applicationId를 body에 받지 않는다. Batch는 위 객체를 checks 배열로 감싼다. Filter는 subject(앱 filter는 생략), action, resources 배열을 보내고 `{"resource_ids":["<PLANT_ID>"]}` 또는 빈 배열을 받는다. 전체 DB를 검색하는 API가 아니며 입력 후보만 검사한다.

정상 check 응답은 `{"allowed":true,"reason":"role_binding"}` 또는 false/no_matching_binding이다. 앱 판정의 허용 reason은 trusted_application/explicit_consent, RBAC는 있으나 앱 조건이 부족하면 application_consent_required가 될 수 있다. 이 reason만으로 부족한 조건이 반드시 동의 row 하나라고 해석하지 않는다. HTTP 200 false도 불허다. Invalid subject/view/배열 초과는 400 등으로 거부된다. Empty 배열은 허용되며 결과 순서를 유지한다. 일반 v1 batch와 아래 원자 HTTP batch의 동일 fenced snapshot 보장을 혼동하지 않는다.

### 5.2 Generic HTTP capability 판정

Catalog GET은 A와 설정된 active/ready PEP app, active·미만료 Provider PEP 연결을 확인한다. Body 없이 active generation만 받으며 pepClientId/issuedAt/expiresAt이 포함된다. 저장한 live snapshot은 조사 증거이지 TTL이 연장된 사용 credential이 아니다.

Decide는 catalogGeneration/catalogDigest/capabilityId/version/contractDigest, method/canonicalPath/canonicalQuery와 그 digest, forwardedHeaderDigest/bodyDigest/bodyLength, providerCredentialDigest/subject/audience, resourceEvidence/type/id, requestBinding을 받는다. 다음은 주요 구조이며 **생략 필드는 실제 요청에서 채워야 한다**.

```json
{"providerId":"<PROVIDER_ID>","pepClientId":"<PEP_CLIENT_ID>","catalogGeneration":5,"catalogDigest":"sha256:<HEX_64>","capabilityId":"<CAPABILITY_ID>","capabilityVersion":1,"contractDigest":"sha256:<HEX_64>","method":"GET","canonicalPath":"<CANONICAL_PATH>","canonicalQuery":"","canonicalQueryDigest":"sha256:<HEX_64>","forwardedHeaderDigest":"sha256:<HEX_64>","bodyDigest":"sha256:<HEX_64>","bodyLength":0,"providerCredentialDigest":"sha256:<HEX_64>","providerCredential":{"subject":"<VERIFIED_SUBJECT>","audience":"<VERIFIED_AUDIENCE>"},"resourceEvidence":{"kind":"constant","canonicalValue":"grant"},"resourceType":"service","resourceId":"grant","requestBinding":"sha256:<HEX_64>"}
```

위 예시는 요청 구조를 설명한다. Digest 자리표시자는 PEP가 실제 전달할 요청에서 구현의 canonical·domain 분리 알고리즘으로 계산한 값으로 바꿔야 한다. PocketBase subject는 `pocketbase:<CANONICAL_ISSUER>/<COLLECTION_ID>/<RECORD_ID>`이며 audience는 검증된 설정 앱 ID다. 호출자가 임의로 적은 JWT claim을 신원의 근거로 사용하지 않는다.

200 응답은 allowed/reason과 요청·catalog·capability·principal의 결합 정보, issuedAt/expiresAt을 반환한다. PEP는 5초 receipt가 허용이고 모든 echo 값이 원래 요청과 일치할 때만 전달한다. 오래된 catalog나 계약·PEP·credential·resource 증거 불일치는 403, quorum/snapshot 실패는 503으로 거부된다. Grant는 PEP가 추출한 body·forwarded header·응답 원문을 독립적으로 재파싱하지 않고 digest에 결합된 descriptor를 검증한다.

같은 endpoint가 처리하는 추가 기능과 각 기능이 필요한 이유는 다음과 같다.

| 기능 | 왜 필요한가 | 사용·제한 |
|---|---|---|
| 경로·query·body·file 범위 | 허용받은 읽기 요청을 다른 변경·파일 요청으로 바꾸지 못하게 한다. | 선언된 method/path/query와 extractor만 허용한다. Opaque/multipart는 maxBytes/contentTypes/header 정책을 요구하며 PEP가 원본 bytes를 hash·전달한다. JSON resource 추출은 jsonPointer다. |
| upstream credential 모드 | 서버의 강한 service token을 임의 사용자 요청에 사용하지 못하게 한다. | forward-user가 기본이며 service-token은 authorized 계약만 허용한다. PEP가 allow 후 교체하고 Grant는 token을 catalog/응답에 저장하지 않는다. 현재 management69는 forward-user다. |
| 목록 recordScope | 목록을 받은 뒤 항목을 지우는 방식의 totals·pagination 누출을 막는다. | 허용 응답에 recordScope.ids를 포함하며 빈 ids는 정상 빈 scope다. 후보 최대1000, 허용 최대199, 초과는 truncate하지 않고 오류다. PEP가 ACL과 client filter를 AND로 결합한다. Constant 목록 ID는 collection 전체 grant가 아니다. |
| 원자 request-array batch | 루트 권한만으로 금지 child 쓰기를 섞거나 일부만 전달하는 것을 막는다. | ordered batchChildren/batchDigest를 추가한다. 같은 fence에서 root와 모든 child를 판정하며 최대20 operation/upsert이면40 check다. ID 있는 upsert는 create+update 둘 다 필요하다. 한 child 불허면 전체 불허다. |
| buffered response expand | 루트가 허용되어도 관련 레코드가 다른 사람의 데이터일 수 있다. | response graph와 ordered references, response digest/length를 결합해 재판정한 뒤 PEP가 buffer를 내보낸다. 최대 depth6/200 occurrence다. Relation edge는 권한 상속이 아니다. 실패하면 응답 bytes를 내보내지 않는다. |
| 응답 status·header·크기 제한 | upstream 오류·파일을 지원하면서 redirect/무제한 전송을 막는다. | Catalog response 계약을 따른다. 실제 active management 응답은 redirects:deny, maxBytes4194304이며 ETag와 선언된 오류 status를 보존한다. 다른 Provider의 계약을 동일하다고 가정하지 않는다. |
| quorum fence | replica가 오래된 허용으로 최근 권한 회수를 놓치지 않게 한다. | Learner는 fence 이상으로 따라잡은 동일 snapshot에서 읽고 변경시 재시도한다. 실패하면503이며 일반 관리 API는 learner에 열지 않는다. |

배포 버전의 관리 onboarding은 신원을 먼저 만들 수 있도록 trusted configured app과 정확한 4개 계약(`/me`, materialize, org list/create)만 제한적으로 허용한다. Tenant management는 요청 path 조직에 결합해 scope를 확인하고 복구 요청은 exact inactive-self 예외를 사용한다. 이를 모든 Provider에 적용되는 membership/consent 우회로 확대하지 않는다.

### 5.3 Bootstrap·백업·PEP·권한 import

Owner bootstrap은 M과 B, 조직 헤더로 선택한 조직에 identity/owner를 원자 연결한다. `201 {"userId":"<USER_ID>","organizationId":"<ORG_ID>"}`다. **배포 버전은 앱을 함께 만들어 trusted로 승격하는 bootstrap이 아니다.** inactive 신원·조직/owner/identity 충돌은 거부되며 일반 사용자의 가입 경로와 분리한다.

Backup GET의 items는 name/lastModified/size와 `storage:"local"` 또는 `"s3"`를 가진다. 로컬 목록과 설정된 S3 목록을 합쳐 반환하며 보관소 조회 실패는 오류다. Backup POST는 논리 snapshot을 로컬에 기록하고 S3 설정이면 업로드까지 수행하며 202여도 job ID를 제공하지 않는다. HTTP restore/download/delete API는 없다. 로컬과 S3를 따로 조회하던 과거 public 경로를 현행으로 안내하지 않는다.

Enforcer PUT body는 `{"applicationId":"<PEP_APP_ID>","lifecycle":"active","validUntil":null}`다. Active Provider와 active/ready 앱이 필요하며 lifecycle은 active/disabled/deleted다. Deleted는 물리 삭제가 아니다. 기존 PEP ID의 Provider/app 재지정과 전역 app identity 충돌은 409, 형식은 422, 미활성/만료 PEP의 catalog/decide는403이다. 서버의 configured PEP app와 연결되어야 한다.

Authority migration은 `manifestVersion:"grant.patch-data-authority.v1",generation:"<SOURCE_GENERATION>",ledgerDigest:"<LOWERCASE_HEX_64>",manifestDigest:"<LOWERCASE_HEX_64>",expectedPreviousGeneration:null`과 users/organizations/memberships/hierarchyEdges/plantBindings/applicationAssignments 배열을 모두 받는다. Prefix 없는64hex와 versioned canonical digest를 사용한다. Users는 issuer/collection/external namespace를 포함한다. 200 applied/generation/digests/appliedAt이며 같은 snapshot 재시도는 검증 후 applied:false다.

최대 총 entity10000과 JSON2MiB 제한, 정렬/unique/참조/CAS/import ownership을 검사한다. Native 권한을 우연히 같은 ID라는 이유로 인수하지 않는다. Omitted import membership/binding은 제거되어 권한을 회수하고 앱은 existing same-org active/ready를 참조할 뿐 생성하지 않는다. Hierarchy snapshot은 권한 상속이 아니다. Validator의 plant subject는 user/organization이며 group을 일반 role-binding처럼 지원한다고 추정하지 않는다. 실패하면 canonical 권한을 부분 반영하지 않지만 staging row가 남을 수 있다. 오류의 HTTP status는 실제 validator/DB 공통 변환을 따른다.

## 6. Private runtime과 retired 경로

별도 runtime listener에는 다음 **3개**가 있다. 사용자 관리 API나 management69에 포함하지 않는다. Remote transport credential은 사용자 Bearer와 다른 내부 secret이며 실제 값은 기록하지 않는다.

| 메서드·경로 | 근본적인 필요 | 요청·응답·제한 |
|---|---|---|
| POST `/read-fence` | Replica에 요청 시작 이후의 quorum 기준점을 제공하여 최근 회수를 반영한다. | 전용 read-fence-key Bearer, body 없음 →200 appliedSlot. 인증401, method405, voter/quorum 부재503. |
| POST `/sql/query` | remote/split 서비스가 voter의 DB 권위를 읽는다. | remote secret Bearer, sql/args/consistency →200 Rhiza query rows/columns/appliedSlot. Local/linearizable만 허용한다. |
| POST `/sql/execute` | 합의 DB에 requestId와 statement를 전달해 mutation 결과를 관리한다. | remote secret Bearer, requestId/statements →200 mutation receipt/status. HTTP200과 committed는 별도 확인한다. |

SQL 경계는 voter 전용, body20MiB, duplicate/unknown/trailing JSON 거부이며 operation timeout이 있다. 일반 사용자가 입력한 SQL을 조직 API처럼 실행하는 사용법이 아니다. Public mux의 동일 path는 private SQL로 이어지지 않는다.

배포 fixture에는 Rauthy `/auth`, `/auth/v1` 프록시, provisioning/verify, 분리된 rauthy-owner/external-owner bootstrap, `/api/v1/admin/backups[/s3]`, `/api/v1/admin/providers/.../pep-clients/...`, `/api/v1/admin/imports/patch-data-authority`가 없다. 현재 ops/bootstrap/identity-binding 경로와 혼동하지 않는다. Rauthy identity-binding 코드가 있어도 현재 PocketBase 전제에서는 해당 hook을 사용할 수 없다.

Exposure 전용 `/api/v1/exposure-authz/read`, `/api/v1/exposure-authz/plant-read`, `/api/v2/exposure-authz/decide`와 `/api/v2/admin/imports/authority-snapshots`는 retired 경로이며 현행 목록에 없다. Generic HTTP capability로 자동 alias하지 않는다. 필드별 mask/view 관리·저장 인증서/credential/crawl token 관리·업무 레코드 CRUD는 Grant 관리 목록에 없다. Provider catalog의 업무 URL을 Grant 자체 CRUD로 설명하지 않는다.

## 7. 근거와 확인 범위

현재 live catalog의 **69개**와 배포 route fixture의 **84개**를 전수 대조하고, private runtime **3개**를 분리했다. 각 관리 feature의 목적·없을 때의 문제·주체/권한·요청/응답·거부/한계를 설명했다. HTTP method/path가 배포 소스에 있고 catalog에 enabled로 실려 있다는 사실, 실제 특정 GET이 성공했다는 사실, 모든 권한 주체/변경 요청이 테스트됐다는 사실은 서로 다르다. 마지막 항목은 주장하지 않는다.

소스 링크는 모두 **배포 커밋**을 고정한다. 로컬 같은 파일의 줄 번호를 배포 코드의 근거로 사용하지 않는다.

| 확인 항목 | 정확한 근거 |
|---|---|
| 이미지/현재 catalog | [grant-commit-comparison.json](runtime/grant-commit-comparison.json), [catalog-management.json](runtime/catalog-management.json), [관리자 catalog](runtime/catalog-admin-grant-management.json), [Provider 액션 실측](runtime/provider-actions-grant-management.json), [runtime-summary.md](runtime/runtime-summary.md) |
| 전체84 method/path | [배포 fixture](https://github.com/mrchypark/grant/blob/16020fe78bc4a1908f8cc105f0a0c728f5bbce4e/internal/api/testdata/routes-voter.txt), [TSV](grant-deployed-routes.tsv) |
| 기본 등록/ETag 호출 | [routes.go](https://github.com/mrchypark/grant/blob/16020fe78bc4a1908f8cc105f0a0c728f5bbce4e/internal/api/routes.go), [routes_action_mutations.go](https://github.com/mrchypark/grant/blob/16020fe78bc4a1908f8cc105f0a0c728f5bbce4e/internal/api/routes_action_mutations.go) |
| 조직/앱/role/member lifecycle | [routes_lifecycle.go](https://github.com/mrchypark/grant/blob/16020fe78bc4a1908f8cc105f0a0c728f5bbce4e/internal/api/routes_lifecycle.go), [service_entity_lifecycle.go](https://github.com/mrchypark/grant/blob/16020fe78bc4a1908f8cc105f0a0c728f5bbce4e/internal/grant/service_entity_lifecycle.go), [service_contract_lifecycle.go](https://github.com/mrchypark/grant/blob/16020fe78bc4a1908f8cc105f0a0c728f5bbce4e/internal/grant/service_contract_lifecycle.go) |
| Owner/onboarding | [routes_ownership.go](https://github.com/mrchypark/grant/blob/16020fe78bc4a1908f8cc105f0a0c728f5bbce4e/internal/api/routes_ownership.go), [service_ownership.go](https://github.com/mrchypark/grant/blob/16020fe78bc4a1908f8cc105f0a0c728f5bbce4e/internal/grant/service_ownership.go), [service_onboarding.go](https://github.com/mrchypark/grant/blob/16020fe78bc4a1908f8cc105f0a0c728f5bbce4e/internal/grant/service_onboarding.go) |
| 액션·ETag·동의 | [store_action_mutations.go](https://github.com/mrchypark/grant/blob/16020fe78bc4a1908f8cc105f0a0c728f5bbce4e/internal/grant/store_action_mutations.go#L113), [service_action_mutations.go](https://github.com/mrchypark/grant/blob/16020fe78bc4a1908f8cc105f0a0c728f5bbce4e/internal/grant/service_action_mutations.go), [service_applications.go](https://github.com/mrchypark/grant/blob/16020fe78bc4a1908f8cc105f0a0c728f5bbce4e/internal/grant/service_applications.go) |
| 인증·learner·platform gate | [http.go의 master/Bearer 분기](https://github.com/mrchypark/grant/blob/16020fe78bc4a1908f8cc105f0a0c728f5bbce4e/internal/api/http.go#L105), [AuthorizePlatformManagement](https://github.com/mrchypark/grant/blob/16020fe78bc4a1908f8cc105f0a0c728f5bbce4e/internal/grant/service_management.go#L21), [Platform:true registry](https://github.com/mrchypark/grant/blob/16020fe78bc4a1908f8cc105f0a0c728f5bbce4e/internal/grant/management_catalog.go#L24) |
| 협약·그룹·권한 | [service_agreements.go](https://github.com/mrchypark/grant/blob/16020fe78bc4a1908f8cc105f0a0c728f5bbce4e/internal/grant/service_agreements.go), [service_groups.go](https://github.com/mrchypark/grant/blob/16020fe78bc4a1908f8cc105f0a0c728f5bbce4e/internal/grant/service_groups.go), [store_authz.sql](https://github.com/mrchypark/grant/blob/16020fe78bc4a1908f8cc105f0a0c728f5bbce4e/internal/grant/store_authz.sql) |
| PDP/ops/runtime | [authz.go](https://github.com/mrchypark/grant/blob/16020fe78bc4a1908f8cc105f0a0c728f5bbce4e/internal/api/authz.go), [capabilities.go](https://github.com/mrchypark/grant/blob/16020fe78bc4a1908f8cc105f0a0c728f5bbce4e/internal/api/capabilities.go), [service_capabilities.go](https://github.com/mrchypark/grant/blob/16020fe78bc4a1908f8cc105f0a0c728f5bbce4e/internal/grant/service_capabilities.go), [service_bootstrap.go](https://github.com/mrchypark/grant/blob/16020fe78bc4a1908f8cc105f0a0c728f5bbce4e/internal/grant/service_bootstrap.go), [store_remote.go](https://github.com/mrchypark/grant/blob/16020fe78bc4a1908f8cc105f0a0c728f5bbce4e/internal/grant/store_remote.go) |
| Rauthy identity hook | [identity_binding.go](https://github.com/mrchypark/grant/blob/16020fe78bc4a1908f8cc105f0a0c728f5bbce4e/internal/grant/identity_binding.go) |
| 기존 개념 설명의 보조 자료 | [http-capabilities.md](../../http-capabilities.md), [database-schema.md](../../database-schema.md), [registration-api-management.md](../../registration-api-management.md) |

기존 문서는 모델 이해의 보조 근거로 사용했으며 배포 커밋의 변경과 live 관측을 우선했다. 변경한 파일은 이 안내서와 추가로 허용된 deployed routes TSV뿐이다. README와 통합 inventory TSV/JSON, 코드, 스키마, 다른 담당자의 산출물은 수정하지 않았다.
