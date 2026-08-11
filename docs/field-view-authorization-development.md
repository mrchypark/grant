# Grant 범용 필드 열람 권한 개발 지침

> Historical preservation notice (2026-07-30): 이 문서는 기존 field/column authorization 설계와
> 구현 이력을 보존한다. 현재 simple three-service deploy baseline에는 포함되지 않는다. 현재 Grant는
> binary coarse authorization만 제공하며 `resource.view`와 `visibilityGranting=true`를 fail closed하고,
> 아래 field policy management route와 `fieldViews` response contract를 등록하지 않는다. 이 설계를 다시
> 활성화하려면 verified requirement, migrated-state 계획, consumer enforcement, 별도 배포 승인이 필요하다.

상태: V1 구현 지침

대상: Grant 개발자, 외부 소비자 연동 개발자, 권한 정책 운영자

연계 구현 예시: Compos `docs/specs/field-view-authorization-development.md`

## 1. 문서 소유권과 규범

이 문서의 `MUST`, `MUST NOT`, `SHOULD`, `SHOULD NOT`, `MAY`는 구현 요구사항이다.

이 문서는 다음 Grant 계약의 정본이다.

- provider-neutral 필드 열람 정책 DB schema
- user/group/organization role 결과 병합
- 관리자 API
- authz check와 batch-check의 exact wire
- learner consistency와 실패 의미
- migration, audit, limits, acceptance tests

소비자 연계 문서는 이 wire를 재정의하면 안 된다. 소비자 문서는 Grant 결과를 실제 조회와 응답 projection에 적용하는 방법, 소비자 고유의 식별자와 exact tuple, 익명화 구현을 정의한다.

## 2. V1 목표와 비목표

Grant는 기존 exact resource RBAC를 통과한 사용자에게 요청된 각 필드의 열람 결과를 반환한다.

wire mode는 정확히 다음 세 값이다.

| mode | 의미 |
|---|---|
| `real` | 실제 값을 열람할 수 있다. |
| `anonymized` | 소비자가 관리하는 canonical profile로 변환한 값만 열람할 수 있다. |
| `none` | 해당 필드를 열람할 권한이 없다. |

V1 범위는 다음뿐이다.

1. user subject에서 user/group/organization role binding을 해석한다.
2. 기존 `provider + resource type + resource id + action` exact RBAC를 적용한다.
3. 허용된 action에 field projection 결정을 추가한다.
4. matching role의 mode를 병합한다.
5. trusted/application check와 batch-check에 같은 결과를 제공한다.

field-aware 요청은 기존 exact tuple에 다음 view를 추가한다.

```text
provider + action + resource.type + resource.id
view.schemaKey + view.fields[]
```

`provider`, `action`, `resource.type`, `resource.id`의 의미와 `schemaKey`를 만드는 방법은 소비자 계약이 소유한다. field view를 적용할 action은 기존 action catalog에서 `visibility_granting=true`로 선언한다. Grant 코어는 특정 provider 이름, action 이름, object 모델 또는 field catalog를 기능 이름이나 범용 저장 계약에 고정하면 안 된다.

다음은 Grant V1 구현의 명시적 비목표 또는 후속 범위다.

- owner, state, payload 조건
- field write/update 권한
- query predicate, search, sort, aggregate, group-by 권한
- SQL column privilege와 database row-level security
- generic ABAC 또는 범용 정책 언어
- explicit deny, role 상속, resource hierarchy, wildcard grant
- Grant가 소비자 value를 읽거나 익명화하는 기능
- Grant 내부에서 소비자 query, write 또는 response projection을 실행하는 기능
- Grant가 소비자 field catalog의 정본을 복제하는 기능
- `/authz/filter` field projection
- 전체교체 update 또는 다른 mutation의 쓰기 안전성 판정

owner/state/payload 조건과 query/write projection은 Grant V1 completion requirement가 아니다.

단, field가 제한된 사용자가 전체교체 update를 수행할 수 있는 소비자는 숨긴 값 overwrite를 막는 안전 장치를 해당 소비자 V1 완료 요구사항으로 구현해야 한다. Grant V1이 그 gate를 직접 구현하지 않는다는 말은 소비자가 이를 생략할 수 있다는 뜻이 아니다. 기존 write authorization과 별도의 read-real gate 또는 동등한 보존 전략이 필요하다. field view는 읽기 증거일 뿐이며 어떤 read tuple이나 `real` 결정도 write allow로 해석하면 안 된다.

## 3. 범용 식별자와 mode 계약

### 3.1 Exact tuple과 schema key

Grant의 기존 RBAC tuple은 exact match다.

```text
provider + action + resource.type + resource.id
```

`schemaKey`는 한 소비자 안에서 field pointer 공간을 변경 불가능하게 식별하는 opaque string이어야 한다. 표시 이름, mutable slug, locale label 또는 alias를 사용하면 안 된다. 예를 들어 schema-backed 소비자는 immutable schema/type ID를 사용할 수 있다.

