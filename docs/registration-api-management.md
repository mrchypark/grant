# 내부 Provider·Application 등록 API 정리

상태: 추가 조회와 lifecycle 검사 구현 및 로컬 검증을 완료했다. `go test -race ./...`, `go vet ./...`, `git diff --check`가 통과했다. 이번 변경을 IED에 배포하거나 신규 API를 IED에서 검증하지는 않았다. 범위는 K8s에 직접 배포하는 내부 서비스이며, [외부 Application 로드맵](external-application-roadmap.md)의 IdP 통합 등록·mTLS·SDK·다중 조직 설치는 이번 구현에 포함하지 않는다.

## 변경 계획

기존 Provider upsert, 액션 전체 교체, 조직별 Application 생성·액션 선택 및 catalog stage/activate 계약을 유지한다. 운영자가 등록 내용을 확인하고 앱에 부여할 후보 액션을 선택할 수 있도록 다음 조회를 추가한다.

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

1. 내부 운영자는 `X-Grant-Master-Key`로 POST `/api/v1/admin/providers`에 `{"id":"patch-data","name":"Patch Data","active":true}`를 보낸다. 기존 ID는 갱신하는 upsert 의미를 유지한다.
2. 운영자는 PUT `/api/v1/admin/providers/patch-data/actions`에 `{"actions":[...]}`를 보낸 뒤 관리자 액션 조회로 확인한다. 이 요청은 전체 교체이므로 기존에 유지할 액션도 포함한다.
3. 조직 관리자는 `Authorization: Bearer <token>`과 `X-Organization-ID: <org>`를 사용해 POST `/api/v1/orgs/{org}/apps`로 Application을 등록한다. 기존 `applicationType`, `appType`, provisioning 계약을 따른다. IdP client를 자동 생성하는 새로운 기능은 추가하지 않는다.
4. 조직 관리자는 Provider 후보 액션을 조회한 후 PUT `/api/v1/orgs/{org}/apps/{app}/actions`에 `{"actionIds":["patch-data.plant.read"]}`를 보낸다. 전체 선택을 교체하며, 기존 trusted 앱의 변경 제한과 사용자 동의 정책을 유지한다.
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
