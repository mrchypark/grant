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

developer console의 redirect URI와 client secret rotation은 기존 `applications`에 붙은 설정이다. 별도 partner 테이블을 만들지 않고, create/list/delete redirect URI와 secret rotate API는 기존 `organization.application.manage` 권한으로 보호한다. Grant backend는 제한된 Rauthy Admin API key로 해당 application의 Rauthy client를 생성/수정하고, secret rotation 결과만 one-time 응답으로 반환한다.

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