Grant는 `resource.type`, `resource.id`, `schemaKey`를 서로 독립된 opaque exact string으로 취급한다. 외부 schema/resource 존재 여부, 세 식별자 사이의 의미적 관계, object와 schema의 대응 관계, workspace ownership은 admin write와 authz 평가 어느 쪽에서도 검증하지 않는다. 소비자는 현재 resource에 맞는 식별자를 계산하고 관계를 검증한 뒤 전달할 책임이 있다.

field-aware admin의 path `actionId`와 authz body의 `action`은 각각 `actions.id`로 exact 조회한다. admin은 조회된 action이 `visibility_granting=true`인지 검증하고, authz 평가는 추가로 action의 `provider_id == resource.provider`인지 검증한다. `actions.code`는 provider catalog의 별도 속성이므로 `action.code == action.id`를 요구하거나 code를 action ID 대신 비교하면 안 된다.

소비자 고유 계약이 schema resource의 `resource.id == view.schemaKey` 같은 제약을 요구한다면 소비자 adapter가 Grant를 호출하기 전에 검증한다. Grant는 admin policy write에 존재하지 않는 binding `resourceId`를 추측하거나 외부 object-to-schema mapping을 조회하지 않으며, field-aware authz 평가에서도 이 equality를 검증하지 않는다.

### 3.2 Field pointer

`fields[]` entry와 `fieldViews` key는 normalized RFC 6901 JSON Pointer다.

```text
/title
/values/email
/values/customer/name
```

V1은 composite `field_id`를 도입하지 않는다. field identity는 요청의 `schemaKey`와 개별 pointer로 해석한다.

정규화 규칙:

1. pointer는 `/`로 시작해야 하며 빈 root pointer는 허용하지 않는다.
2. token의 `~`와 `/`는 각각 `~0`, `~1`로 escape한다.
3. invalid `~` escape는 `400 Bad Request`다.
4. server는 decode 후 RFC 6901 canonical form으로 다시 encode한다.
5. canonical duplicate가 생기면 요청 전체를 400으로 거부한다.
6. 비교는 case-sensitive exact match다.
7. `*`는 RFC 6901에서 특별한 문자가 아니다. `/values/*`는 이름이 `*`인 literal key를 가리키는 exact pointer로 허용한다.

Grant는 pointer를 wildcard, glob, prefix 또는 schema-wide sentinel로 해석하거나 확장하면 안 된다. literal `*` segment의 허용과 wildcard grant 금지는 서로 다른 계약이다.

### 3.3 Mode 저장과 merge

DB에는 positive `real | anonymized`만 저장한다.

```text
positive row 없음 = none
none < anonymized < real
```

`none`은 explicit deny가 아니다. 여러 matching role 중 가장 높은 mode가 이긴다. 제한적인 role 하나가 다른 role의 `real`을 취소하면 안 된다.

새 pointer는 명시적으로 grant되기 전까지 `none`이다. wildcard, prefix, glob 또는 schema-wide 자동 포함은 금지한다.

### 3.4 Fixed envelope

Grant 코어는 소비자 response envelope를 알지 못하며 어떤 pointer도 자동 노출하지 않는다. row-level allow만으로 field view 밖에 노출할 수 있는 correlation/routing envelope는 소비자 계약이 최소 집합으로 정의한다.

소비자가 fixed envelope로 정한 field는 자신의 `fields[]`와 정책 운영 대상에서 제외할 수 있다. 그 제외 목록은 provider-neutral Grant 코어의 상수나 validation rule이 아니라 소비자 adapter/계약의 일부다. Grant admin API와 evaluator는 `/id`를 포함한 모든 valid canonical pointer를 허용한다. 미래 속성은 fixed envelope에 자동 추가하면 안 되며 explicit positive grant 전까지 `none`으로 취급한다.

## 4. 책임 경계

### 4.1 Grant

Grant는 다음을 MUST 수행한다.

- trusted user/org를 기존 RBAC로 검증한다.
- direct user, group, organization binding의 matching role을 계산한다.
- requested pointer마다 effective mode를 반환한다.
- field-aware 요청의 exact 식별자 형식과 catalog의 provider ownership/`visibility_granting` capability를 검증한다.
- field-aware 요청을 strong consistency로 평가한다.
- 요청한 모든 pointer를 정확히 한 번 반환한다.
- corrupt DB, unknown mode, incomplete result를 전체 call 실패로 처리한다.

Grant는 외부 value, anonymization/masking profile, renderer, salt, key 또는 변환 결과를 받거나 저장하면 안 된다.

### 4.2 소비자

소비자는 다음을 MUST 수행한다.

- principal을 Grant 내부 `users.id`로 매핑한다.
- resource와 schema의 immutable canonical 식별자를 계산한다.
- 출력하려는 모든 보호 필드를 normalized pointer로 요청한다.
- fixed envelope를 최소화하고 나머지 field를 모두 decision 대상으로 만든다.
- `real`에는 원본 값, `anonymized`에는 관리자 관리형 canonical profile 결과만 사용한다.
- explicit하고 complete한 `none`만 field omission으로 처리한다.
- missing, duplicate, extra, malformed decision 또는 transform 실패에서 보호 요청 전체를 실패시킨다.
- field 제한 상태의 mutation이 숨긴 값을 손상하거나 우회 노출하지 않도록 별도의 write safety를 집행한다.

