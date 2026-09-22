# 내부 Provider·Application 등록 API 정리

상태: 현재 브랜치의 등록·수명주기 API 계약이다. 실제 IED 배포 완료를 뜻하지 않는다. 적용 시 새 관리 카탈로그 세대와 필요한 Action 부여를 함께 반영해야 한다. 범위는 Kubernetes 내부 Provider이며 외부 SDK·mTLS 및 Goauthy 어댑터는 포함하지 않는다.

## 변경 계획

Provider 생성·수정 분리, 액션 전체 교체, 조직별 Application 생성·액션 선택 및 catalog stage/activate 계약을 유지한다. 운영자가 등록 내용을 확인하고 앱에 부여할 후보 액션을 선택할 수 있도록 다음 조회를 추가한다.

| 추가 조회 | 인증·범위 | 목적 |
|---|---|---|
| GET `/api/v1/admin/providers/{providerId}/actions` | master, voter | 해당 Provider의 전체 액션을 `[]Action`으로 조회한다. |
| GET `/api/v1/admin/providers/{providerId}/http-capability-active` | master, voter | PEP 설정과 독립적으로 현재 활성 catalog 상태를 확인한다. |
| GET `/api/v1/orgs/{organizationId}/providers/{providerId}/actions` | user, voter, 해당 조직의 `organization.application.manage` | 활성 Provider에서 `grantableToApplications=true`인 후보 액션을 조회한다. |

세 조회는 strong read를 사용한다. 액션 조회는 `code` 순서의 `[]Action`을 반환하고 빈 결과는 `[]`다. 존재하지 않는 Provider는 404이며, 조직 후보 조회에서는 비활성 Provider도 404다. 관리자 조회는 비활성 Provider도 확인할 수 있다.

관리자 활성 catalog 응답은 `schemaVersion`, `providerId`, `generation`, `catalogDigest`, `activatedAt`, `activatedBy`, `capabilityCount`, `responseResourceCount`, `capabilities`와 선택적 `responseResources`를 포함한다. PEP용 응답의 `pepClientId`, `issuedAt`, `expiresAt`은 포함하지 않는다. Provider가 없으면 404 `service provider not found`, 활성 catalog가 없으면 404 `HTTP capability catalog not active`를 반환한다. 이 조회는 PEP 설정 없이도 작동하며, 실제 전달용 catalog는 기존 `/api/v2/http-capabilities/catalog/{providerId}`를 사용한다.

후보 조회는 사용자에게 해당 리소스 권한이 있다는 뜻이 아니다. 앱이 이미 선택한 액션은 기존 GET `/api/v1/orgs/{organizationId}/apps/{applicationId}/actions`로 조회한다. Provider 액션 정의와 Application의 사용 액션은 구분한다.

액션 전체 교체에서 활성 catalog가 참조하는 액션을 삭제하면 409로 거부한다. Catalog를 활성화할 때도 현재 등록된 액션 참조를 다시 검증한다. 액션을 앱에 부여할 수 없도록 변경하면 기존 앱 선택과 동의의 해당 액션 연결을 제거하되 역할 연결은 보존한다. Application 액션 교체는 비활성 Provider의 액션을 거부한다. 이 검사는 동시 변경에서도 일관되게 적용되어야 한다.

## 등록과 사용 순서

