# Grant Authorization Model

`grant`는 identity provider가 아니다. 로그인과 외부 identity 검증은 외부 인증 공급자가 맡고, `grant`는 검증된 identity를 내부 사용자에 연결해 조직, 애플리케이션, service provider, resource scope의 권한을 관리하고 판정한다.

## 인증 공급자 방향과 현재 지원 범위

인증 연동 대상은 **PocketBase, Rauthy, 개발 중인 Goauthy**다. 현재 `GRANT_AUTH_PROVIDER`는 배포당 `rauthy` 또는 `pocketbase` 하나를 선택한다. Goauthy adapter는 아직 없으며 프로토콜을 미리 가정하지 않는다.

| 대상 | 현재 구현 |
|---|---|
| PocketBase | 설정된 issuer origin 및 auth collection 하나의 auth-refresh를 호출한다. patch-auth의 users, viewers, temporary_accounts 중 선택할 수 있으며 요청자가 endpoint를 바꾸지 못한다. |
| Rauthy | 설정된 introspection endpoint에 client 인증과 토큰을 보내고, userinfo의 subject가 같은지 확인한다. 기본 경로는 `/auth/v1/oidc/introspect`, `/auth/v1/oidc/userinfo`다. 로컬 JWT 서명 검증이나 OIDC discovery 방식은 아니다. |
| Goauthy | 연동 목표다. 저장소·인증 인터페이스가 확인되면 토큰 검증 요청/응답, 안정적인 subject, 앱·audience 매핑, 만료·철회 동작을 정한다. OIDC 또는 PocketBase 호환이라고 미리 가정하지 않는다. |

인증 공급자와 권한 대상인 `service_providers`는 다른 개념이다. PocketBase identity는 issuer·collection·record를 모두 구분한다. 권한 catalog의 `patch-data` provider 이름은 유지하며 인증 공급자 이름과 연결하지 않는다. 외부 subject를 내부 사용자 ID로 추정하지 않는다.

일반 요청에서 조직 소속과 역할·리소스 권한은 Grant의 로컬 권한 데이터가 결정한다. 인증 성공 자체가 조직 가입이나 관리자 권한을 뜻하지 않는다. master 인증 owner bootstrap은 별도 관리 동작이다. `GRANT_POCKETBASE_APPLICATION_ID`는 검증된 토큰을 사용할 Grant 애플리케이션 설정이며 폐기된 Exposure 전용 API와 무관하다.

구현 근거: [공급자 설정](../internal/grant/config.go), [외부 토큰 검증](../internal/grant/auth_external.go), [내부 identity·조직 검증](../internal/grant/auth_current.go). `/auth/v1/*` 프록시와 앱 provisioning 확인은 현재 Rauthy 전용이다. Goauthy 추가를 이유로 이 기능을 다른 공급자에 자동 적용하지 않는다.

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

Exposure 전용 API 세 개와 고정 `plant.get` 계약은 폐기되어 404를 반환한다. provider별 HTTP 계약은 [범용 HTTP capability](http-capabilities.md)를 사용한다. 기존 소비자는 catalog·PEP binding·요청 evidence를 준비해야 하며 자동 호환 경로는 없다.
애플리케이션이 최종 사용자 토큰으로 판정을 요청할 때는 별도 API를 사용한다.

```text
POST /api/v1/application-authz/check
POST /api/v1/application-authz/batch-check
POST /api/v1/application-authz/filter
```

이 경로는 전용 `x-grant-app-auth-key`, Bearer token, `x-organization-id`를 모두 요구하고 strong consistency만 사용한다. `GRANT_APP_AUTH_KEY`는 필수이며 `GRANT_MASTER_KEY`와 다른 값이어야 한다. master key는 application-authz 인증에 사용할 수 없고 app-auth key도 admin 인증에 사용할 수 없다. subject와 application id는 body에서 받지 않는다. 서버가 검증된 토큰 `sub`와 `client_id`를 사용해 exact 사용자와 동일 조직의 active application을 결정한다. client mismatch, 모호한 매핑, 검증 정보가 없는 cache 또는 identity provider 장애에서 안전한 cache가 없으면 fail closed한다. 기존 `/api/v1/authz/*` master API는 `x-grant-master-key`로 명시한 subject를 검사하는 trusted-provider 계약으로 유지한다.

## PocketBase authentication mode

`GRANT_AUTH_PROVIDER=pocketbase`는 `GRANT_POCKETBASE_BASE_URL`의 신뢰된 origin과 `GRANT_POCKETBASE_COLLECTION`(기본 `users`)을 고정한다. 매 요청 `POST /api/collections/{collection}/auth-refresh`로 bearer를 검증하며 redirect와 토큰 캐시는 사용하지 않는다. 응답은 64 KiB로 제한한다. `record.collectionName`은 설정된 collection과 같아야 하며, 검증된 `collectionId`와 `id`를 사용한다. 한 Grant 인스턴스는 collection 하나를 선택하며 토큰 claim으로 다른 issuer나 collection을 탐색하지 않는다.

내부 identity key는 `(pocketbase, canonical issuer origin, collectionId, record.id)`다. 이메일은 verified일 때만 선택적으로 보관하며 이메일이나 raw ID로 다른 사용자를 연결하지 않는다. username 기반 viewers와 temporary_accounts도 설정된 collection으로 사용할 수 있다. 인증 성공은 조직 소속이나 권한을 만들지 않는다.