anonymization profile은 소비자 관리자 control plane의 canonical 정의다. user-authored schema나 request body가 profile을 만들거나 바꾸면 안 된다.

### 4.3 Compos 연계 예시

Compos는 이 범용 기능의 소비자 예시이며 Grant 코어 계약의 이름이나 범위를 결정하지 않는다. Compos adapter의 exact tuple은 다음과 같다.

| 용도 | action | resource type | resource id | schema key |
|---|---|---|---|---|
| object instance read/list projection | `compos.object.read` | `compos.object` | `<object_id>` | `<object_type_id>` |
| object type read schema | `compos.object_type.schema.read` | `compos.object_type` | `<object_type_id>` | `<object_type_id>` |
| object type command surface | `compos.object_type.command.read` | `compos.object_type` | `<object_type_id>` | `<object_type_id>` |

Compos adapter는 schema/command field-aware 요청에서 `resource.id == view.schemaKey`인지 Grant 호출 전에 검증한다. instance read의 object-to-type mapping도 Compos가 검증하고 immutable `object_type_id`를 `schemaKey`로 보낸다. Grant는 두 경우 모두 식별자 관계를 검증하지 않는다.

Compos row allow fixed envelope는 `/id` 하나다. Compos adapter와 정책 운영은 `/id`를 `fields[]`와 실제 policy scope에서 제외하지만 Grant 코어는 `/id` pointer를 정상 입력으로 허용한다. 현재 나머지 object response field와 각 `/values/...` leaf는 모두 field decision 대상이다. Compos의 provider-neutral projection, masking profile, read-real update gate는 연계 문서가 소유한다.

## 5. 현재 Grant 기준선

기존 코드는 다음 기반을 제공한다.

- `role_bindings`의 exact `provider_id + resource_type + resource_id` scope
- HTTP authz의 `user` subject
- active membership을 통한 organization/group/direct user candidate
- active user/org/provider/binding과 agreement kill switch
- application action과 consent/trust 교집합
- learner shared `/authz/check`, `/batch-check`, `/filter`
- 최대 1,000개 batch/filter item

외부 resource 존재와 tenant/workspace ownership은 Grant가 검증하지 않는다. field view 없는 기존 요청의 binary `allowed` 결과와 response shape는 유지해야 한다.

## 6. DB schema

field view는 role-action에 연결한다.

- action은 provider 전역 작업 정의라 role별 projection을 표현하지 못한다.
- binding은 subject와 exact resource scope를 선택한다.
- role-action field view는 같은 action의 role별 projection을 표현한다.

```sql
CREATE TABLE IF NOT EXISTS role_action_field_views (
    role_id TEXT NOT NULL,
    action_id TEXT NOT NULL,
    resource_type TEXT NOT NULL,
    schema_key TEXT NOT NULL,
    field_pointer TEXT NOT NULL,
    view_mode TEXT NOT NULL
        CONSTRAINT role_action_field_view_mode
        CHECK (view_mode IN ('real', 'anonymized')),
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    PRIMARY KEY (
        role_id,
        action_id,
        resource_type,
        schema_key,
        field_pointer
    )
);

CREATE INDEX IF NOT EXISTS idx_role_action_field_views_lookup
    ON role_action_field_views (
        role_id,
        action_id,
        resource_type,
        schema_key
    );

CREATE TABLE IF NOT EXISTS field_view_policy_audit_outbox (
    event_id TEXT PRIMARY KEY,
    idempotency_key TEXT NOT NULL UNIQUE,
    request_hash TEXT NOT NULL,
    organization_id TEXT NOT NULL,
    actor_user_id TEXT NOT NULL,
    role_id TEXT NOT NULL,
    action_id TEXT NOT NULL,
    resource_type TEXT NOT NULL,
    schema_key TEXT NOT NULL,
    before_json TEXT NOT NULL,
    after_json TEXT NOT NULL,
    created_at TEXT NOT NULL
);
```

복합 PK는 저장 uniqueness일 뿐 composite field identity가 아니다.

write path는 strong read 또는 같은 transaction에서 다음을 검증한다.

1. `(role_id, action_id)`가 `role_actions`에 존재한다.
2. role이 path organization 소유다.
3. action이 존재하며 해당 role에 부여되고 `visibility_granting=true`인 action이다.
4. `resourceType`과 `schemaKey`가 비어 있지 않고 byte limit 이하다.
5. pointer가 normalized RFC 6901이고 byte limit 이하다. `/id`도 valid pointer로 허용한다.
6. 저장 mode가 `real | anonymized`다.

admin endpoint에는 binding `resourceId`가 없으므로 write path가 이를 검증하면 안 된다. Grant가 알지 못하는 external resource-to-schema mapping도 admin write 조건으로 두면 안 된다. action capability는 `actions.id`와 `visibility_granting`으로 검증하며 `actions.code == actions.id`를 요구하지 않는다. `resource.type`, `resource.id`, `schemaKey` 사이의 소비자 고유 관계는 Grant가 admin write나 authz 평가에서 검증하지 않는다.

Grant는 schema/pointer가 외부에 실제 존재하는지 검증하지 않는다. stale row는 그 pointer가 요청되지 않으면 결과에 참여하지 않는다.

