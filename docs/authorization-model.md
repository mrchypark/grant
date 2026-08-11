# Grant Authorization Model

`grant`는 identity provider가 아니다. 사용자의 로그인과 외부 identity 확인은 Rauthy 같은 OIDC provider가 맡고, `grant`는 조직, 애플리케이션, service provider, resource scope에 대한 권한 부여와 판정을 맡는다.

## 핵심 객체

- `organizations`: tenant와 권한 평가의 기본 단위다.
- `parties`: 법인/사업자 식별자다. `organizations.id`와 `business_no`는 같은 값으로 보지 않는다.
- `groups`: 조직 내부 사용자 묶음이다.
- `service_providers`: grant가 권한 판정 대상으로 인정하는 provider catalog다. grant 자체의 provider id는 `grant`다.
- `actions`: provider별 action catalog다.
- `roles`: 조직 안에서 action 묶음을 이름 붙인 것이다.
- `role_bindings`: subject가 어떤 provider/resource scope에 어떤 role을 가지는지 저장하는 권한 원본이다.
- `role_action_field_views`: 보존된 field authorization history를 위한 legacy table이다. coarse deploy runtime은 읽거나 평가하지 않는다.
- `agreements`: 조직 간 권한 부여의 근거다.
- `applications`: 조직 내부 또는 first-party application이다.
- `application_grants`: 사용자가 application에 동의한 action subset이다.

## Role Binding Scope

`role_bindings`의 scope는 항상 아래 세 값으로 해석한다.

```text
provider_id + resource_type + resource_id
```

grant 내부 조직 권한은 다음 형태다.

```text
provider_id = grant
resource_type = organization
resource_id = organizations.id
```

compos 같은 외부 provider의 resource는 해당 provider가 정의한 `resource_type/resource_id`를 그대로 저장한다. grant는 외부 resource 목록을 소유하지 않으므로 존재 여부를 DB FK로 검증하지 않는다.

## Subject

binding subject는 세 가지다.

- `organization`: 조직 전체에 권한을 부여한다.
- `group`: 조직 내부 그룹에 권한을 부여한다.
- `user`: 조직 소속 사용자에게 직접 권한을 부여한다.

group/user binding은 반드시 `subject_organization_id` 안에서만 유효하다. organization subject binding으로 다른 조직 resource에 접근시키려면 active `agreements`가 필요하다.

그룹 관리 API는 `organization.group.manage`로 보호한다. list/get은 learner에서도 읽을 수 있지만 create/update/delete와 member 변경은 voter에서만 수행한다. member 추가는 같은 조직의 active 사용자 membership에 한정되고, group binding이 남아 있으면 group 삭제는 `409 Conflict`다.

## Coarse Deploy Baseline And Preserved Field Work

현재 deploy baseline의 Grant 판정은 subject, action, provider/resource scope에 대한 binary allow/deny다.
`check`와 `batch-check` 응답은 `allowed`와 `reason`만 반환하며 field set, field mode, mask를 반환하지
않는다. `filter`는 coarse 판정을 통과한 resource id만 반환한다.

여섯 trusted/application `check`, `batch-check`, `filter` endpoint에서 resource에 `view` key가 존재하면
그 값의 shape, `schemaKey`, pointer를 해석하지 않고 모두 `400 Bad Request`로 거부한다. `view: null`도
field-aware capability 요청으로 간주한다. field-aware 요청을 viewless 요청 또는 모든 field 허용으로
해석하는 fallback은 없다.

field policy `GET|PUT .../field-views` route는 voter와 learner 모두 등록하지 않는다. action catalog
mutation에서 `visibilityGranting=true`도 write 전에 거부한다. 보존된 legacy field-view table과
migration은 이 phase에서 삭제하지 않지만 coarse evaluator/readiness는 그 table에 의존하지 않는다.
반대로 active/nonempty field policy/audit row, 알려진 mask/activation state 또는 `visibility_granting` action이 있는
database는 startup에서 fail closed하며 별도 forward/restore 계획이 필요하다. 빈 legacy table만 있는
fresh/untouched state는 blocker가 아니다.

과거 field/column authorization 설계와 구현 기록은
[field-view-authorization-development.md](./field-view-authorization-development.md)에 보존한다. 이 문서는
현재 deploy contract가 아니라 향후 verified requirement가 생길 때 다시 평가할 historical design이다.