`GRANT_POCKETBASE_APPLICATION_ID`는 application authz에 사용할 서버 설정 application이다. HTTP capability evidence subject는 `pocketbase:<issuer>/<collectionId>/<recordId>`, audience는 이 application ID다. Exposure가 보내는 값은 힌트이며 Grant가 실제 auth-refresh 결과와 비교한다.

master-key 보호 `POST /api/v1/admin/bootstrap/external-owner`는 검증된 사용자와 명시한 `x-organization-id`에 owner membership/binding 및 설정 application을 원자 생성한다. authn의 organizations claim을 읽지 않는다. 정상 membership과 role/binding의 권위는 Grant다. `POST /api/v1/me/materialize`는 identity만 만들며 소속이나 권한을 부여하지 않는다.

권한 원장은 master-key 보호 voter endpoint `POST /api/v1/admin/imports/patch-data-authority`로만 `grant.patch-data-authority.v1` 전체 snapshot을 적용한다. manifest는 정렬된 user/external identity, organization, membership, hierarchy edge, exact plant binding, 그리고 plant-read를 사용할 existing active/ready application의 `applicationAssignments`를 담는다. `ledgerDigest`는 원본 rule ledger의 lowercase SHA-256이고 `manifestDigest`는 digest 필드 자신과 CAS 전제만 제외한 versioned payload 전체를 length-framed canonical encoding으로 계산한다. exact generation과 두 digest를 재시도하면 동일 receipt를 반환하되 canonical row와 import provenance까지 다시 검증한다. 같은 generation의 다른 payload, 잘못된 `expectedPreviousGeneration`, native unowned row collision, 참조 오류, cycle은 모두 한 transaction의 assertion에서 zero-write로 실패한다.

각 `users` 항목은 명시적인 `issuer`와 `collectionId`가 필수다. digest의 user frame 순서는 `user`, `userId`, `issuer`, `collectionId`, `externalUserId`다. importer는 인증 서버를 조회하거나 이메일로 사용자를 연결하지 않고, 신뢰된 master 호출자가 제공한 namespace와 내부 `userId`를 그대로 사용한다. 이는 버전 번호를 올리지 않은 의도적 breaking change다. 기존 raw-ID manifest 및 기존 schema checksum의 DB/backup은 호환되지 않으며 자동 재연결이나 데이터 삭제를 하지 않는다. 새 DB와 명시적으로 검증된 mapping이 필요하다.

importer는 absent row를 만들거나 `patch_data_authority_import_entities`가 같은 entity kind/id로 소유한 row만 갱신·교체한다. 이름과 내용이 우연히 같은 native user, organization, membership, plant binding, `application_actions` row도 암묵적으로 채택하지 않는다. 다음 generation은 이전 import 소유 membership, plant binding, application assignment만 먼저 회수한 뒤 새 snapshot을 원자 반영한다. omitted user/organization/identity는 identity history로 남을 수 있지만 active membership/binding이 없으므로 권한을 갖지 않는다. `applicationAssignments`는 application 자체를 만들거나 수정하지 않고, 검증된 application에 `patch-data.plant.read` action만 import-owned `application_actions`로 연결한다. 이 assignment를 다음 generation에서 빼면 import-owned action만 제거하며 native action은 절대 제거하지 않는다. membership, binding, application action은 Grant canonical evaluator가 직접 사용하고 정상 요청에서 Patch Data를 재조회하지 않는다. hierarchy는 검증·보존하지만 전파 의미가 확정되지 않아 evaluator 입력으로 사용하지 않는다.

producer는 digest encoding을 재구현하지 않고 repository-native tool을 사용한다. 입력 JSON은 API manifest와 같은 strict shape이며 계산 전 `manifestDigest`는 생략하거나 임의 값이어도 된다. 출력된 lowercase hex를 최종 `manifestDigest`에 넣은 뒤 같은 JSON을 import API로 전송한다.

```sh
go run ./cmd/grant-patch-data-manifest-digest < manifest.json
```

V2 authority snapshot 수신 API와 exporter는 제거됐다. V1 manifest import와
`/usr/local/bin/grant-patch-data-manifest-digest` 도구는 유지한다. 서버 entrypoint는
`/usr/local/bin/grant`다.


`deploy/overlays/patch-data`는 IED-style Grant-only manifest다. 이 overlay는 `GRANT_AUTH_PROVIDER=pocketbase`, 내부 `http://patch-auth.patch.svc.cluster.local:8090`, 고정 application id `patch-data-exposure`을 설정하고 Rauthy workload와 Grant의 Rauthy secretKeyRef를 제거한다. `deploy/overlays/ied-patch-data`는 이 정확한 manifest를 compose하는 명시적 IED 선택 이름이다. 기존 `deploy/overlays/ied`는 legacy chaos namespace mode이므로 Patch Data 용도로 재사용하지 않는다. Grant master/app 및 Hiqlite secret은 별도로 제공해야 한다. base와 production overlay는 Rauthy deployment contract를 그대로 유지한다.

## 업데이트 규칙

아래가 바뀌면 이 문서와 [database-schema.md](./database-schema.md)를 함께 갱신한다.

- `role_bindings` scope 의미
- agreement 검증 규칙
- application grant action subset 규칙
- developer console application 설정 권한 규칙
- authz check/filter 응답 계약
- coarse/field capability 경계와 preserved field-work 상태