## 7. Role resolution

기존 exact SQL을 통과한 candidate만 field merge에 참여한다.

```text
active user
AND active subject organization
AND active provider
AND active organization membership
AND active exact resource binding
AND role contains requested action
```

direct user, group, organization candidate는 동일한 입력이다.

각 pointer의 결과는 matching role positive grant 중 `none < anonymized < real` 최댓값이다.

- `allowed=false`: 요청 pointer 전체를 `none`으로 반환한다.
- `allowed=true`: matching role을 병합한다.
- 모든 pointer가 `none`이어도 resource-level `allowed`를 false로 바꾸지 않는다.

application-authz는 기존 `RBAC ∩ application action ∩ consent/trust`를 먼저 만족해야 한다. application trust와 consent는 field mode를 확대하지 않는다.

## 8. 관리자 API

### 8.1 Endpoint

```text
GET /api/v1/orgs/{organizationId}/roles/{roleId}/actions/{actionId}/field-views
PUT /api/v1/orgs/{organizationId}/roles/{roleId}/actions/{actionId}/field-views
```

- GET query는 `resourceType`, `schemaKey`를 필수로 받는다.
- PUT은 한 `resourceType + schemaKey` scope를 full replace한다.
- 두 경로는 `organization.permission.manage`로 보호한다.
- PUT은 voter-only strong write다.
- learner PUT은 `requires_voter_node`로 거부한다.
- PUT은 non-empty `Idempotency-Key` header를 필수로 받는다.

### 8.2 PUT

```json
{
  "resourceType": "records.customer",
  "schemaKey": "customer-schema-019f",
  "fieldViews": {
    "/values/name": "real",
    "/values/email": "anonymized",
    "/values/internal_note": "none"
  }
}
```

처리 규칙:

1. key는 normalized pointer다.
2. `real`, `anonymized`는 upsert한다.
3. `none`과 요청에서 빠진 기존 pointer는 삭제한다.
4. 빈 map은 해당 scope의 positive row 전체를 삭제한다.
5. action/resourceType의 외부 의미나 object-to-schema mapping을 추론하지 않는다.
6. `/id`를 포함한 valid canonical pointer를 provider-neutral하게 허용한다.
7. validate, delete, upsert와 append-only audit/outbox insert는 같은 Grant DB transaction이어야 한다.

idempotency 의미:

- canonical path/body hash와 `Idempotency-Key`를 audit/outbox row에 저장한다.
- 같은 key와 같은 hash의 retry는 기존 `after_json` 결과를 반환하고 새 mutation/event를 만들지 않는다.
- 같은 key와 다른 hash는 `409 Conflict`다.
- transaction commit 결과를 알 수 없는 caller는 같은 key로 retry해야 한다.
- DB transaction이 실패하면 policy와 audit/outbox가 모두 없어야 한다.

GET과 PUT response는 저장된 positive row만 pointer lexical order로 반환한다.

```json
{
  "roleId": "customer-support",
  "actionId": "records.customer.read",
  "resourceType": "records.customer",
  "schemaKey": "customer-schema-019f",
  "fieldViews": {
    "/values/email": "anonymized",
    "/values/name": "real"
  },
  "updatedAt": "2026-07-14T12:00:00Z"
}
```

`updatedAt` 의미는 다음과 같다.

- 해당 scope가 한 번도 PUT되지 않았고 positive row도 없으면 GET은 `fieldViews: {}`와 `updatedAt: null`을 반환한다.
- non-empty PUT 뒤에는 positive row의 가장 최신 `updated_at`을 반환한다.
- empty full-replace PUT으로 positive row가 모두 삭제된 뒤에는 같은 scope의 audit/outbox `created_at`을 fallback timestamp로 반환한다.
- 같은 idempotency key/hash retry는 최초 commit의 동일한 `updatedAt`에 수렴한다.

관리 GET에 없는 pointer는 effective `none`이다.

role action 또는 provider action 제거 시 관련 field view를 `role_actions`보다 먼저 같은 transaction에서 삭제한다. 유지되는 role-action policy는 보존한다.

## 9. Trusted check wire

check와 batch-check resource에 optional camelCase `view`를 추가한다.

```json
{
  "provider": "records",
  "type": "records.customer",
  "id": "customer_1",
  "view": {
    "schemaKey": "customer-schema-019f",
    "fields": ["/values/name", "/values/email"]
  }
}
```

`view`가 없으면 기존 binary authz다. `view`가 있으면 schema key와 1개 이상의 unique canonical pointer가 필요하다.

canonical resource wire는 다음과 같다.

```json
{
  "action": "records.customer.read",
  "resource": {
    "provider": "records",
    "type": "records.customer",
    "id": "customer_1",
    "view": {
      "schemaKey": "customer-schema-019f",
      "fields": ["/title", "/values/email"]
    }
  }
}
```