## Agreements

조직 간 권한 부여는 active agreement를 근거로 둔다.

- `grantor_organization_id`: resource를 제공하는 조직
- `grantee_organization_id`: 권한을 받는 조직
- `status = active`
- `starts_at`/`ends_at` 기간 안에 있어야 함

grant 내부 organization resource에 대해 `resource_id != subject_organization_id`인 organization binding은 `agreement_id` 없이 만들 수 없다.

## Applications

application은 조직이 만든 실행 주체다. 조직 owner/관리자는 application 자체를 허가하거나 비활성화할 수 있고, 사용자는 `/api/v1/orgs/{org}/apps/{app}/consent`의 GET/PUT/DELETE로 자기 자신의 명시적 consent만 관리한다. body로 다른 `userId`나 application id를 지정할 수 없다. 과거의 조직 관리자 grant mutation 경로는 제공하지 않는다.

application이 사용할 수 있는 action은 `application_actions`에 먼저 등록되어야 하며, `application_grant_actions`는 그 subset만 허용된다.

현재 release는 redirect URI 변경이나 client secret 회전 API를 노출하지 않는다. 조직의 application 생성은 generated Rauthy client id를 가진 `unprovisioned` local reservation을 만들며, 선택적 scalar `redirectUri`를 application과 같은 transaction에서 manifest에 저장할 수 있다. 생략하면 manifest는 빈 배열이다. 값은 canonical absolute HTTPS 또는 explicit-port literal loopback-IP HTTP여야 하며 userinfo, query, fragment, wildcard, localhost, custom scheme은 거부된다. create와 single-application GET만 optional `redirectUri`를 readback하고 목록 API의 shape은 바꾸지 않는다. 별도 운영자/IaC가 Rauthy client를 생성한 뒤 master provisioning verify는 `Clients` 그룹의 `read` 권한만 가진 API key로 `GET /auth/v1/clients/{client_id}`를 호출해 id, 이름, 활성·confidential 상태, local manifest와 byte-for-byte 같은 redirect URI 배열, flow 설정을 검증해야 application이 `ready`가 된다. public application은 Rauthy challenges의 exact set이 `{S256}`이어야 한다. Grant는 Rauthy client를 생성·수정하거나 client secret을 회전하지 않으며, 운영자/IaC의 write credential도 보유하지 않는다.

trusted auto-approve는 `first_party = true`와 `consent_policy = auto_approve`가 모두 성립할 때만 적용한다. 조직 application API는 두 신뢰 필드를 입력받지 않으며 master trust API만 승격/해제한다. trusted application은 조직 경로에서 보안 설정을 바꿀 수 없다.

application action 판정은 `RBAC ∩ active user/org/provider/application ∩ current application_actions ∩ consent`다. consent는 exact 사용자 active grant의 현재 action이거나 trusted auto-approve여야 한다. action 축소 뒤 stale grant를 재사용하는 fallback은 없다.

## Authz API

외부 서비스는 세 가지 API를 사용한다.

```text
POST /api/v1/authz/check
POST /api/v1/authz/batch-check
POST /api/v1/authz/filter
```

`check`는 단일 resource 판정, `batch-check`는 여러 판정, `filter`는 서비스가 넘긴 후보 resource 중 허용된 id만 반환한다.

grant는 외부 provider의 전체 resource 목록을 모른다. 따라서 unbounded list API는 제공하지 않는다.

Exposure read V1은 한 개의 application-authenticated endpoint만 추가한다.

```text
POST /api/v1/exposure-authz/read
```

이 endpoint는 기존 application-authz와 같은 `Authorization: Bearer ...`, `x-organization-id`,
`x-grant-app-auth-key`를 요구하고 strong consistency coarse evaluator를 사용한다. body는 닫힌 object이며
`exposureId`, 0보다 큰 정수 `exposureVersion`, 정확히 71 ASCII byte인
`sha256:<64 lowercase hex>` literal `artifactDigest`,
`resourceId`, opaque `requestBinding`만 허용한다. action은 caller가 선택하지 않고 `exposure.read`로
고정된다. Compos schema, pointer, field/view/mask, filter, transform, resource version은 받거나 반환하지
않는다.

