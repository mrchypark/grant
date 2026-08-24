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

## Patch Data authentication mode

`GRANT_AUTH_PROVIDER=patch-data`를 선택하면 Rauthy introspection/userinfo 경로 대신 Grant가 설정된 내부 origin `GRANT_PATCH_DATA_BASE_URL`의 고정된 `POST /api/collections/users/auth-refresh`만 호출한다. origin은 정확한 `http://patch-data.<lowercase-dns-label-namespace>.svc.cluster.local:8090` 또는 로컬 테스트용 literal loopback IP의 `http://<loopback>:8090`만 허용한다. HTTPS, 다른 service/port, public·private·metadata IP, bare/wildcard host, uppercase/noncanonical DNS, userinfo, path(인코딩 포함), query, fragment는 startup에서 거부한다. HTTP/ALL proxy 환경 설정은 이 전용 client에 적용하지 않고 redirect도 따르지 않으므로 Bearer가 중간 proxy나 caller 제공 URL로 전달되지 않는다. 들어온 Bearer credential은 Patch Data가 검증하도록 전달하고 Grant는 JWT를 decode하거나 claim을 신뢰하지 않는다.

Patch Data 응답의 인증 계약은 PocketBase auth-refresh shape인 `{ "token": "...", "record": { "id", "email", "verified", ... } }`다. `record.verified`는 반드시 `true`여야 한다. `id`는 비어 있거나 공백·제어문자를 포함할 수 없다. email은 trim/lowercase normalization 후 유효한 `@` 주소여야 한다. 정상 인증에서는 64 KiB 전체 응답 한도 안의 `organizations` 필드를 중복·배열 cardinality·내부 형태와 무관하게 opaque compatibility data로 무시한다. master-key로 보호된 external-owner bootstrap에서만 정확히 한 `organizations` 배열을 요구하고, 최대 100개/각 255 bytes 이하의 trim/control 검증 및 dedupe 뒤 선택 조직 하나를 확인한다. missing/empty/duplicate/malformed relation은 bootstrap에서 fail closed한다. 정상 `/me`, application authz, Exposure authz, 조직 복구 요청은 `x-organization-id`를 caller scope로 받고 Grant의 strong local `organization_memberships`, 조직 상태, role/binding만 권위로 사용한다. 따라서 Patch Data relation의 추가/삭제/형태 변경은 정상 요청의 Grant 접근을 부여하거나 회수하지 못한다. Grant는 refresh token의 expiry/revocation을 연장하지 않도록 Patch Data identity를 cache하지 않고 매 요청 refresh한다. 응답은 64 KiB를 hard bound로 stream하며 Content-Length가 없거나 거짓이어도 즉시 중단한다. 401/403만 credential failure이고, redirect·다른 4xx/5xx·transport/timeout은 upstream detail을 노출하지 않는 unavailable(503) failure다; Rauthy cache key는 provider name을 SHA-256 input domain에 포함한다.

Patch Data application authorization에는 caller/token client id를 사용하지 않는다. `POST /api/v1/admin/bootstrap/external-owner`는 master key와 Patch Data relation에 들어 있는 명시적 조직 하나 아래 user/identity, organization, owner membership/binding, 설정된 `GRANT_PATCH_DATA_EXPOSURE_APPLICATION_ID`의 application을 한 트랜잭션에서 만든다. application은 caller가 선택할 수 없으며 선택 조직 하나에만 속하고, 초기 상태는 현재 별도 consent가 없는 서버 통합에 맞춰 `confidential/server`, `first_party=true`, `consent_policy=auto_approve`, `active`, `ready`, `rauthy_client_id=NULL`, `created_by_user_id=owner`다. application id/org/slug/type/creator는 immutable bootstrap identity이고, 이후 master trust 변경으로 `first_party/consent_policy`가 바뀌어도 동일 bootstrap retry는 성공하며 그 변경을 되돌리지 않는다. 다른 조직·owner·충돌하는 app id/slug는 zero-write conflict다. 저장소/쿼럼 장애는 conflict가 아니라 unavailable(503)이다. 기존 `/auth/v1/*` proxy와 `/api/v1/admin/bootstrap/rauthy-owner`는 Rauthy mode에만 남는다.