Grant는 `action`을 `actions.id`로 조회하고 catalog의 `provider_id == resource.provider`, `visibility_granting=true`를 확인한 뒤 provider/action/resource tuple을 기존 RBAC와 exact match한다. `actions.code`는 이 비교에 참여하지 않는다. `resource.type`, `resource.id`, `view.schemaKey`는 opaque exact 값이며 Grant는 이들 사이의 관계를 검증하지 않는다. 예를 들어 Compos schema/command equality와 instance object-to-schema mapping은 Compos adapter가 Grant 호출 전에 검증한다.

### 9.1 Check request/response

```http
POST /api/v1/authz/check?consistency=strong
x-grant-master-key: ...
content-type: application/json
```

```json
{
  "subject": {
    "type": "user",
    "id": "grant-user-1",
    "org_id": "grant-org-1"
  },
  "action": "records.customer.read",
  "resource": {
    "provider": "records",
    "type": "records.customer",
    "id": "customer_1",
    "view": {
      "schemaKey": "customer-schema-019f",
      "fields": ["/values/name", "/values/email", "/values/internal_note"]
    }
  }
}
```

허용 응답:

```json
{
  "allowed": true,
  "reason": "role_binding",
  "fieldViews": {
    "/values/name": "real",
    "/values/email": "anonymized",
    "/values/internal_note": "none"
  }
}
```

resource-level deny도 complete result를 반환한다.

```json
{
  "allowed": false,
  "reason": "no_matching_binding",
  "fieldViews": {
    "/values/name": "none",
    "/values/email": "none",
    "/values/internal_note": "none"
  }
}
```

`view`가 없으면 `fieldViews`를 생략한다. V1은 `decisionId`, `policyRevision`, `resourceVersion`을 요구하지 않는다.

### 9.2 Batch-check

`POST /api/v1/authz/batch-check?consistency=strong`의 각 check resource가 독립적인 `view`를 가진다.

응답은 입력 순서와 길이를 보존한다. 각 result의 `fieldViews` key set은 해당 check의 requested pointer set과 정확히 같아야 한다. item 하나라도 불완전하면 batch 전체를 non-2xx로 실패시킨다.

## 10. Filter와 list

`POST /api/v1/authz/filter`는 V1에서 row-access-only다.

- filter resource에 `view`가 있으면 400으로 거부한다.
- response는 기존 `resourceIds`만 반환한다.
- `fieldViews`, `resourceViews` 또는 유사 결과를 추가하면 안 된다.

소비자 list 순서는 다음과 같다.

1. 소비자가 후보 resource를 만든다.
2. filter로 row 접근 가능한 ID만 남긴다.
3. 허용된 resource를 field-aware batch-check에 넣는다.
4. batch 전체를 검증한 후 projection을 실행한다.

filter와 batch 사이에 RBAC가 바뀔 수 있으므로 최종 batch가 false 또는 실패하면 보호 list 요청 전체를 실패시킨다. filter 결과만으로 field를 직렬화하면 안 된다.

## 11. Application authz wire

V1 field view는 다음 경로만 지원한다.

```text
POST /api/v1/application-authz/check
POST /api/v1/application-authz/batch-check
```

`/api/v1/application-authz/filter`는 row-access-only다. resource에 `view`가 있으면 400으로 거부하며 field result를 반환하지 않는다.

application-authz는 기존 `x-grant-app-auth-key`, bearer token, `x-organization-id`를 요구한다. body에 subject/application ID를 추가하지 않는다. check body는 trusted check와 같은 resource `view`를 사용한다.

response는 trusted check와 같은 `fieldViews`를 사용하며 reason은 기존 `explicit_consent` 또는 `trusted_application`이다. batch도 동일한 순서, 완전성, 전체 실패 규칙을 사용한다.

## 12. Subject mapping과 caller trust

trusted `/api/v1/authz/*`를 사용하는 소비자는 다음을 지킨다.

- `subject.id`는 Grant 내부 `users.id`다.
- 외부 도메인 object/person ID를 subject로 사용하면 안 된다.
- `subject.org_id`는 active Grant organization membership과 일치해야 한다.
- tenant/workspace-to-organization mapping은 소비자 server가 결정하며 end-user body를 신뢰하면 안 된다.
- `subject.type`은 `user`다. group/org role은 Grant가 전개한다.

현재 master key는 admin에도 쓰이는 전역 secret이다. 소비자 network는 Grant learner/read service에만 접근하고 voter/admin service를 직접 호출하면 안 된다.

bearer와 application consent가 필요한 호출만 application-authz를 사용한다. 두 credential mode를 혼합하면 안 된다.

## 13. Consistency와 learner

- field-aware trusted check/batch는 `ReadConsistency::Strong`을 강제한다.
- caller가 query를 생략해도 local로 downgrade하면 안 된다.
- strong 실패 시 local learner로 fallback하면 안 된다.
- application check/batch는 현재처럼 항상 strong이다.
- learner가 healthy해도 quorum strong-read가 unavailable이면 non-2xx다.
- `view` 없는 기존 binary authz consistency 계약은 유지한다.

## 14. 평가 구현

현재 `crates/infra/src/database/authz.rs`의 active user/provider/org/membership과 exact binding SQL을 유지한다. field-aware check는 boolean 집계 전에 matching `role_id + action_id`를 보존한다.

field view lookup key:

```text
candidate.role_id
candidate.action_id
request.resource.type
request.view.schemaKey
requested field pointer
```