V1 policy key는 아래처럼 version/digest와 분리한다.

```text
provider_id = exposure
action_id = exposure.read
resource_type = exposureId
resource_id = resourceId
```

따라서 같은 `exposureId`에 대한 기존 permission은 Exposure가 publish한 새 immutable version에도
의도적으로 이어진다. `exposureVersion`과 `artifactDigest`는 policy row가 아니라 decision replay
binding이다. prefix가 없는 digest, uppercase, 다른 prefix, whitespace는 거부하며 accepted
`artifactDigest`를 normalize하거나 prefix를 추가하지 않고 byte-for-byte echo한다. allow와 policy deny 모두
exact exposure tuple, `resourceId`, `requestBinding`, 검증된 내부
user/org/application에서 만든 domain-separated `principalDigest`, `issuedAt`, `expiresAt`을 반환한다.
5초 `expiresAt`은 Grant decision freshness envelope이지 bearer token 만료 시각에 대한 주장이 아니다.
Exposure는 business data를 읽기 전에 모든 echo field와 `requestBinding`, 아직 지나지 않은 `expiresAt`을
원 요청과 비교해야 하며 다른 tuple/resource에 decision을 재사용하면 안 된다. per-version grant는 V1
non-goal이다. Grant는 Compos를 조회하지 않고 authoritative source가 없으므로 `resourceVersion` 또는
`expectedResourceVersion`을 만들지 않는다.

fresh database는 의도적으로 Grant 자체 provider/action만 seed하므로 Exposure read는 사전 provisioning
전까지 fail-closed deny다. 배포/IaC는 기존 surface를 사용해 다음 상태를 명시적으로 만들어야 한다.

1. master `POST /api/v1/admin/providers`로 active provider `exposure`를 등록한다.
2. master `PUT /api/v1/admin/providers/exposure/actions`로 `exposure.read`를
   `visibilityGranting=false`, `grantableToApplications=true`로 등록한다.
3. 조직 role을 만들고 `PUT .../roles/{roleId}/actions`로 `exposure.read`를 연결한 뒤,
   `POST .../bindings`로 exact `resourceType=exposureId`, `resourceId=resourceId` scope를 부여한다.
4. Exposure application reservation과 Rauthy client를 기존 provisioning/verify 절차로 ready 상태로 만들고
   current application action에 `exposure.read`를 포함한다. 그 뒤 master trust를 명시적으로 부여하거나
   exact 사용자의 active consent에 이 action을 포함한다.

이 절차는 provider/action catalog나 application capability를 준비할 뿐 grant, binding, trust, consent를
자동 생성하지 않는다. 어느 단계든 빠지거나 inactive/expired이면 decision은 deny다.

애플리케이션이 최종 사용자 토큰으로 판정을 요청할 때는 별도 API를 사용한다.

```text
POST /api/v1/application-authz/check
POST /api/v1/application-authz/batch-check
POST /api/v1/application-authz/filter
```

이 경로는 전용 `x-grant-app-auth-key`, Bearer token, `x-organization-id`를 모두 요구하고 strong consistency만 사용한다. `GRANT_APP_AUTH_KEY`는 필수이며 `GRANT_MASTER_KEY`와 다른 값이어야 한다. master key는 application-authz 인증에 사용할 수 없고 app-auth key도 admin 인증에 사용할 수 없다. subject와 application id는 body에서 받지 않는다. 서버가 검증된 토큰 `sub`와 `client_id`를 사용해 exact 사용자와 동일 조직의 active application을 결정한다. client mismatch, 모호한 매핑, 검증 정보가 없는 cache 또는 identity provider 장애에서 안전한 cache가 없으면 fail closed한다. 기존 `/api/v1/authz/*` master API는 `x-grant-master-key`로 명시한 subject를 검사하는 trusted-provider 계약으로 유지한다.

## 업데이트 규칙

아래가 바뀌면 이 문서와 [database-schema.md](./database-schema.md)를 함께 갱신한다.

- `role_bindings` scope 의미
- agreement 검증 규칙
- application grant action subset 규칙
- developer console application 설정 권한 규칙
- authz check/filter 응답 계약
- coarse/field capability 경계와 preserved field-work 상태