`POST /api/v1/me/materialize`는 caller가 고른 `x-organization-id`에서 voter node가 처리하되 Patch Data relation으로 소속을 승인하지 않는다. provider-qualified `(patch-data, record.id)`로만 local user/identity를 materialize하므로 email이나 bare id의 기존 local user를 연결하지 않는다. local `users.active = false`이면 403으로 거부한다. 성공 응답은 `userId`, `organizationId`, Grant local `membershipStatus`만 가진 정확히 3-field object이며, 이 호출은 membership, role, role binding을 만들거나 권한을 부여하지 않는다.

권한 원장은 master-key 보호 voter endpoint `POST /api/v1/admin/imports/patch-data-authority`로만 `grant.patch-data-authority.v1` 전체 snapshot을 적용한다. manifest는 정렬된 user/external identity, organization, membership, hierarchy edge, exact plant binding, 그리고 plant-read를 사용할 existing active/ready application의 `applicationAssignments`를 담는다. `ledgerDigest`는 원본 rule ledger의 lowercase SHA-256이고 `manifestDigest`는 digest 필드 자신과 CAS 전제만 제외한 versioned payload 전체를 length-framed canonical encoding으로 계산한다. exact generation과 두 digest를 재시도하면 동일 receipt를 반환하되 canonical row와 import provenance까지 다시 검증한다. 같은 generation의 다른 payload, 잘못된 `expectedPreviousGeneration`, native unowned row collision, 참조 오류, cycle은 모두 한 transaction의 assertion에서 zero-write로 실패한다.

importer는 absent row를 만들거나 `patch_data_authority_import_entities`가 같은 entity kind/id로 소유한 row만 갱신·교체한다. 이름과 내용이 우연히 같은 native user, organization, membership, plant binding, `application_actions` row도 암묵적으로 채택하지 않는다. 다음 generation은 이전 import 소유 membership, plant binding, application assignment만 먼저 회수한 뒤 새 snapshot을 원자 반영한다. omitted user/organization/identity는 identity history로 남을 수 있지만 active membership/binding이 없으므로 권한을 갖지 않는다. `applicationAssignments`는 application 자체를 만들거나 수정하지 않고, 검증된 application에 `patch-data.plant.read` action만 import-owned `application_actions`로 연결한다. 이 assignment를 다음 generation에서 빼면 import-owned action만 제거하며 native action은 절대 제거하지 않는다. membership, binding, application action은 Grant canonical evaluator가 직접 사용하고 정상 요청에서 Patch Data를 재조회하지 않는다. hierarchy는 검증·보존하지만 전파 의미가 확정되지 않아 evaluator 입력으로 사용하지 않는다.

producer는 digest encoding을 재구현하지 않고 repository-native tool을 사용한다. 입력 JSON은 API manifest와 같은 strict shape이며 계산 전 `manifestDigest`는 생략하거나 임의 값이어도 된다. 출력된 lowercase hex를 최종 `manifestDigest`에 넣은 뒤 같은 JSON을 import API로 전송한다.

```sh
cargo run --quiet -p grant --bin grant-patch-data-manifest-digest < manifest.json
```

Exposure의 plant 조회는 closed endpoint `POST /api/v1/exposure-authz/plant-read`를 사용한다. caller는 기존 replay tuple과 `operationId=plant.get`만 제공하고 Grant는 이를 고정적으로 provider `patch-data`, action `patch-data.plant.read`, resource type `plant`에 매핑한다. 응답은 operation/provider/action/resource type을 모두 echo하며 임의 selector는 strict JSON decode 또는 operation validation으로 거부한다.

`deploy/overlays/patch-data`는 IED-style Grant-only manifest다. 이 overlay는 `GRANT_AUTH_PROVIDER=patch-data`, 내부 `http://patch-data.patch.svc.cluster.local:8090`, 고정 application id `patch-data-exposure`을 설정하고 Rauthy workload와 Grant의 Rauthy secretKeyRef를 제거한다. `deploy/overlays/ied-patch-data`는 이 정확한 manifest를 compose하는 명시적 IED 선택 이름이다. 기존 `deploy/overlays/ied`는 legacy chaos namespace mode이므로 Patch Data 용도로 재사용하지 않는다. Grant master/app 및 Hiqlite secret은 별도로 제공해야 한다. base와 production overlay는 Rauthy deployment contract를 그대로 유지한다.

## 업데이트 규칙

아래가 바뀌면 이 문서와 [database-schema.md](./database-schema.md)를 함께 갱신한다.

- `role_bindings` scope 의미
- agreement 검증 규칙
- application grant action subset 규칙
- developer console application 설정 권한 규칙
- authz check/filter 응답 계약
- coarse/field capability 경계와 preserved field-work 상태