Rust service는 다음을 수행한다.

1. `actions.id`로 action을 조회하고 provider ownership과 `visibility_granting=true`를 검증한다.
2. `resource.type`, `resource.id`, `schemaKey`를 opaque exact 값으로 유지한다.
3. exact RBAC candidate를 계산한다.
4. application 경로이면 기존 app action과 consent/trust를 적용한다.
5. pointer별 positive row를 조회한다.
6. `none < anonymized < real`로 병합한다.
7. absence에 `none`을 채운다.
8. requested pointer set과 동일한 response를 만든다.

PK와 `idx_role_action_field_views_lookup`이면 충분하다. V1은 field value를 평가하지 않으므로 JSON/FTS/view-mode index가 필요하지 않다.

## 15. 엄격한 실패 의미

Grant는 다음 상황에서 전체 call을 non-2xx로 실패시킨다.

- corrupt/unknown DB mode
- requested pointer 결과 누락 또는 extra result
- batch input/result cardinality 불일치
- pointer normalization과 response key 불일치
- DB query, strong read, row conversion, merge 실패
- 한 field-aware check의 candidate role이 256개를 초과함

부분 field 또는 부분 batch 결과를 반환하면 안 된다.

소비자는 다음을 전체 보호 요청 실패로 처리한다.

- timeout/non-2xx 또는 malformed JSON
- missing, extra, duplicate pointer result
- unknown mode 또는 batch 순서/길이 불일치
- anonymization transform 실패

오직 explicit하고 complete한 `none`만 field omission을 의미한다. 오류를 조용히 `none`으로 바꾸거나 일부 필드만 반환하면 안 된다.

admin mutation에서 audit/outbox insert가 실패하면 전체 transaction과 HTTP 요청이 실패한다. commit 결과가 불명확한 caller retry는 같은 `Idempotency-Key`를 사용해야 한다. 이미 commit된 outbox의 외부 relay 실패는 authz policy를 되돌리지 않으며 relay가 retry한다.

## 16. 제한

| 항목 | 제한 |
|---|---:|
| batch checks | 1,000 |
| filter resources | 1,000 |
| 한 resource의 requested pointers | 256 |
| 한 admin PUT entries | 512 |
| 한 field-aware check의 candidate roles | 256 |
| `schemaKey` byte length | 128 |
| normalized pointer byte length | 512 |
| `resourceType` byte length | 128 |

입력 크기 제한 초과와 invalid/duplicate pointer는 400이다. evaluator candidate role이 256개를 초과하면 configuration/capacity 오류로 취급해 partial allow 없이 field-aware call 전체를 503으로 실패시킨다. 같은 batch의 binary check 결과도 부분 반환하지 않는다.

## 17. Audit와 readiness

모든 admin PUT은 policy 변경과 같은 Grant DB transaction에 `field_view_policy_audit_outbox` append-only row를 반드시 기록한다.

```text
event = grant.field_view_policy.replaced
organization_id, actor_user_id, role_id, action_id
resource_type, schema_key
before/after positive grants
idempotency_key, request_hash, event_id, timestamp
```

DB audit/outbox insert가 실패하면 policy mutation도 rollback한다. request handler가 external sink에 직접 쓰고 이를 DB mutation과 원자적이라고 간주하면 안 된다.

외부 durable sink가 필요한 배포는 committed outbox를 relay한다.

- relay는 `event_id`를 sink idempotency key로 사용해 at-least-once 전송한다.
- sink timeout 또는 실패 시 DB policy를 되돌리지 않고 같은 event를 retry한다.
- relay cursor/delivery state가 필요하면 append-only audit row와 분리한다.
- 재시작 뒤 미전송 row를 다시 발견할 수 있어야 한다.
- actual value, secret, bearer, profile 내용과 transform 결과는 audit/outbox에 기록하지 않는다.

readiness는 배포가 선택한 mandatory component만 검증한다.

- `role_action_field_views`는 V1 필수 schema다.
- `field_view_policy_audit_outbox`도 V1 필수 schema다.
- external sink availability의 readiness 포함 여부는 운영 계약으로 정한다.

decision metric은 endpoint, allowed, reason, pointer/mode count, consistency, latency만 기록해야 한다.

## 18. Migration, rollout, rollback

### 18.1 Migration과 backfill

두 필수 table은 빈 상태로 추가한다. 기존 `view` 없는 binary authz는 변하지 않고, field-aware 요청은 모든 pointer가 `none`이다.

기존 실제 값 출력을 보존할 role만 소비자별 immutable schema key와 pointer를 명시적으로 열거해 `real`로 backfill한다. backfill은 dry-run, idempotent upsert, strong verification을 제공하며 wildcard나 새 pointer 자동 포함을 금지한다.

schema 구현과 동시에 다음 문서를 갱신한다.

- `docs/database-schema.md`
- `docs/authorization-model.md`
- `docs/architecture.md`
- `docs/local-development.md`

### 18.2 Rollout