1. 내부 운영자는 `X-Grant-Master-Key`로 POST `/api/v1/admin/providers`에 `{"id":"patch-data","name":"Patch Data","active":true}`를 보낸다. 기존 ID에는 409를 반환하며 수정은 PATCH `/api/v1/admin/providers/{providerId}`로 요청한다.
2. 운영자는 PUT `/api/v1/admin/providers/patch-data/actions`에 `{"actions":[...]}`를 보낸 뒤 관리자 액션 조회로 확인한다. 이 요청은 전체 교체이므로 기존에 유지할 액션과 GET 응답의 ETag를 If-Match로 함께 보낸다.
3. 조직 관리자는 `Authorization: Bearer <token>`과 `X-Organization-ID: <org>`를 사용해 POST `/api/v1/orgs/{org}/apps`로 Application을 등록한다. `applicationType`, `appType`과 선택적 redirect manifest를 등록한다. IdP client ID를 생성하지 않는다. Rauthy 앱은 플랫폼 identity-binding 검증 뒤 ready이며 PocketBase 앱은 기존처럼 ID 없이 ready다.
4. 조직 관리자는 Provider 후보 액션을 조회한 후 PUT `/api/v1/orgs/{org}/apps/{app}/actions`에 `{"actionIds":["patch-data.plant.read"]}`를 보낸다. GET으로 얻은 ETag를 If-Match로 보내 전체 선택을 교체하며, 기존 trusted 앱의 변경 제한과 사용자 동의 정책을 유지한다.
5. 권한 관리자는 역할에 같은 액션 ID를 배정하고 필요한 사용자·그룹·조직에 정확한 리소스 binding을 생성한다. 예를 들어 plant 읽기는 `providerId:"patch-data"`, `resourceType:"plant"`, `resourceId:"<plant ID>"`를 사용한다. 앱 선택만으로 이 권한을 얻지 않는다.
6. 운영자는 기존 generation stage, PEP 등록, compare-and-swap activation 절차를 수행한다. 관리자 활성 상태 조회로 확인한 뒤 기존 PEP catalog 조회와 실제 Exposure 요청으로 사용을 검증한다.

액션 정의 예시는 다음과 같다. 실제 전체 교체 요청에서는 다른 유지 대상 액션도 함께 전송한다.

```json
{"actions":[{"id":"patch-data.plant.read","code":"plant.read","name":"Read plant","riskLevel":"high","visibilityGranting":false,"grantableToApplications":true}]}
```

활성 catalog에서 액션을 제거하려면 해당 액션을 참조하지 않는 다음 catalog를 먼저 stage·activate한 후 액션 전체 교체를 수행한다. 액션 참조는 일반 capability뿐 아니라 catalog가 선언하는 관련 응답과 배치 계약도 검증 대상이다.

## 검증과 완료 기준

- 관리자 조회는 master 인증과 voter 제한을 지키고, Provider별 결과를 섞지 않는다.
- 조직 후보 조회는 인증·조직 일치·앱 관리 권한을 검사하고 비활성 Provider와 부여 불가능한 액션을 노출하지 않는다.
- 등록 액션 조회는 존재하지 않는 Provider와 액션이 없는 Provider를 구분한다. 활성 상태 조회의 미등록 상태도 HTTP 계약에 명시한다.
- 활성 참조 액션 삭제, stale catalog 활성화, 비활성 Provider 액션 선택을 거부한다. 실패한 변경은 기존 상태를 보존한다.
- 액션의 앱 부여 가능 여부를 해제하면 앱 선택·동의 연결이 제거되고 역할 연결은 유지된다.
- HTTP route fixture, 계약 문서와 실제 테스트 결과를 일치시킨다. 새 경로를 Exposure에 공개하거나 배포했다고 추정하지 않는다.

[HTTP 계약](porting-go-http-contract.md), [권한 모델](authorization-model.md), [catalog 운영](http-capabilities.md)을 함께 참조한다. 관계나 권한 전개 의미가 바뀌는 내용은 [database-schema.md](database-schema.md)의 릴레이션과 구조의 이유에도 반영한다.

## 부분 변경과 동시 수정 방지

목록에 한 액션을 추가할 때는 전체 교체보다 개별 API를 사용한다.

| 대상 | 요청 | 의미 |
|---|---|---|
| Provider 액션 | POST `/api/v1/admin/providers/{providerId}/actions` | 단일 액션 정의 생성, 201 |
| Provider 액션 | PATCH `/api/v1/admin/providers/{providerId}/actions/{actionId}` | 전달한 메타데이터만 수정, 200 |
| Provider 액션 | DELETE 위 개별 경로 | 액션과 역할·앱·동의 연결 삭제, 204 |
| 역할 액션 | PUT/DELETE `/api/v1/orgs/{organizationId}/roles/{roleId}/actions/{actionId}` | 본문 없이 단일 연결 추가·제거, 204 |
| 앱 액션 | PUT/DELETE `/api/v1/orgs/{organizationId}/apps/{applicationId}/actions/{actionId}` | 본문 없이 단일 연결 추가·제거, 204 |