1. voter에 두 필수 table, backward-compatible schema와 lifecycle cleanup을 배포한다.
2. learner readiness에서 schema 복제를 확인한다.
3. admin API와 provider-neutral field-aware check/batch를 배포한다.
4. 승인 정책을 explicit backfill한다.
5. 각 소비자 adapter를 shadow 검증한다.
6. mapping/pointer/profile 오류를 수정한다.
7. 소비자 enforcement를 활성화한다.
8. strong-read failure, revoke, idempotent mutation과 outbox relay retry drill을 수행한다.

### 18.3 Rollback

소비자가 enforcement를 활성화한 이후 rollback은 다음 중 하나여야 한다.

- 이전의 검증된 field-aware version
- 보호 read endpoint의 fail-closed maintenance 상태

enforcement를 끄고 field-unaware serialization으로 돌아가면 안 된다. field-unaware version은 maintenance gate가 먼저 실제 값 노출을 차단한 경우에만 배포할 수 있다.

## 19. 코드 변경 지점

### Domain/DB

- `crates/domain/src/lib.rs`: `FieldViewMode`, pointer normalization과 공통 limit
- `crates/infra/src/database/schema.rs`: policy/audit-outbox table, index, readiness
- `crates/infra/src/database/rows.rs`: positive row conversion과 unknown-mode error
- `crates/infra/src/database/roles.rs`: admin replace/list, audit-outbox atomic insert와 cleanup
- `crates/infra/src/database/catalog.rs`: provider action cleanup
- `crates/infra/src/database/authz.rs`: matching role과 bulk field view 조회
- `crates/infra/src/database/mod.rs`: field-aware check/result 구조와 candidate role limit

### Service/API

- `crates/infra/src/service.rs`: 범용 scope/pointer validation, role merge, idempotent admin auth, strong read
- `crates/api/src/dto.rs`: optional `view`, response `fieldViews`, admin DTO
- `crates/api/src/mappers.rs`: RFC 6901 normalization과 mapping
- `crates/api/src/authz.rs`: trusted/application check와 batch만 확장
- `crates/api/src/handlers.rs`: admin GET/PUT
- `crates/api/src/lib.rs`: voter write와 learner read route

provider-specific tuple 관계와 fixed-envelope validation은 Grant 코어/registry가 아니라 소비자 codebase의 adapter 경계에 격리한다.

## 20. Acceptance tests

### Identity/storage

- opaque immutable consumer schema key가 `schemaKey`로 round-trip한다.
- valid RFC 6901 pointer가 round-trip한다.
- invalid escape/root/relative/normalized duplicate를 400으로 거부한다.
- literal `*` segment가 exact pointer로 round-trip하고 wildcard/glob/prefix 확장은 일어나지 않는다.
- composite field ID가 추가되지 않는다.
- DB는 `real | anonymized`만 저장하고 absence는 `none`이다.
- admin `none` 입력은 row 삭제다.
- 새 pointer는 `none`이며 schema-wide wildcard grant는 존재하지 않는다.
- Grant는 `/id`를 포함한 valid pointer를 허용하고 소비자 adapter만 자신의 fixed-envelope pointer를 요청/정책 대상에서 제외한다.
- never-written empty scope GET은 `updatedAt: null`이다.
- empty full-replace PUT 뒤 GET과 idempotent retry는 audit/outbox timestamp를 반환한다.

### RBAC/merge

- direct user, group, organization binding이 적용된다.
- inactive user/org/provider/membership/binding은 grant를 만들지 않는다.
- exact resource가 다른 binding은 적용되지 않는다.
- `none < anonymized < real`이 모든 role 조합에서 성립한다.
- 한 role의 absence가 다른 positive grant를 취소하지 않는다.
- owner/state/payload 조건 없이 Grant V1 acceptance가 완료된다.
- 임의 provider의 `visibility_granting=true` action/resource type exact tuple이 provider-neutral하게 동작한다.
- `actions.id`와 `actions.code`가 달라도 ID 기준으로 동작하며 code equality를 요구하지 않는다.
- `visibility_granting=false` action은 field view admin/evaluation에 사용할 수 없다.
- admin write는 요청에 없는 binding resource ID나 external resource-to-schema mapping을 요구하지 않는다.
- Compos schema/command 예시는 Compos adapter가 Grant 호출 전에 resource ID와 schema key equality를 검증하며 Grant는 검증하지 않는다.

### Wire

- `view` 없는 check/batch는 기존 shape를 유지한다.
- check key set은 requested pointer set과 정확히 같다.
- deny는 complete `none` map을 반환한다.
- batch는 순서/길이를 보존하며 incomplete item은 전체 non-2xx다.
- corrupt DB mode는 전체 non-2xx다.
- 256 candidate roles는 허용되고 257 이상은 partial allow 없이 전체 503이다.
- V1은 decision/revision/resource version을 요구하지 않는다.
- 소비자 fixed envelope 외 모든 사용자 가시 field는 complete field decision을 요구한다.
- 미래 field는 explicit pointer grant 전까지 `none`이다.

### Filter/application

- filter의 `view`를 400으로 거부하고 `resourceIds`만 반환한다.
- list는 filter 후 field-aware batch를 호출한다.
- final batch 실패에서 보호 list 전체가 실패한다.
- application check/batch가 동일한 field result를 반환한다.
- application filter는 field result를 반환하지 않는다.
- consent/trust는 field mode를 확대하지 않는다.

### Failure/operations

- missing/extra/duplicate/malformed result는 소비자 전체 요청 실패다.
- explicit complete `none`만 omission이다.
- transform 실패는 전체 요청 실패다.
- field-aware learner read는 strong이며 local fallback이 없다.
- lifecycle delete 뒤 orphan policy가 없다.
- policy와 audit/outbox insert 중 하나가 실패하면 둘 다 rollback한다.
- 같은 idempotency key/hash retry는 한 event와 같은 response에 수렴하고 key 재사용/hash 변경은 409다.
- relay failure/restart가 event ID 기반 retry로 수렴하며 committed policy를 rollback하지 않는다.
- empty migration tables는 binary authz를 바꾸지 않는다.
- rollback은 field-aware version 또는 fail-closed maintenance만 허용한다.
- read tuple/field view를 전체교체 update allow로 재사용하지 않는 소비자 교차 계약 test가 존재한다.

## 21. 구현 체크리스트

### Phase 1: Schema/admin

- [ ] wire enum과 positive-only DB enum을 정의한다.
- [ ] policy와 mandatory audit/outbox table, index, lifecycle cleanup을 추가한다.
- [ ] audit outbox relay/readiness 경계를 정한다.
- [ ] idempotency key를 포함한 admin GET/PUT full-replace transaction을 구현한다.
- [ ] never-written/empty-replace `updatedAt` 의미를 구현한다.
- [ ] schema와 authorization 문서를 갱신한다.

### Phase 2: Evaluator/wire

- [ ] optional `view`와 `fieldViews` DTO를 추가한다.
- [ ] RFC 6901 normalization/duplicate rejection과 literal `*` exact pointer를 구현한다.
- [ ] `actions.id`, provider ownership, `visibility_granting` 기반 exact tuple을 구현하고 resource/schema 관계 검증을 Grant에 추가하지 않는다.
- [ ] precedence와 complete-decision validation을 구현한다.
- [ ] field-aware check/batch에 strong read를 강제한다.
- [ ] candidate role 256 boundary와 503 overflow를 구현한다.

### Phase 3: Filter/application

- [ ] trusted/application filter를 row-access-only로 유지한다.
- [ ] application check/batch에 동일 계약을 적용한다.
- [ ] filter-then-batch list integration test를 추가한다.

### Phase 4: Consumer rollout

- [ ] explicit backfill dry-run을 검증한다.
- [ ] 소비자별 immutable identifier, fixed envelope, anonymization profile을 확인한다.
- [ ] 전체교체 mutation 소비자의 read-real gate 또는 동등한 보존 전략을 검증한다.
- [ ] shadow validation 후 enforcement를 활성화한다.
- [ ] revoke/strong failure와 audit relay retry drill을 수행한다.
- [ ] field-aware rollback과 maintenance gate를 검증한다.

## 22. 최종 불변식

1. field view는 provider-neutral Grant 기능이며 consumer brand가 core API나 저장 이름을 결정하지 않는다.
2. `schemaKey`는 소비자 pointer 공간의 immutable opaque key다.
3. field identity는 normalized RFC 6901 pointer이며 composite field ID가 아니다.
4. literal `*` segment는 exact pointer로 허용하지만 wildcard/glob/prefix 해석과 확장은 금지한다.
5. wire mode는 `real | anonymized | none`이다.
6. DB는 positive `real | anonymized`만 저장하며 absence는 `none`이다.
7. mode는 `none < anonymized < real`로 병합하며 explicit deny는 없다.
8. 적용 action은 catalog의 `actions.id`, provider ownership과 `visibility_granting=true`로 판단하며 `actions.code == actions.id`를 요구하지 않는다.
9. admin write는 요청에 없는 resource ID나 외부 object-to-schema mapping을 검증하지 않는다.
10. consumer-specific resource/schema 관계는 소비자 adapter가 Grant 호출 전에 검증하며 Grant는 opaque 값 사이의 관계를 검증하지 않는다.
11. filter는 row-access-only이고 list는 filter 후 batch-check한다.
12. application field view는 check와 batch에만 존재한다.
13. corrupt, unknown, incomplete decision은 전체 call 실패다.
14. explicit complete `none`만 field omission이다.
15. anonymization profile과 실행은 소비자 admin control plane에 있다.
16. strong field view read는 local로 fallback하지 않는다.
17. 한 field-aware check의 candidate role limit은 256이며 초과는 partial allow 없는 503이다.
18. never-written empty scope는 `updatedAt: null`, empty PUT 이후 scope는 audit/outbox timestamp를 사용한다.
19. fixed envelope는 소비자별 최소 계약이며 새 field를 자동 포함하지 않는다.
20. admin PUT은 policy와 append-only audit/outbox를 같은 Grant DB transaction에 기록한다.
21. external audit sink는 committed outbox relay이며 DB mutation과 직접 원자화하지 않는다.
22. field view는 읽기 증거이며 write allow가 아니다.
23. 전체교체 mutation 안전성은 Grant V1 코어 범위 밖이어도 해당 소비자 V1 완료 책임이다.