PATCH는 `name`, `description`, `riskLevel`, `visibilityGranting`, `grantableToApplications`, `consentLabel`, `consentDescription`을 받는다. `id`, `code`, `providerId`는 변경할 수 없다. 생략한 필드는 유지하며 nullable 문자열에 `null`을 보내면 값을 제거한다. `visibilityGranting=true`는 현재 지원하지 않는다. 같은 필드를 동시에 PATCH하면 나중에 실행된 트랜잭션의 값이 남는다.

Provider·역할·앱의 `GET .../actions` 응답에서 `ETag`를 얻는다. 전체 교체 `PUT .../actions`에는 정확한 값을 `If-Match`로 보낸다. 조건이 없으면 428, GET 이후 상태가 달라지면 412다. 실패 시 다시 GET하여 변경 내용을 병합한 뒤 요청한다. `PUT`의 전체 교체 의미는 유지하며 무조건 재시도해서 다른 변경을 덮어쓰지 않는다.

사용자 동의는 선택 목록 전체를 확인하는 절차를 유지한다. 최초 `PUT .../consent`에는 `If-None-Match: *`, 기존 동의 갱신에는 `GET .../consent`의 `ETag`를 `If-Match`로 보낸다. 아직 동의가 없으면 GET은 404다. 앱 액션 개별 삭제도 이전 동의의 연결을 같은 트랜잭션에서 제거한다. trusted 앱 변경 제한과 활성 Provider·앱 부여 가능 여부 검사는 개별 API에도 적용된다.

## 초기 등록과 운영 전용 API

`POST /api/v1/admin/bootstrap/owner`는 master key, 설정된 IdP의 Bearer, 명시적 `X-Organization-ID`를 요구한다. 사용자 identity와 조직 owner 관계만 만들며 Application은 위 등록 API로 별도 생성한다. 인증 공급자 이름을 API 경로로 선택하지 않는다. 기존 `rauthy-owner`·`external-owner` 경로는 제거한다.

백업은 private `GET/POST /ops/backups`, 기존 PatchData 권한 이전은 private `POST /ops/migrations/patch-data-authority`로 옮긴다. 이전 도구는 `scripts/import-patch-data-authority.sh`로 호출할 수 있다. 이 특정 레거시 형식의 이름은 마이그레이션 어댑터에만 남고 일반 등록 API에는 들어가지 않는다.

PEP는 판정을 요청하는 배포 인스턴스의 신뢰 경계를 식별한다. 개발자의 Application 등록과 구분하여 배포 저장소의 선언 파일로 관리하고, 운영자가 private `PUT /ops/providers/{providerId}/enforcers/{pepClientId}`에 적용한다. Pod 시작 때 자동 재등록하지 않아 운영자가 비활성화한 바인딩을 되살리지 않는다. `/ops`와 bootstrap은 관리 Exposure 카탈로그에서 제외하고 master key로만 호출한다.

## 등록 이후의 관리

[수명주기 전체 계약](api-lifecycle.md)의 조직 생성·목록·프로필, 구성원 제거, 앱 상태·삭제, 역할 수정·삭제 및 관리자 동의 철회 경로를 사용한다. `POST /api/v1/admin/orgs`는 기존 활성 `ownerUserId`에 owner membership과 조직 범위 owner Binding을 원자 생성한다. 호출자를 암묵적으로 owner로 만들지 않는다.

IdP는 클라이언트를 별도로 등록한다. Grant에는 플랫폼 운영자가 `PUT /api/v1/admin/orgs/{organizationId}/apps/{applicationId}/identity-binding`으로 `{"clientId":"existing-client"}`를 전달한다. 읽기 전용 검증을 통과한 기존 클라이언트만 연결되며 일반 조직 관리자가 임의 ID를 등록해 ready로 만들 수 없다. 이전 POST provisioning/verify 및 Grant auth 프록시는 제거되었다. 구성원 재가입은 삭제된 개인 권한을 복원하지 않지만 현재 조직 전체 권한은 적용할 수 있다.
