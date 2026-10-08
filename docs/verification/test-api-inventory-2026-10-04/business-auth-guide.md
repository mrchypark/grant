# 테스트 배포의 업무·인증 API 안내

작성일은 2026-10-04입니다. 이 문서는 발전소 업무 데이터와 로그인 기능이 **왜 필요한지**, 어떤 요청으로 사용하는지, 어떤 권한이 필요한지를 설명합니다. 관리 API 69개의 메서드·경로·필드·필요성은 [Grant 관리 상세 안내](grant-api-guide.md)에서 다룹니다. 전체 활성 계약은 [TSV](api-inventory.tsv)와 [JSON](api-inventory.json)을 참조합니다. 아래에서 기능의 필요성을 설명한 예시는 사용 시나리오이며 실제 고객 데이터나 실행된 요청이 아닙니다.

## 1. 현재 공개 범위와 근거를 읽는 방법

현재 공개 여부는 로컬 코드에 경로가 있다는 사실보다 **배포에서 가져온 활성 카탈로그**를 우선합니다. 런타임 담당자가 2026-10-04 20:37:03 KST에 IED 클러스터의 `patch-data-grant-exposure-go-v1` 네임스페이스에서 조회한 결과를 사용했습니다.

| 구분 | 활성 카탈로그 | 확인된 범위 | 해석 |
|---|---|---|---|
| 업무 데이터 | [catalog-data.json](runtime/catalog-data.json)의 `.catalog` | `patch-data`, generation **1**, 활성 경로 **369개**, 응답 리소스 **42개** | 오늘 배포에 등록된 경로입니다. 모든 경로에 오늘 쓰기 요청을 실행했다는 뜻은 아닙니다. |
| Grant 관리 | [catalog-management.json](runtime/catalog-management.json)의 `.catalog` | `grant-management`, generation **5**, 활성 경로 **69개** | 그룹 3개만 검증했던 과거 결과보다 현재 목록을 우선합니다. 상세 기능과 메서드·경로는 [Grant 관리 안내](grant-api-guide.md)와 통합 인벤토리를 참조합니다. |
| 직접 인증 | [auth-methods 관찰](runtime/patch-auth-auth-methods.json) | Patch Auth `users`: password·OTP·MFA enabled, OAuth2 flag true이나 provider 목록은 비어 있습니다. | 직접 네이티브 인증 설정 관찰입니다. Exposure 계정 프록시는 배포되지 않았고 별도 활성 계정 카탈로그는 확인되지 않았습니다. |

인증의 메서드·경로·필요성·조건·근거 수준은 [직접 네이티브 인증 경로표](native-auth-routes.tsv)에도 정리했습니다. 이 표의 **70행은 소스 조건부 경로까지 포함한 설명 행 수이며 활성 Exposure 경로 수가 아닙니다**. 현재 직접 인증에서 성공을 확인한 기능은 **users 로그인 방식 조회**이며, password·OTP·MFA 설정 확인을 해당 로그인 성공으로 확대하지 않습니다. OAuth2 flag만 켜져 있고 제공자 목록은 비어 있어 **현재 외부 OAuth 로그인 흐름은 구성되지 않은 상태로 안내합니다**. CSAP 훅 및 viewer·임시 계정의 기능은 소스 조건부 설명입니다. 표의 users 이외 경로와 파일 토큰·약관 관리·CSAP 추가 경로는 배포 성공 호출을 확인하지 않았고, 두 이미지 소스 후보 전체의 라우트 동일성도 검증하지 않았습니다.

업무 카탈로그 digest는 `sha256:c731b83d70142bed04cc68e829cedc08acaa357645feb295097e25de371a99b4`, 관리 카탈로그 digest는 `sha256:41a7d3ac23e7c3a4f820b553077cb9e01bd0eb38e9105b33196b7af8a1d66a2e`입니다. 파일 최상위의 `observation`은 관찰 시점·대상·조회 방법을, `catalog`는 실제 계약을 담습니다.

업무 API 369개는 목록·단건 조회 **168개**, 생성·수정·삭제 **180개**, 파일 GET·HEAD **20개**, 배치 **1개**로 구성됩니다. 컬렉션 이름과 안정적인 컬렉션 ID는 같은 기능의 두 URL 별칭이므로, 369개의 서로 다른 업무 기능이 있다는 뜻은 아닙니다. 기본 컬렉션은 31개이고 뷰는 11개입니다. 기본 컬렉션 가운데 `rsd_command_tasks`는 읽기만 공개하므로 쓰기 가능한 컬렉션은 30개입니다.

필드와 관계의 **소스 기준선**은 [업무 manifest](../../../internal/grant/patch_data_collections.json), [카탈로그 설명](../../catalog-coverage.md), [Patch Data 스키마 스냅샷](../../../../patch-data-auth-extraction/database/pb_migrations/1785485804_collections_snapshot.js)입니다. 로컬 추출 작업트리 HEAD는 `b7258579b92808b2f1c56795034214946224a602`이며 [배포 이미지의 OCI revision](runtime/image-patch-data.json)과 같습니다. 스냅샷 끝의 인증 분리 변환까지 적용하여 읽었습니다. 이 연결은 배포 이미지가 표기한 소스의 스키마 근거이며 **현재 DB에서 받아 온 스키마는 아닙니다**. [오늘 collection 정의 조회](runtime/collection-definitions-data.json)는 **401**로 실패했고 `collections=[]`, `businessCollectionCount=null`입니다. 따라서 DB의 필드 드리프트·후속 변경·전체 필드 값은 확인하지 못했습니다. 각 컬렉션에서 말하는 필요성은 이 구조가 지원하는 업무를 설명한 것이며, JSON 내부의 계약이나 장비 동작처럼 근거가 부족한 부분은 추론으로 표시합니다.

업무·관리 활성 계약은 모두 `accessMode=authorized`입니다. 업무 요청은 Exposure에서 Grant 평가를 거친 뒤 **서버 소유 service-token**으로 Patch Data에 전달합니다. 사용자 토큰을 업무 서버의 로그인 토큰으로 취급하지 않습니다. 업무·관리 합계 **438개**에 직접 Patch Auth 기능이나 아래의 소스 전용 경로를 더하지 않습니다.

[2026-09-12 검증](../ied-endpoints-2026-09-12/README.md)과 [369개 경로별 결과](../ied-endpoints-2026-09-12/route-summary.json)는 당시 배포에서 실제 CRUD·권한 부여/회수·파일·배치를 검증한 과거 근거입니다. 오늘의 활성 카탈로그와 과거 성공 기록은 서로 다른 증거입니다. 과거 관리 3개 결과를 오늘의 전체 관리 API 수로 사용하지 않습니다.

## 2. 로그인, 업무 프로필, 권한을 나누는 이유

Patch Auth는 비밀번호와 토큰을 확인하여 “누구인가”를 결정합니다. Grant는 조직 소속, 역할, 계약, 애플리케이션 허용과 동의를 확인하여 “무엇을 할 수 있는가”를 결정합니다. Exposure는 요청의 메서드·경로와 활성 계약을 맞춘 뒤 Grant의 허용 결과를 확인하고 업무 서버에 전달합니다.

따라서 Patch Data의 `users`, `viewers`, `temporary_accounts`는 **업무 관계에 사용하는 기본 프로필**입니다. 같은 컬렉션 ID를 유지하더라도 로그인 컬렉션이 아닙니다. Patch Data의 `users`를 만들거나 `organizations` 관계를 바꾸는 것으로 로그인 계정이나 Grant 조직 소속이 생기지 않습니다. 반대로 로그인에 성공한 사람도 발전소 조회 바인딩이 없으면 그 발전소를 볼 수 없습니다.

인증 신원은 검증한 **issuer origin + 인증 컬렉션 ID + 계정 레코드 ID**로 구분합니다. 이메일이 같거나 다른 컬렉션의 레코드 ID가 같다는 이유로 두 계정을 합치지 않습니다. 이 구분은 같은 사람 이름을 가진 업무 프로필을 잘못 인증 주체로 받아들이는 일을 막기 위해 필요합니다.

## 3. 모든 업무 컬렉션에 적용되는 메서드와 경로

다음 표의 `{C}`에는 4절에 적힌 **컬렉션 이름 또는 그 컬렉션의 안정적인 ID만** 넣습니다. `{id}`는 컬렉션 ID가 아니라 개별 레코드 ID이고, `{filename}`은 해당 레코드 응답에 포함된 실제 파일명입니다. `{C}`가 임의의 미등록 컬렉션을 허용하는 와일드카드는 아닙니다.

| 표기 | 메서드·경로 | 언제 필요한가 | 입력과 응답 | 필수 권한 |
|---|---|---|---|---|
| 목록 | `GET /api/collections/{C}/records` | 목록 화면, 검색, 페이지 이동에 필요합니다. 예를 들어 권한이 있는 발전소만 나열합니다. | `page`, `perPage`, `skipTotal`, 허용된 `filter`·`sort`, 선언된 `expand`를 받습니다. 기본 응답은 `page`, `perPage`, `totalItems`, `totalPages`, `items`이며 `skipTotal`을 쓰면 전체 건수를 기대하지 않습니다. | `patch-data.R.read`와 결과에 포함될 각 레코드의 정확한 바인딩이 필요합니다. |
| 단건 | `GET /api/collections/{C}/records/{id}` | 상세 화면과 수정 전 현재값 확인에 필요합니다. | 선언된 `expand`만 선택할 수 있습니다. `id`, `collectionId`, `collectionName`과 컬렉션별 필드가 반환됩니다. | `patch-data.R.read`, 리소스 `R/{id}`가 필요합니다. |
| 생성 | `POST /api/collections/{C}/records` | 새 발전소·이슈·보고서 등 업무 객체를 등록합니다. | 스키마에 맞는 JSON 또는 multipart를 받습니다. 관계 필드는 실제 연결할 레코드 ID이고, 필수 파일은 파일 바이트로 제출합니다. 성공 응답은 생성된 레코드입니다. | `patch-data.R.create`, 리소스 **`collection/{컬렉션 이름}`**이 필요합니다. |
| 수정 | `PATCH /api/collections/{C}/records/{id}` | 제목·배치·담당자·첨부 등을 변경합니다. | 바꿀 필드의 JSON 또는 multipart를 보냅니다. 성공 응답은 수정된 레코드입니다. | `patch-data.R.update`, 리소스 `R/{id}`가 필요합니다. |
| 삭제 | `DELETE /api/collections/{C}/records/{id}` | 잘못 등록한 자료나 필요 없어진 업무 객체를 제거합니다. | 본문 없이 요청하며 네이티브 성공은 204이고 응답 본문이 없습니다. | `patch-data.R.delete`, 리소스 `R/{id}`가 필요합니다. |
| 파일 내용 | `GET /api/files/{C}/{id}/{filename}` | 사진·첨부·HTML 보고서를 실제로 표시하거나 다운로드합니다. | JSON이 아닌 파일 바이트가 반환됩니다. 허용된 `download`, `thumb`, Range·조건부 헤더를 사용할 수 있습니다. | `patch-data.R.file.read`, 리소스 `R/{id}`가 별도로 필요합니다. |
| 파일 정보 | `HEAD /api/files/{C}/{id}/{filename}` | 다운로드 전에 길이·미디어 타입 등 헤더를 확인합니다. | 파일 본문 없이 헤더를 반환합니다. GET과 같은 파일 접근 권한이 필요합니다. | `patch-data.R.file.read`, 리소스 `R/{id}`가 필요합니다. |

여기서 `R`은 아래 목록의 canonical resource type입니다. `plants`만 `R=plant`이고 나머지는 컬렉션 이름과 같습니다. URL에서 이름을 안정적인 ID로 바꾸어도 action이나 권한 범위는 바뀌지 않습니다. 각 컬렉션의 `.read`, `.create`, `.update`, `.delete`, `.file.read`는 독립적인 권한입니다. 읽기 권한으로 파일을 내려받거나 데이터를 수정할 수는 없습니다.

권한을 사용하려면 해당 action이 역할과 애플리케이션에 명시적으로 포함되고, 주체·조직·정확한 대상에 대한 유효한 바인딩이 있어야 합니다. 조직 소속·계약·신뢰·동의 등 해당 평가 조건도 만족해야 합니다. 목록은 서버가 권한 조건을 원본 쿼리에 결합하므로, 허용된 애플리케이션에 조회 가능한 레코드가 없으면 정상적인 빈 목록을 반환할 수 있습니다. 목록 URL 자체에 대한 허용을 전체 행에 대한 허용으로 해석하지 않습니다.

예를 들어 발전소 목록의 두 구체적 URL은 `GET /api/collections/plants/records?page=1&perPage=20&sort=name`과 `GET /api/collections/30hrtar148uy66c/records?page=1&perPage=20&sort=name`입니다. 상세 URL은 `GET /api/collections/plants/records/{실제 발전소 레코드 ID}`입니다. 검색 예시는 `filter=name='동부 발전소'`이며 실제 요청에서는 쿼리를 URL 인코딩합니다. 이 예시가 해당 이름의 레코드가 존재하거나 권한이 부여되었다는 뜻은 아닙니다.

`allowedFields`는 **검색·정렬 허용 목록**입니다. 응답 필드 목록이나 쓰기 허용 필드 목록이 아닙니다. 관계 경로, 파일, JSON 내부 키, 숨김 필드, 비밀번호값을 임의로 검색할 수 없습니다. `fields`를 통한 임의 응답 투영도 현재 공개하지 않습니다. 예를 들어 `plants`는 `created,id,isArchive,name,updated`로 검색·정렬할 수 있지만 `metadata` 내부의 위치값이나 `organization.name`으로 검색할 수 있다고 가정해서는 안 됩니다. 각 컬렉션의 정확한 허용 필드는 manifest와 활성 카탈로그의 `recordScope.allowedFields`에 있습니다.

일반 변경 요청은 1 MiB, 파일 보유 컬렉션의 변경과 파일 전송은 128 MiB까지라는 **전달 계약 상한**이 있습니다. 개별 파일의 네이티브 크기·MIME 제한은 별도로 적용됩니다. 대부분의 레코드 JSON 응답은 1 MiB로 제한됩니다. 전송 상한이 크다고 필수 필드나 파일 종류 검증이 생략되지는 않습니다.

## 4. 업무 컬렉션 42개의 역할과 사용 예시

아래의 “전체 CRUD”는 목록·단건·생성·수정·삭제이며 이름/ID 별칭을 합쳐 **10개 경로**입니다. “전체 CRUD + 파일”은 두 파일 메서드와 두 별칭을 더한 **14개 경로**, “읽기 전용”은 목록·단건의 두 별칭인 **4개 경로**입니다. 모든 항목은 3절의 경로를 구체적인 이름과 ID로 치환하여 사용합니다. 파일이 없는 컬렉션에 파일 URL을 덧붙이지 않습니다.

필드 설명에서 “필수”는 소스 스키마의 required 항목이고, JSON 내부의 샘플 구조는 검증된 별도 계약이 없는 한 제시하지 않습니다. 관계의 참조 대상이 있다는 사실은 그 대상을 읽거나 변경할 권한까지 뜻하지 않습니다.

### 01. `users`: 업무 사용자 프로필

안정적인 ID는 `_pb_users_auth_`, canonical type은 `users`, 읽기 action은 `patch-data.users.read`입니다. 공개 동작은 **전체 CRUD + 파일, 14개 경로**입니다.

발전소 이슈의 작성자·담당자, 조직 소유자, 댓글 작성자를 동일한 업무 프로필로 연결하려면 사용자 레코드가 필요합니다. 이름을 각 이슈에 복사하면 이름 변경이나 동명이인 구분을 일관되게 처리하기 어렵습니다. 이 컬렉션은 그러한 관계의 기준을 제공하며 인증 비밀번호를 저장하지 않습니다.

입력·응답의 주요 개념은 필수 `username`, 표시 이름 `name`, 조직 관계 `organizations`, 확장 업무 정보 `metadata` JSON, 삭제 표식 `deleted`, 프로필 파일 `avatar`입니다. 소스 변환은 `password`, `tokenKey`, `email`, `emailVisibility`, `verified`, `failed_count`, `pw_changed_at`을 제거합니다. `agreements` JSON이 스냅샷 변환 후 남아 있더라도 그 업무 복사본이 현재 인증 동의의 권위 있는 상태라는 뜻은 아닙니다. 예를 들어 점검 담당자의 표시 이름을 변경할 때 이 레코드를 수정하여 이슈 작성자 연결을 유지합니다. `avatar` 조회에는 `patch-data.users.file.read`가 필요합니다.

### 02. `plants`: 발전소 업무의 기준 객체

안정적인 ID는 `30hrtar148uy66c`, canonical type은 **`plant`**, 읽기 action은 **`patch-data.plant.read`**입니다. 공개 동작은 **전체 CRUD, 10개 경로**입니다.

패널 배치, 이슈, 작업지시, 보고서가 어느 발전소에 속하는지 연결하려면 발전소 기준 객체가 필요합니다. 발전소가 없으면 같은 패널이나 이슈가 다른 현장 자료와 섞이기 쉽습니다. 여러 업무 기록의 중심 관계가 이 컬렉션으로 모입니다.

필수 필드는 `name`, 조직 관계 `organization`, 업무 권한 관계 `permission`입니다. `metadata` JSON, `images`, `files`, `isArchive`, 같은 컬렉션을 참조하는 `inherits`·`refPlant`도 있습니다. `inherits`의 이름만으로 Grant 권한이 상속된다고 해석하지 않습니다. 예를 들어 “동부 발전소”를 새로 등록할 때 이름·실제 조직 ID·permission ID를 함께 지정한 뒤 그 발전소에 패널맵을 연결합니다. 위치 정보를 다루는 네이티브 지오코딩 훅이 있으므로 전체 수정 권한에는 훅의 부수 효과도 포함됩니다. `images`·`files`의 바이너리는 각각 해당 파일 컬렉션에서 받습니다.

### 03. `panelmaps`: 발전소 패널 배치 데이터

안정적인 ID는 `l17r68r7ddzqcij`, canonical type은 `panelmaps`, 읽기 action은 `patch-data.panelmaps.read`입니다. 공개 동작은 **전체 CRUD, 10개 경로**입니다.

발전소 전체와 그 안의 패널 배치는 서로 다른 변경 단위를 갖습니다. 배치 데이터를 별도 레코드로 저장하면 현장 배치가 바뀌어도 발전소 프로필을 그대로 유지할 수 있습니다. 배치와 관련된 이슈나 화면을 연결하는 데 필요한 구조입니다.

필수 `plant`는 발전소 관계이고 필수 `data`는 JSON이며 선택 `metadata`도 JSON입니다. JSON의 좌표·패널 키 구조는 이 스키마만으로 확정하지 않습니다. 예를 들어 해당 발전소의 검증된 배치 JSON을 `POST /api/collections/panelmaps/records`로 등록하고, 이후 `PATCH .../records/{id}`로 배치를 갱신합니다. 실제 장비 제어를 실행하는 API라는 근거는 없습니다.

### 04. `organizations`: 업무 조직과 계층

안정적인 ID는 `1l9cjra913llae4`, canonical type은 `organizations`, 읽기 action은 `patch-data.organizations.read`입니다. 공개 동작은 **전체 CRUD + 파일, 14개 경로**입니다.

여러 발전소를 조직별로 묶고 조직 간 상하 관계를 표현하려면 업무 조직 데이터가 필요합니다. 발전소의 소속과 화면의 조직 표시를 통일하여 자료를 관리할 수 있습니다. 이 업무 조직 레코드는 Grant 조직과 별도의 데이터이므로 변경만으로 관리 권한이 생기지는 않습니다.

필수 `name`·소유 사용자 관계 `owner`, 선택 `parent` 조직 관계, `metadata` JSON, 파일 `logo`·`icon`·`homeBanner`가 있습니다. 예를 들어 지사 이름과 로고를 갱신하고 상위 조직을 연결합니다. 조직 변경 훅은 `org_parent_map`을 재구성할 수 있으므로 조직 수정은 단일 필드 이상의 영향을 가질 수 있습니다. 파일 3종은 서로 다른 file field지만 파일 URL에는 필드 이름 대신 응답에 적힌 파일명을 사용합니다.

### 05. `viewers`: 열람자 업무 프로필

안정적인 ID는 `kovp4j74yv9jr2v`, canonical type은 `viewers`, 읽기 action은 `patch-data.viewers.read`입니다. 공개 동작은 **전체 CRUD, 10개 경로**입니다.

일반 사용자와 별도로 열람자 관계를 유지하려면 viewer 프로필이 필요합니다. `permissions`가 사용자와 열람자를 각각 참조하므로 동일한 업무 모델 안에서도 대상을 구분할 수 있습니다. 조회용 뷰와 연결하더라도 viewer라는 이름 자체가 데이터 조회 권한을 자동으로 주지는 않습니다.

필수 `username`, 필수 `name`, 조직 관계 `organizations`, 삭제 표식 `deleted`가 주요 필드입니다. 비밀번호·이메일·토큰 필드는 업무 변환에서 제거됩니다. 예를 들어 외부 열람자의 표시 이름과 관련 조직을 등록하되, 실제 로그인은 Patch Auth의 viewer 계정으로 하고 데이터 읽기 권한은 Grant에서 따로 부여합니다.

### 06. `invitations`: 조직·발전소 초대 기록

안정적인 ID는 `lrjsosb8mul77k0`, canonical type은 `invitations`, 읽기 action은 `patch-data.invitations.read`입니다. 공개 동작은 **전체 CRUD, 10개 경로**입니다.

누구에게 어느 조직·발전소의 참여를 제안했는지와 사용 여부를 보관하려면 초대 기록이 필요합니다. 이를 사용자 프로필과 구분하면 아직 가입하지 않은 대상에게 보낸 초대도 관리할 수 있습니다. 초대의 만료와 사용 여부는 프로필 존재 여부와 다른 업무 상태입니다.

필수 `organization`, 발전소 관계 배열 `plants`, 대상 `email`, `isUsed`, `expired`가 있습니다. 예를 들어 특정 발전소를 대상으로 이메일 초대를 기록하고 사용 뒤 `isUsed`를 바꿉니다. CRUD 등록 자체가 초대 메일 발송·가입 승인·Grant 소속 등록을 수행한다는 근거는 없으므로 그런 동작은 별도 확인이 필요합니다.

### 07. `images`: 재사용 가능한 사진·이미지

안정적인 ID는 `w8l3o9jedinck7g`, canonical type은 `images`, 읽기 action은 `patch-data.images.read`입니다. 공개 동작은 **전체 CRUD + 파일, 14개 경로**입니다.

현장 사진을 발전소·댓글·이슈에 연결하려면 이미지 자체를 별도로 저장할 수 있어야 합니다. 같은 사진을 여러 업무 레코드에서 참조할 때 바이너리를 중복 저장하지 않고 관계만 연결할 수 있습니다. 사진 메타데이터 조회와 실제 바이트 다운로드도 따로 통제할 수 있습니다.

필수 파일 `image`와 문자 `tags`가 주요 필드입니다. `tags`는 이 스키마에서 문자열이며 `issue_tags` 관계와는 다릅니다. 예를 들어 손상된 패널 사진을 multipart로 생성하고 반환된 이미지 ID를 이슈의 `images`에 넣습니다. 사진 내용 GET·HEAD에는 `patch-data.images.file.read`가 필요합니다.

### 08. `dashboard`: 발전소 대시보드 데이터

안정적인 ID는 `i95ea9ateuaxwqg`, canonical type은 `dashboard`, 읽기 action은 `patch-data.dashboard.read`입니다. 공개 동작은 **전체 CRUD, 10개 경로**입니다.

발전소 기본 정보와 별도로 대시보드 상태를 저장하려면 발전소별 데이터 레코드가 필요합니다. 화면 구성이나 업무 요약을 한 객체로 읽고 변경할 수 있는 구조입니다. 다만 **대시보드의 구체적인 위젯·지표 의미는 이름과 `data` JSON에 근거한 해석**이며 스키마만으로 확정하지 않습니다.

필수 `plant` 관계와 필수 `data` JSON이 있습니다. 예를 들어 특정 발전소의 기존 클라이언트 계약에 맞는 대시보드 JSON을 수정하고 다시 읽어 화면에 반영합니다. 이 API가 실시간 발전량 계산이나 모니터링 수집을 직접 수행한다고 설명하지 않습니다.

### 09. `comments`: 발전소의 일반 의견·논의

안정적인 ID는 `lrsdfyc753nlnz7`, canonical type은 `comments`, 읽기 action은 `patch-data.comments.read`입니다. 공개 동작은 **전체 CRUD, 10개 경로**입니다.

특정 이슈로 등록하기 전의 현장 의견이나 발전소 단위의 논의를 남기려면 일반 댓글이 필요합니다. `parent` 관계로 답글을 연결하고 해결·보관 상태를 유지할 수 있으므로 단순한 텍스트 메모보다 후속 논의를 추적하기 쉽습니다.

필수 `plant`·`user` 관계와 `title`, `text`, `images`, `tag` JSON, `parent`, 날짜 `resolved`·`related`, `isPrivate`, `isArchive`가 있습니다. 예를 들어 발전소 점검 의견을 작성하고 다른 의견의 `parent`를 지정해 답글을 남깁니다. `isPrivate` 값을 썼다는 사실만으로 응답 필드가 가려지는 것은 아니며 읽기 권한은 전체 레코드 단위로 부여해야 합니다.

### 10. `permissions`: 기존 업무 접근 관계

안정적인 ID는 `pt20y45muocfut5`, canonical type은 `permissions`, 읽기 action은 `patch-data.permissions.read`입니다. 공개 동작은 **전체 CRUD, 10개 경로**입니다.

발전소와 사용자·열람자·임시 계정 간의 업무 관계를 유지하려면 이 관계 데이터가 필요합니다. 기존 시스템의 참조와 화면 구성을 유지하는 용도로 읽을 수 있습니다. 인증·권한 분리 이후에는 **이 데이터가 Grant 바인딩의 대체 권한 저장소가 아닙니다**.

관계 필드는 `plant`, `users`, `viewers`, `temporary`입니다. 예를 들어 기존 발전소 permission 레코드가 참조하는 열람자 목록을 확인합니다. 이 목록에 사용자를 추가해도 Grant의 `.read` action·바인딩·애플리케이션 허용을 별도로 만들지 않으면 새로운 API 접근 권한이 생기지 않습니다.

### 11. `groupconditions`: 발전소별 조건 묶음

안정적인 ID는 `1t0dxj7x1r6vvo0`, canonical type은 `groupconditions`, 읽기 action은 `patch-data.groupconditions.read`입니다. 공개 동작은 **전체 CRUD, 10개 경로**입니다.

이름을 붙인 조건 묶음을 발전소별로 재사용하려면 조건을 독립된 객체로 저장해야 합니다. 조건을 사용하는 화면마다 같은 내용을 복사하는 대신 하나의 묶음을 갱신할 수 있습니다. **조건이 구체적으로 무엇을 판정하는지는 이름과 JSON 필드로만 해석한 부분**이며 자동 경보나 Grant 조건식이라고 단정하지 않습니다.

필수 `plant`, 필수 `name`, 필수 `conditions` JSON이 있습니다. 예를 들어 기존 애플리케이션이 사용하는 “점검 대상 묶음”의 이름과 검증된 조건 JSON을 저장합니다. JSON 내부 연산자와 필수 키는 별도 소비자 계약 확인이 필요합니다.

### 12. `passwords`: 조직에 연결된 업무 비밀번호값

안정적인 ID는 `8ngtkucypa7xlaj`, canonical type은 `passwords`, 읽기 action은 `patch-data.passwords.read`입니다. 공개 동작은 **전체 CRUD, 10개 경로**입니다.

필수 `organization` 관계와 문자 `password`가 있는 조직별 업무 데이터입니다. 인증 분리 과정에서 제거한 로그인 해시와는 다른 컬렉션이므로 같은 이름의 값이라도 로그인 비밀번호로 해석하지 않습니다. **공유 링크 암호인지 장비 암호인지 등 정확한 사업 목적은 스키마만으로 확인되지 않습니다.**

예를 들어 권한을 가진 운영자가 특정 조직에 연결된 기존 업무 비밀번호 레코드를 갱신하는 사용을 구조상 지원합니다. 민감한 원문이 전체 응답에 포함될 수 있으므로 읽기 권한도 민감합니다. 이 API로 Patch Auth 비밀번호를 바꾸거나 계정 잠금을 해제할 수는 없습니다.

### 13. `temporary_accounts`: 한시 업무 참여자의 참조

안정적인 ID는 `d57i4dxnnp7l8hh`, canonical type은 `temporary_accounts`, 읽기 action은 `patch-data.temporary_accounts.read`입니다. 공개 동작은 **전체 CRUD, 10개 경로**입니다.

일시적인 작업 참여자를 조직·작업지시에 연결하려면 일반 사용자와 구분되는 업무 참조가 필요합니다. 만료 정보와 링크를 보관하여 작업의 한시성을 표현할 수 있습니다. 업무 프로필의 만료와 인증 토큰의 유효기간은 서로 다른 개념입니다.

필수 `username`, 필수 `organization`, 필수 `link`, 선택 `expired`, `work_order` 관계가 주요 필드입니다. 예를 들어 외부 점검 작업에 참여하는 계정 참조를 작업지시에 연결합니다. `link`가 곧 로그인 토큰이라는 근거는 없습니다. 이 레코드를 생성해도 실제 Patch Auth 임시 계정이 생기지 않으며 `expired`만으로 인증 만료가 강제된다고 주장하지 않습니다.

### 14. `issue_comments`: 이슈에 붙는 현장 논의

안정적인 ID는 `snv8bayo8ma6hw0`, canonical type은 `issue_comments`, 읽기 action은 `patch-data.issue_comments.read`입니다. 공개 동작은 **전체 CRUD, 10개 경로**입니다.

발전소 전체에 대한 댓글과 별도로 특정 문제의 진행 상황·증거·답변을 모으려면 이슈 댓글이 필요합니다. 이슈 관계가 있어 여러 문제의 논의가 섞이지 않고, 이미지와 맵 참조를 함께 유지할 수 있습니다.

필수 `issue`, 선택 `user`, editor 형식 `content`, `images`, JSON `map_ids`, 삭제 표식 `deleted`가 있습니다. 예를 들어 균열 이슈에 점검 결과와 사진 ID를 붙여 댓글을 생성합니다. `deleted`를 설정하는 업무 표식과 DELETE로 실제 레코드를 지우는 동작은 다릅니다.

### 15. `issue_content_logs`: 이슈 본문 변경 이력

안정적인 ID는 `misvzvyhp0fe7g4`, canonical type은 `issue_content_logs`, 읽기 action은 `patch-data.issue_content_logs.read`입니다. 공개 동작은 **전체 CRUD, 10개 경로**입니다.

현재 이슈 본문만 저장하면 변경 전에 어떤 설명이 있었는지 복구·확인하기 어렵습니다. 네이티브 이슈 수정 훅은 본문이 바뀔 때 이전 `content`를 별도 로그로 남기는 구조입니다. 업무 설명을 나중에 검토하는 데 필요한 이력입니다.

`issue`, `user` 관계와 editor 형식 `content`가 있습니다. 예를 들어 수정 전 점검 설명을 로그 단건으로 확인합니다. 카탈로그가 로그 쓰기·삭제도 공개하므로 변경 불가능한 감사 원장이라는 보장은 없습니다. 또한 서비스 토큰 경로의 로그 작성자 ID는 실제 최종 사용자로 자동 치환되지 않습니다.

### 16. `issue_fields_logs`: 이슈 필드 변경 이력

안정적인 ID는 `v5q8qxup8hbxg21`, canonical type은 `issue_fields_logs`, 읽기 action은 `patch-data.issue_fields_logs.read`입니다. 공개 동작은 **전체 CRUD, 10개 경로**입니다.

담당자·날짜·제목 등 본문 이외의 변경도 추적하려면 별도 필드 로그가 필요합니다. 소스 훅의 `diffs`는 변경된 키와 **새 값**을 저장합니다. 모든 필드의 이전 값·새 값 쌍을 저장한다고 설명하지 않습니다.

`issue`·`user` 관계와 필수 `diffs` JSON이 주요 필드입니다. 예를 들어 담당자나 해결 날짜가 바뀐 기록을 이슈 본문 이력과 함께 확인합니다. 훅이 실패를 기록하고 계속 진행할 수 있으므로 모든 변경에 완전한 감사 이력이 보장된다는 뜻은 아닙니다. 이 로그에도 전체 CRUD가 공개되어 있습니다.

### 17. `issue_tags`: 이슈 분류어

안정적인 ID는 `i10pcc82twxf6pd`, canonical type은 `issue_tags`, 읽기 action은 `patch-data.issue_tags.read`입니다. 공개 동작은 **전체 CRUD, 10개 경로**입니다.

같은 종류의 문제를 일관된 이름으로 표시하려면 분류어를 독립된 객체로 저장할 필요가 있습니다. 이슈마다 태그 문자를 따로 입력하는 것보다 기존 태그 ID를 재사용할 수 있습니다. 필드는 문자 `name`이며 `issues.tags`가 이 컬렉션을 참조합니다.

예를 들어 “점검 필요”라는 태그를 등록한 뒤 여러 이슈의 `tags`에 연결합니다. 특정 태그가 필수거나 고정된 산업 분류라는 근거는 없으므로 예시는 설명용 이름입니다. 태그 조회 권한과 태그가 붙은 이슈 조회 권한은 독립적입니다.

### 18. `issues`: 발전소 문제와 처리 상태

안정적인 ID는 `hqrb76fyop74r06`, canonical type은 `issues`, 읽기 action은 `patch-data.issues.read`입니다. 공개 동작은 **전체 CRUD, 10개 경로**입니다.

현장의 문제를 제목·설명·담당자·일정·해결 상태로 관리하려면 이슈 객체가 필요합니다. 이슈를 중심으로 댓글, 작업지시, 변경 이력, 일정을 연결할 수 있어 하나의 문제에 대한 후속 업무를 추적할 수 있습니다.

필수 `plant`, `title`, `creator`와 선택 editor `content`, `images`, `tags`, `assignee`, 날짜 `date`·`resolved`·`deleted`, JSON `map_ids`, `isPublic`, `is_all_day`가 있습니다. 예를 들어 특정 발전소 패널 이상을 등록하고 담당자를 지정한 뒤 해결 날짜를 기록합니다. 수정은 로그 훅의 부수 효과를 포함합니다. `isPublic=true`가 Grant 인증·레코드 권한을 자동으로 우회하지 않습니다.

### 19. `work_orders`: 여러 이슈를 묶는 작업지시

안정적인 ID는 `yzruwgkyotkp02h`, canonical type은 `work_orders`, 읽기 action은 `patch-data.work_orders.read`입니다. 공개 동작은 **전체 CRUD, 10개 경로**입니다.

문제 기록과 실제 작업 단위를 나누려면 작업지시가 필요합니다. 하나의 작업이 여러 이슈를 처리할 수 있으므로 이슈 배열을 묶는 레코드가 일정한 작업 범위를 제공합니다. 임시 계정이 이 작업지시를 참조할 수도 있습니다.

필수 `issues` 관계와 필수 `plant`, 선택 `user`가 있습니다. 예를 들어 같은 발전소의 두 이슈를 하나의 작업지시로 묶어 담당 사용자와 연결합니다. 완료율·비용·승인 단계는 스키마에 없는 개념이므로 이 API가 그런 기능까지 제공한다고 가정하지 않습니다.

### 20. `command_history`: 발전소 명령 데이터의 이력

안정적인 ID는 `ogq6rbg3df1g496`, canonical type은 `command_history`, 읽기 action은 `patch-data.command_history.read`입니다. 공개 동작은 **전체 CRUD, 10개 경로**입니다.

발전소에 관련된 명령 데이터와 보낸 사용자를 추적하려면 명령 이력 레코드가 필요합니다. 필수 `plant`, 필수 `command` JSON, 선택 `sender` 관계를 보관합니다. **명령의 구체적 형식·대상 장치·성공 여부는 이 JSON 스키마만으로 확정되지 않습니다.**

예를 들어 기존 시스템이 기록한 명령 JSON과 발전소·송신자 관계를 조회하여 당시 요청 내용을 검토합니다. 이 컬렉션에 POST한다고 실제 장치에 명령이 전달되거나 RSD가 실행된다고 주장하지 않습니다. 이름이 이력이어도 CRUD가 가능하므로 변경 불가능한 감사 기록으로 취급할 수 없습니다.

### 21. `issue_schedules`: 이슈에 연결된 일정

안정적인 ID는 `kyzupz6cwz00swl`, canonical type은 `issue_schedules`, 읽기 action은 `patch-data.issue_schedules.read`입니다. 공개 동작은 **전체 CRUD, 10개 경로**입니다.

이슈 자체의 날짜와 별도로 여러 후속 일정을 등록하려면 일정 객체가 필요합니다. 재점검·방문 등 후속 시점을 이슈 관계로 묶어 유지할 수 있습니다. 필드는 `issue`, `date`, `title`, 삭제 표식 `deleted`입니다.

예를 들어 해결되지 않은 이슈에 다음 점검 일정을 생성한 뒤 날짜를 수정합니다. 알림 발송이나 외부 캘린더 동기화가 구현되었다는 근거는 없으며 이 API는 일정 데이터를 관리합니다.

### 22. `reports`: 파일로 보관하는 보고서

안정적인 ID는 `r0thjxht4e4dz2m`, canonical type은 `reports`, 읽기 action은 `patch-data.reports.read`입니다. 공개 동작은 **전체 CRUD + 파일, 14개 경로**입니다.

업무 결과를 당시의 문서 파일로 보존하려면 보고서 메타데이터와 파일을 함께 저장해야 합니다. 보고 기간과 대상 발전소를 연결하면 나중에 어느 현장의 어느 기간을 다룬 보고서인지 확인할 수 있습니다.

필수 `organization`, 발전소 배열 `plants`, `title`, 파일 `file`, 날짜 `start`·`end`와 선택 `comment`, `description`, `isArchive`가 있습니다. 소스의 `file`은 **`text/html`, 최대 5 MiB**이므로 PDF 업로드를 지원한다고 안내하지 않습니다. 예를 들어 월간 HTML 점검 보고서를 multipart로 생성하고 반환된 파일명으로 `GET /api/files/reports/{id}/{filename}`을 호출합니다. 문서 내용 다운로드에는 `patch-data.reports.file.read`가 필요합니다.

### 23. `files`: 공통 첨부 파일

안정적인 ID는 `pbc_3446931122`, canonical type은 `files`, 읽기 action은 `patch-data.files.read`입니다. 공개 동작은 **전체 CRUD + 파일, 14개 경로**입니다.

이미지 전용 객체나 HTML 보고서와 구분하여 일반 첨부를 재사용하려면 공통 파일 객체가 필요합니다. 발전소가 `files` 관계로 여러 첨부를 연결할 수 있으므로 파일명·태그와 바이너리를 분리하여 관리할 수 있습니다.

필수 파일 `file`, 선택 `name`, 문자 `tags`가 주요 필드입니다. 예를 들어 스키마가 허용하는 현장 첨부를 등록하고 발전소의 `files`에 파일 ID를 연결합니다. 필드 이름이 일반적이라고 모든 MIME과 모든 크기가 허용된다고 단정하지 않습니다. 바이너리 읽기는 `patch-data.files.file.read`로 별도 승인합니다.

### 24. `blueprints`: 날짜를 가진 발전소 설계·배치 자료

안정적인 ID는 `pbc_1114506494`, canonical type은 `blueprints`, 읽기 action은 `patch-data.blueprints.read`입니다. 공개 동작은 **전체 CRUD, 10개 경로**입니다.

동일 발전소의 날짜별 설계·배치 자료를 저장하려면 패널맵과 별도의 기준일이 있는 객체가 필요합니다. 목록용 뷰와 상세용 객체를 나누어 큰 본문을 필요한 순간에만 읽을 수 있습니다. **설계 자료라는 설명은 컬렉션 이름과 필드 관계에 근거한 해석**이며 CAD 형식이나 JSON 내부 요소까지 확정하지 않습니다.

필수 `plant`, 필수 JSON `data`, 필수 `date`, 선택 JSON `metadata`·`internalMeta`, `isArchive`가 있습니다. 예를 들어 기존 애플리케이션 계약에 맞는 특정 날짜의 배치 자료를 등록합니다. `internalMeta`는 소스에서 숨김 필드가 아니므로 이름만 보고 응답에서 제외된다고 가정하지 않습니다.

### 25. `resources`: 조직·발전소에 연결된 확장 자료

안정적인 ID는 `pbc_2337082678`, canonical type은 `resources`, 읽기 action은 `patch-data.resources.read`입니다. 공개 동작은 **전체 CRUD, 10개 경로**입니다.

조직이나 발전소에 연결할 확장 JSON을 독립적으로 저장하는 구조입니다. 필수 `data` JSON과 선택 `organization`·`plant` 관계가 확인됩니다. **어떤 종류의 자원을 뜻하는지에 대한 전용 설명이 없어 이름과 관계만으로 해석**하며 장비·재고·인력 관리 기능이라고 확정하지 않습니다.

예를 들어 기존 소비자가 사용하는 보조 자료 JSON을 해당 발전소 ID와 함께 저장하는 사용을 지원합니다. 이를 실제 운영 목적에 연결하기 전에는 JSON 계약과 소비자 코드를 확인해야 합니다. 리소스라는 일반 이름이 모든 다른 컬렉션에 대한 권한을 대표하지는 않습니다.

### 26. `reports_v2`: 구조화된 보고서와 손실 지표

안정적인 ID는 `pbc_3451676670`, canonical type은 `reports_v2`, 읽기 action은 `patch-data.reports_v2.read`입니다. 공개 동작은 **전체 CRUD, 10개 경로**입니다.

보고서 파일만으로는 기간별 결과와 손실 항목을 구조적으로 비교하기 어렵습니다. 본문·요약 JSON과 숫자 지표를 가진 보고서 객체를 두면 목록 뷰와 최신 보고서 뷰에서 결과를 재사용할 수 있습니다. API는 저장된 보고서 데이터를 관리하며 손실 계산 알고리즘 자체가 아닙니다.

주요 필드는 `plant`, `organization`, `title`, JSON `content`·`summary`·`metadata`, `start`, `end`, `description`, `isArchive`, `type`, 숫자 `gen`, `loss_shade`, `loss_dust`, `loss_defect`, `loss_indirect`, `loss_total`입니다. 이름상 음영·먼지·결함·간접·총손실을 구분하지만 **단위·산식·비율 여부는 미확인**입니다. 예를 들어 월간 보고서의 기간과 저장된 손실 수치를 등록하고 상세에서 본문과 요약을 읽습니다. `type`은 소스에서 텍스트이며 고정 enum이 아닙니다.

### 27. `org_parent_map`: 조직 조상 관계의 물리화 자료

안정적인 ID는 `35180llsus0m925`, canonical type은 `org_parent_map`, 읽기 action은 `patch-data.org_parent_map.read`입니다. 공개 동작은 **전체 CRUD, 10개 경로**입니다.

조직 계층을 반복해서 재귀 조회하는 대신 조직과 상위 조직 관계를 미리 저장하려면 이 매핑이 필요합니다. 소스 훅은 조직 변경·시작 시 재귀 SQL로 관계를 다시 계산합니다. 따라서 일반 수동 관리 자료와 달리 조직 원본을 바꾸면 이 행들이 재생성될 수 있습니다.

필수 `organization`·`parent` 관계와 필수 숫자 `depth`, 이름 `name`이 있습니다. 레코드 ID는 `lower(hex(organizationID || ':' || parentID))`이며 두 15자 ID의 경우 **62자**입니다. 예를 들어 자회사와 상위 회사의 연결·깊이를 읽습니다. 공개 수정 권한이 있더라도 직접 수정한 행이 훅에서 보존된다는 보장은 없습니다. 이 업무 트리가 Grant 조직 권한 상속을 자동으로 구성하지 않습니다.

### 28. `plants_viewer`: 열람자용 발전소 투영

안정적인 ID는 `6260lz1eo0w2vcr`, canonical type은 `plants_viewer`, 읽기 action은 `patch-data.plants_viewer.read`입니다. 공개 동작은 **읽기 전용, 4개 경로**입니다.

열람 화면에 필요한 발전소 필드를 일정한 형태로 읽으려면 별도 뷰가 필요합니다. SQL은 `plants`에서 `id,name,metadata,organization,images,refPlant,isArchive,inherits,permission,created`를 선택하고 원본의 `files`·`updated`는 선택하지 않습니다. 별도 저장 객체가 아니라 원본을 투영합니다.

예를 들어 열람자 화면에서 발전소 이름과 이미지를 표시합니다. SQL에 사용자의 신원을 검사하는 조건은 없으므로 뷰 이름 자체가 열람자별 필터는 아닙니다. 원본 발전소 권한과 별도로 이 뷰가 반환하는 ID에 대한 바인딩이 필요합니다.

### 29. `panelmaps_viewer`: 열람자용 패널맵 투영

안정적인 ID는 `8hamrn2tnq9j6c6`, canonical type은 `panelmaps_viewer`, 읽기 action은 `patch-data.panelmaps_viewer.read`입니다. 공개 동작은 **읽기 전용, 4개 경로**입니다.

패널맵 상세를 열람자 발전소 뷰와 연결하여 일정한 응답 형태로 읽기 위한 뷰입니다. SQL은 패널맵 `id,data,created`와 `plants_viewer`의 ID를 `plant`로 선택합니다. 원본 `metadata`는 이 투영에 포함되지 않습니다.

예를 들어 열람자 화면에서 `data`를 표시하고 선언된 `expand=plant`로 열람자용 발전소를 읽습니다. 패널맵 뷰와 확장되는 발전소 뷰 양쪽 권한이 필요합니다. 이 뷰에 PATCH하여 원본 배치를 바꾸는 동작은 공개하지 않습니다.

### 30. `temporary_panelmaps`: 한시 화면용 패널맵 투영

안정적인 ID는 `zkvh97egvg137hs`, canonical type은 `temporary_panelmaps`, 읽기 action은 `patch-data.temporary_panelmaps.read`입니다. 공개 동작은 **읽기 전용, 4개 경로**입니다.

한시 업무 화면이 사용하는 제한된 패널맵 형태를 제공하는 뷰입니다. SQL은 원본 `panelmaps`에서 `id,plant,data,created`를 선택합니다. **한시 화면 용도는 이름에 근거한 해석**이며 SQL 자체에 임시 계정 만료 필터는 없습니다.

예를 들어 한시 점검 화면에서 허용된 패널맵의 배치 JSON을 조회합니다. 원본이나 다른 viewer 뷰의 바인딩으로 자동 접근할 수 없으며 이 namespace의 독립적 권한이 필요합니다. `metadata` 수정·조회 기능을 이 뷰에 추가해서 해석하지 않습니다.

### 31. `temporary_plants`: 한시 화면용 발전소 투영

안정적인 ID는 `fiqvrx9bli2qvag`, canonical type은 `temporary_plants`, 읽기 action은 `patch-data.temporary_plants.read`입니다. 공개 동작은 **읽기 전용, 4개 경로**입니다.

한시 업무 화면이 발전소의 일부 정보를 읽도록 별도 응답 형태를 제공하는 뷰입니다. SQL은 `id,name,metadata,organization,refPlant,isArchive,created`를 선택하고 이미지·파일·permission·inherits는 선택하지 않습니다. 필요한 정보의 구조를 분리하는 이유가 스키마에서 확인됩니다.

예를 들어 외부 점검 화면에서 현장 이름과 조직 참조를 표시합니다. **외부 점검이라는 예시는 이름에 근거한 활용 설명**이며 뷰가 스스로 임시 자격을 검증한다는 뜻은 아닙니다. 기본 발전소와 동일한 ID를 반환해도 `temporary_plants` 권한을 별도로 확인합니다.

### 32. `temporary_blueprints`: 한시 화면용 설계 자료 상세

안정적인 ID는 `pbc_405412299`, canonical type은 `temporary_blueprints`, 읽기 action은 `patch-data.temporary_blueprints.read`입니다. 공개 동작은 **읽기 전용, 4개 경로**입니다.

설계 자료의 본문은 제공하되 원본 메타데이터와 분리된 형태가 필요할 때 사용하는 뷰입니다. SQL은 `blueprints`의 `id,plant,data,date,created,updated`를 선택합니다. `metadata`, `internalMeta`, `isArchive`는 응답 투영에 없습니다.

예를 들어 한시 업무 화면에서 특정 날짜의 설계 `data`를 상세 조회합니다. **한시 업무 목적은 이름에 근거한 설명**이며 SQL이 만료·보관 여부를 자동 필터링하는 것은 아닙니다. 권한은 이 뷰의 레코드별로 확인하고, 수정은 원본의 별도 권한으로 처리해야 합니다.

### 33. `blueprints_viewer`: 열람자용 설계 자료 상세

안정적인 ID는 `pbc_1337089469`, canonical type은 `blueprints_viewer`, 읽기 action은 `patch-data.blueprints_viewer.read`입니다. 공개 동작은 **읽기 전용, 4개 경로**입니다.

열람자용 발전소 관계를 유지하면서 날짜별 설계 본문을 제공하려면 별도 상세 뷰가 필요합니다. SQL은 `id,data,date,created,updated`와 `plants_viewer`의 ID를 `plant`로 선택합니다. 원본 메타데이터는 포함하지 않습니다.

예를 들어 열람 화면에서 설계 자료 본문과 해당 발전소를 함께 표시합니다. `plant` 관계를 확장하면 발전소 viewer 뷰까지 읽기 권한이 필요합니다. 원본 `blueprints`가 보인다는 사실만으로 이 뷰가 보이거나 그 반대가 되지 않습니다.

### 34. `latest_loss_reports`: 발전소별 최신 비보관 보고서

안정적인 ID는 `pbc_1915755548`, canonical type은 `latest_loss_reports`, 읽기 action은 `patch-data.latest_loss_reports.read`입니다. 공개 동작은 **읽기 전용, 4개 경로**입니다.

모든 보고서를 받아 클라이언트마다 최신 판정 코드를 작성하는 대신 발전소별 최신 보고서를 서버에서 일정한 기준으로 선택할 필요가 있습니다. SQL은 `reports_v2`에서 `isArchive=false`만 대상으로 발전소별 **`end` 내림차순 → `created` 내림차순 → `id` 내림차순**으로 한 행을 선택합니다. 최신이 단순 생성 시각만을 뜻하지 않는 이유가 명시되어 있습니다.

응답은 `organization`·`organizationName`, `plant`·`plantName`, 제목·기간·설명·메타데이터·발전/손실 필드를 포함합니다. 다수 보고서 필드는 이 뷰 스키마에서 JSON형으로 선언되어 있어 원본 숫자·문자 자료형을 그대로 검색 허용 필드로 간주하지 않습니다. 예를 들어 여러 발전소의 최신 손실 보고서를 개요 화면에 표시합니다. 최신 선정 결과의 원본 ID를 사용하더라도 이 뷰의 권한은 독립적입니다.

### 35. `reports_v2_list`: 본문을 뺀 비보관 보고서 목록

안정적인 ID는 `pbc_2089487142`, canonical type은 `reports_v2_list`, 읽기 action은 `patch-data.reports_v2_list.read`입니다. 공개 동작은 **읽기 전용, 4개 경로**입니다.

큰 `content`·`summary`를 매번 받지 않고 기간별 보고서를 탐색하려면 목록 전용 투영이 필요합니다. SQL은 `isArchive=false`인 보고서의 ID, 발전소·조직, 제목·기간·설명·type·metadata·gen·손실 수치와 생성/수정 시각을 선택합니다. 최신 한 건만 고르는 뷰와 달리 조건에 맞는 여러 보고서를 보여 줍니다.

예를 들어 월간 보고서를 목록에서 고른 후 원본 `reports_v2` 상세를 별도 권한으로 읽습니다. 이 뷰에서 `content`나 `summary`가 나오기를 기대하지 않습니다. 보관 보고서가 목록에 없는 것은 이 SQL의 결과이며 전체 원본 삭제를 뜻하지 않습니다.

### 36. `blueprints_list`: 본문을 뺀 설계 자료 목록

안정적인 ID는 `pbc_1185224127`, canonical type은 `blueprints_list`, 읽기 action은 `patch-data.blueprints_list.read`입니다. 공개 동작은 **읽기 전용, 4개 경로**입니다.

큰 설계 `data`를 모두 내려받지 않고 날짜별 자료를 고르려면 목록 전용 뷰가 필요합니다. SQL은 `id,plant,metadata,internalMeta,date,isArchive,created,updated`를 선택하며 `data`는 제외합니다. 별도의 `WHERE isArchive=false`는 없으므로 모든 보관 자료가 자동 제외된다고 안내하지 않습니다.

예를 들어 날짜와 메타데이터로 자료를 선택한 다음 원본 상세에서 본문을 읽습니다. `internalMeta`가 이 뷰에 포함되므로 외부 노출 시 전체 응답을 읽을 수 있는 주체에만 권한을 부여해야 합니다. 본문을 받으려면 원본의 별도 읽기 권한이 필요합니다.

### 37. `rsd_command_tasks`: RSD 명령 작업 상태

안정적인 ID는 `pbc_4011293327`, canonical type은 `rsd_command_tasks`, 읽기 action은 `patch-data.rsd_command_tasks.read`입니다. 기본 컬렉션이지만 공개 동작은 **읽기 전용, 4개 경로**입니다.

명령 작업의 취소 가능 기간, 실행 담당, 완료·실패 상태를 추적하려면 작업 상태 객체가 필요합니다. 단순 명령 이력과 달리 마감과 상태 전이가 있어 처리 중인 작업을 확인할 수 있습니다. **RSD의 실제 장치 동작·명령 전송 방식은 이 스키마로 확정하지 않습니다.**

필수 `plant`, `detected_notification` JSON, `command_targets` JSON, `cancel_deadline_at`, `status`가 있으며 status 값은 `cancelable,canceled,executing,completed,failed,manual`입니다. 선택 필드는 `resolved_notification`, `canceled_at`, `canceled_by`, `cancel_reason`, `claimed_at`, `claimed_by`, `claim_deadline_at`, `finished_at`, `error_message`입니다. 예를 들어 작업이 실행 중인지와 실패 메시지를 상세에서 확인합니다. 네이티브 상태 전이 규칙을 서비스 superuser가 우회할 수 있어 POST·PATCH·DELETE·배치 upsert를 공개하지 않습니다. 조회 필드에 취소 정보가 있다는 이유로 취소 API가 공개되었다고 해석하지 않습니다.

### 38. `agreement_documents`: 버전이 있는 약관 문서

안정적인 ID는 `pbc_165310787`, canonical type은 `agreement_documents`, 읽기 action은 `patch-data.agreement_documents.read`입니다. 공개 동작은 **전체 CRUD, 10개 경로**입니다.

약관 제목뿐 아니라 어떤 버전의 내용을 게시했는지 유지하려면 독립적인 문서 객체가 필요합니다. 필수 여부·활성 여부·가입 화면 표시 여부를 구분하여 문서 교체 시 이전 버전을 식별할 수 있습니다. 소스에는 `key,version` 조합의 고유 제약이 있습니다.

필수 `key`, `title`, `version`, 선택 Markdown `content_md`, `is_required`, `is_active`, `published_at`, `show_on_sign_up`가 있습니다. 예를 들어 특정 버전의 약관 본문과 게시 상태를 관리합니다. **Patch Auth에도 같은 이름의 인증 소유 문서 사본이 있습니다.** 업무 사본을 변경한다고 인증 사본이 자동 동기화되거나 사용자가 동의한 것으로 처리되지 않습니다. Grant의 조직 간 `agreements` 계약과도 다른 기능입니다.

### 39. `anomaly_occurrences`: 이상 발생과 해소의 단위 기록

안정적인 ID는 `anomalyoccur001`, canonical type은 `anomaly_occurrences`, 읽기 action은 `patch-data.anomaly_occurrences.read`입니다. 공개 동작은 **전체 CRUD, 10개 경로**입니다.

이상 유형 하나의 발생·해소를 같은 사건으로 연결하려면 occurrence 레코드가 필요합니다. 현재 상태만 보관하는 대신 사건 키와 탐지·해소 정보를 같이 유지할 수 있어 후속 작업의 근거로 사용할 수 있습니다. **어떤 알고리즘이 이상을 탐지하고 어느 프로세스가 생성하는지는 별도 확인 대상**입니다.

필수 `plant`, 문자 `anomaly_type`, 문자 `event_key`, 상태 `status`(`detected,resolved`)와 선택 JSON `source`, `targets`, `detected_notification`, `resolved_notification`, 날짜 `detected_at`, `resolved_at`가 있습니다. 예를 들어 기존 탐지 결과를 등록한 뒤 해소 통지와 시각을 같은 사건에 기록합니다. `event_key`라는 이름만으로 중복 방지·멱등 처리가 보장되거나 고유 제약이 있다고 주장하지 않습니다.

### 40. `blueprints_viewer_list`: 열람자용 설계 자료의 가벼운 목록

안정적인 ID는 `pbc_1337089470`, canonical type은 `blueprints_viewer_list`, 읽기 action은 `patch-data.blueprints_viewer_list.read`입니다. 공개 동작은 **읽기 전용, 4개 경로**입니다.

열람자 화면에서 설계 본문을 모두 받지 않고 날짜별 자료를 선택하려면 viewer 목록 전용 뷰가 필요합니다. SQL은 `id,date,created,updated`와 `plants_viewer` ID를 `plant`로 선택하고 `data`를 제외합니다. 상세 viewer 뷰와 목록 뷰의 전달량을 구분하는 구조입니다.

예를 들어 날짜 목록에서 하나를 선택한 뒤 `blueprints_viewer` 상세를 별도 권한으로 조회합니다. 목록의 허용이 상세 본문이나 발전소 확장 허용을 자동으로 포함하지 않습니다. SQL 자체의 열람자 신원 필터도 없으므로 Grant의 행별 권한이 필요합니다.

### 41. `temporary_blueprints_list`: 한시 화면용 설계 자료 목록

안정적인 ID는 `pbc_405412300`, canonical type은 `temporary_blueprints_list`, 읽기 action은 `patch-data.temporary_blueprints_list.read`입니다. 공개 동작은 **읽기 전용, 4개 경로**입니다.

한시 업무 화면에 날짜·발전소·식별자만 전달하고 큰 설계 본문을 상세에서 따로 읽게 하려면 이 뷰가 필요합니다. SQL은 원본의 `id,plant,date,created,updated`를 선택하며 `data`와 메타데이터는 선택하지 않습니다. **한시 화면이라는 용도 설명은 이름에 근거한 해석**입니다.

예를 들어 작업 대상 날짜를 목록에서 고른 뒤 `temporary_blueprints`의 본문을 읽습니다. 이름의 temporary가 토큰 만료 검사나 기간별 자동 삭제를 구현한다는 뜻은 아닙니다. 목록 뷰와 상세 뷰의 권한을 각각 설정합니다.

### 42. `anomaly_occurrence_checkpoints`: 이상 처리의 진행 위치

안정적인 ID는 `projcheckpoint1`, canonical type은 `anomaly_occurrence_checkpoints`, 읽기 action은 `patch-data.anomaly_occurrence_checkpoints.read`입니다. 공개 동작은 **전체 CRUD, 10개 경로**입니다.

시간·ID 순서로 데이터를 처리할 때 어디까지 읽었는지 저장하려면 진행 위치가 필요합니다. 필수 날짜 `cursor_created`와 필수 문자열 `cursor_id`, 생성·수정 시각이 확인됩니다. **이상 occurrence 처리의 재개 지점이라는 해석은 이름과 커서 필드 구조에 근거한 추론**이며 실제 처리기·재시도·중복 방지 알고리즘은 확인하지 않았습니다.

예를 들어 운영자가 저장된 마지막 처리 시각과 ID를 확인하여 처리 진행 상황을 조사합니다. 이 값을 바꾸면 재처리 또는 건너뛰기에 영향을 줄 가능성이 있으므로 쓰기 권한은 전체 작업 의미를 이해하는 주체에게 부여합니다. 정확히 한 번 처리된다는 보장은 이 스키마로 입증되지 않습니다.

## 5. 파일·배치·관계 확장을 따로 설명하는 이유

### 파일 업로드와 다운로드

파일을 포함한 레코드는 업로드와 레코드 메타데이터 생성이 한 동작입니다. 예를 들어 새 사진은 `POST /api/collections/images/records`의 multipart `image` 필드로 올리고, 기존 사진 변경은 `PATCH /api/collections/images/records/{id}`로 처리합니다. JSON으로 파일명만 작성하는 것은 새 파일 바이트를 업로드하는 것과 다릅니다. 파일 제거도 네이티브 필드 계약에 맞는 수정 동작입니다.

| 컬렉션 | 네이티브 파일 필드 | 업로드 권한 | 실제 파일 읽기 권한 |
|---|---|---|---|
| `users` | `avatar` | `patch-data.users.create` 또는 `.update` | `patch-data.users.file.read` |
| `organizations` | `logo`, `icon`, `homeBanner` | `patch-data.organizations.create` 또는 `.update` | `patch-data.organizations.file.read` |
| `images` | `image` | `patch-data.images.create` 또는 `.update` | `patch-data.images.file.read` |
| `reports` | `file` | `patch-data.reports.create` 또는 `.update` | `patch-data.reports.file.read` |
| `files` | `file` | `patch-data.files.create` 또는 `.update` | `patch-data.files.file.read` |

파일 field는 7개지만 URL은 field별로 만들어지는 것이 아니라 컬렉션·레코드·파일명으로 식별하므로 5개 컬렉션 × 2개 별칭 × GET/HEAD = 20개 경로입니다. 다른 레코드가 이미지 관계를 갖더라도 이미지 바이너리 권한을 자동으로 상속하지 않습니다. Range는 일부 바이트만 받고, 조건부 헤더는 이미 가진 파일 버전과 비교하는 데 필요합니다. 썸네일·다운로드 옵션의 정확한 형식과 허용값은 활성 계약과 원본 파일 검증에 따릅니다.

### 배치: 서로 관련된 변경을 함께 처리하기

`POST /api/batch`는 여러 변경을 원자적인 네이티브 요청 하나로 전달할 때 필요합니다. 예를 들어 패널맵 두 레코드를 한 번에 생성·변경하는 경우에 사용할 수 있습니다. 루트 권한은 action `patch-data.batch.execute`, 리소스 **`batch/patch-data`**입니다. 이 권한과 각 하위 작업의 create/update/delete 권한을 **모두** 확인합니다.

JSON 입력은 `requests` 배열이고 각 원소는 `method`, `url`, 선택 `body`·`headers` 개념을 갖습니다. 예를 들어 하위 항목 `method=PATCH`, `url=/api/collections/panelmaps/records/{id}`, `body`에 검증된 변경 필드를 넣습니다. 최대 20개 작업이며 아무 URL이나 넣는 전달 API가 아닙니다. 허용된 mutation capability는 180개이고, 뷰·RSD 쓰기는 포함되지 않습니다. 응답은 각 하위 작업의 `status`·`body`를 담는 배열입니다.

배치 내부의 `PUT /api/collections/{C}/records`는 명시적인 `body.id`로 create/update 양쪽을 점검하는 upsert 계약이 있습니다. 30개 쓰기 가능 컬렉션의 두 별칭인 60개 템플릿입니다. **독립된 외부 PUT API 60개가 추가 공개된 것은 아닙니다.** 같은 URL이라도 batch 내부 항목으로만 등록된 계약입니다. 기존 행이 있는지로 권한을 한쪽만 검사하지 않고 생성 대상과 정확한 레코드 수정 대상 둘 다 점검합니다.

multipart 배치는 `@jsonPayload`로 요청 배열을 담고 파일 필드는 `requests.{index}.{field}` 또는 `requests[{index}].{field}` 계약을 사용합니다. 하위 권한 하나라도 거부되면 전체 요청을 전달하지 않습니다. 네이티브 실행 실패의 원자적 롤백은 원본 batch 기능이며, 오늘 활성 카탈로그를 조회한 것으로 모든 롤백·동시성 경우를 새로 검증했다는 뜻은 아닙니다.

### expand: 연결된 자료를 함께 읽기

`expand`는 관계 ID만 받는 대신 관련 레코드 본문을 응답의 `expand` 객체로 함께 받는 기능입니다. 예를 들어 `GET /api/collections/panelmaps/records/{id}?expand=plant`로 패널맵과 발전소를 함께 표시할 수 있습니다. 루트 패널맵 `.read`와 관련 발전소의 `patch-data.plant.read` 및 각각의 정확한 ID 바인딩이 필요합니다.

활성 응답 graph에 선언된 관계만 확장할 수 있습니다. 최대 깊이는 6, 최대 응답 레코드는 200입니다. 관련 데이터까지 허용되는지 확인한 뒤 응답 바이트를 내보내므로 루트만 허용되고 자식이 거부되면 자식 내용을 일부 누출하는 방식으로 응답하지 않습니다. 소스의 74개 명시적 관계는 임의 조인이나 모든 역관계 접근 허용을 의미하지 않습니다. 현재 미공개인 관계 기반 filter/sort와도 다른 기능입니다.

### 전체 레코드 변경 권한의 의미

생성·수정·삭제 권한은 해당 네이티브 동작 전체를 승인합니다. 관계 필드 변경, cascade delete, 조직 관계 재구성, 이슈 로그 추가, 발전소 지오코딩 훅 등 부수 효과까지 포함될 수 있습니다. 관련 대상마다 별도로 쓰기 권한을 확인하거나 필드별 수정 허용 목록을 적용하는 계약은 아닙니다. 예를 들어 이슈 담당자 변경 권한을 허용하면 동일한 전체 수정 API에서 다른 mutable 필드도 변경할 수 있다는 점을 고려해야 합니다.

`isArchive`, `deleted`, `resolved`, `expired`는 각각 소스 필드일 뿐 공통 삭제·인증 만료 정책이 아닙니다. 값만 바꿔서 보관·만료 처리가 전 시스템에 동일하게 전개된다고 가정하지 않습니다. 뷰에 명시된 SQL 필터와 인증 훅·Grant 평가 조건을 각각 확인합니다.

## 6. Patch Auth 인증 기능

이 절의 인증 컬렉션은 **Patch Auth origin**에 속합니다. 앞에서 설명한 동일한 이름·ID의 **Patch Data 업무 origin**과 혼동하면 안 됩니다. [배포 설정](runtime/exposure-configuration.json)의 인증 서비스는 `http://patch-auth.patch-data-grant-exposure-go-v1.svc.cluster.local:8091`입니다. 클러스터 내부 주소이며 외부 인터넷 공개 URL이라는 뜻은 아닙니다. 네트워크 endpoint와 포트 포워드를 통한 응답이 확인되었습니다. 사용자 접근 주소는 [런타임 네트워크 근거](runtime/network-access.json)와 해당 환경의 접속 방법을 따릅니다.

소스 근거는 [Patch Auth README](../../../../patch-auth/README.md), [인증 스키마 설명](../../../../patch-auth/docs/database-schema.md), [인증 migration](../../../../patch-auth/pb_migrations/1778142859_auth_collections.js), [보안 migration](../../../../patch-auth/pb_migrations/1785485805_auth_security.js)입니다. **인증 소스와 배포의 연결 확실성은 업무 스키마보다 낮습니다.** 배포 이미지 digest는 `sha256:89adab664fa7f7be9457d8a5d1b1e31ca890e618b84cbcced60ed89975dfb335`이고 [OCI labels](runtime/image-patch-auth.json)는 비어 있어 배포 revision을 검증하지 못했습니다. [레지스트리 tag 관찰](runtime/image-tags-patch-auth.json)은 같은 digest에 `sha-5130e33e8457e94395ce69180faf863cbf2f8dd6`와 `sha-bc973ffe0c84a6f4728aa183890033f9d44f27ef`를 연결하며 로컬 HEAD는 전자입니다. tag는 변경 가능하므로 이 연결만으로 로컬 보안 훅 전체가 배포되었다고 단정하지 않습니다. 소스의 PocketBase 0.40.3 기준도 배포 바이너리 버전 관찰과 구분합니다.

### 오늘 직접 관찰한 인증 설정

2026-10-04 **20:44:38 KST**에 `GET /api/collections/users/auth-methods`가 **200**을 반환했습니다. 비밀번호 로그인은 enabled이고 식별 필드는 `email,username`, OTP·MFA도 enabled입니다. OAuth2는 flag가 true지만 **`providerIds=[]`**이므로 사용 가능한 Google 등 외부 제공자를 안내하지 않습니다. 로컬 README의 Google 지원 가능 설명은 현재 사용 가능 제공자 증거가 아닙니다. 로컬 초기 스키마의 OTP·MFA disabled 설명보다 오늘 관찰한 users 설정을 우선합니다.

[배포 설정](runtime/exposure-configuration.json)의 `accountExposure.deployed=false`와 [계정 카탈로그 조사](runtime/catalog-account-status.json)는 별도 계정 Exposure가 없음을 보여 줍니다. 따라서 인증 기능을 Exposure의 `public_authn` 경로나 활성 account catalog로 표시하지 않습니다. 이번 조사에서 로그인·refresh·OTP 발송·메일 확인·OAuth 성공을 실행하지 않았습니다. **설정 enabled, 경로 계약, 실제 성공 호출**은 서로 다른 근거 수준입니다. users 이외의 인증 컬렉션 설정도 런타임에서 확인하지 않았습니다.

[네이티브 단건 관찰](runtime/patch-auth-nonexistent-record.json)은 유효한 15자 합성 ID `inventory000001`에 대한 `GET /api/collections/users/records/inventory000001`이 **404**였음을 기록합니다. 앞선 잘못된 입력의 401을 이 최종 결과로 사용하지 않습니다. 존재하지 않는 자료의 404는 본인 프로필 조회 성공이나 타인 자료 차단의 전수 검증이 아닙니다. 현재 관찰의 자세한 시점과 소스 후보는 [런타임 요약](runtime/runtime-summary.md)을 참조합니다.

### 인증 컬렉션 세 가지

| 인증 컬렉션 / 안정적인 ID | 본질적인 필요성 | 신원·프로필 개념과 소스 기본 규칙 |
|---|---|---|
| `users` / `_pb_users_auth_` | 일반 사용자가 자신의 신원을 확인하고 프로필을 관리할 수 있어야 합니다. | 이메일 또는 username으로 비밀번호 로그인합니다. 이름·avatar·deleted 등 프로필이 있습니다. 소스 기준으로 공개 생성, 본인 목록/단건/수정/삭제가 가능하고 본인 판정은 컬렉션 ID와 레코드 ID를 함께 확인합니다. |
| `viewers` / `kovp4j74yv9jr2v` | 일반 사용자와 구분된 열람자 자격을 확인합니다. 실제 업무 조회는 Grant가 따로 승인합니다. | username으로 비밀번호 로그인합니다. 소스 기준으로 생성은 superuser만 가능하며 본인의 프로필 조회·수정·삭제를 허용합니다. |
| `temporary_accounts` / `d57i4dxnnp7l8hh` | 한시 업무 참여자를 일반 계정과 구분하여 인증합니다. | username으로 비밀번호 로그인합니다. 소스 기준으로 생성·수정·삭제는 superuser 전용이고 본인 조회만 허용합니다. `expired`는 프로필 정보이며 자체적으로 자격 만료를 강제한다는 근거가 없습니다. |

인증 데이터에는 네이티브 `password`·`tokenKey`·`email`·`emailVisibility`·`verified` 개념이 있습니다. `password`와 `tokenKey`는 반환용 공개 필드가 아닙니다. 인증 서비스에는 업무 조직·작업지시·임시 링크 관계가 없으며, 조직 권한을 담은 토큰을 임의로 발급하지 않습니다.

### 직접 네이티브 인증 메서드·경로와 필요한 입력

아래 메서드·경로는 **users 직접 네이티브 기능의 계약 안내**입니다. `auth-methods`의 200과 password/OTP/MFA enabled는 런타임 근거이며 나머지 요청·응답 세부 계약은 소스의 PocketBase 기준선입니다. 소스 버전 미확인과 미실행 범위를 그대로 유지하며 Exposure 활성 경로 수에 포함하지 않습니다. 일반 네이티브 `{A}` 템플릿에는 `users`·`viewers`·`temporary_accounts`와 각각의 안정적인 ID를 사용할 수 있는 구조지만, users 이외 설정과 ID 별칭의 실제 성공 호출은 미확인입니다.

| 기능 | 네이티브 메서드·경로 | 왜 필요한가 / 사용 예시 | 입력·응답과 요구조건 |
|---|---|---|---|
| 로그인 수단 확인 | `GET /api/collections/users/auth-methods` | 화면이 실제 설정에 맞는 로그인 수단을 보여 주는 데 필요합니다. | **현재 200 관찰**. 비밀번호·OAuth·OTP·MFA 설정 개념을 읽습니다. 표시 설정이 있다는 사실은 OAuth 비밀키나 SMTP가 정상이라는 증거가 아닙니다. |
| 비밀번호 로그인 | `POST /api/collections/users/auth-with-password` | 입력한 자격으로 계정 소유자를 확인하는 기본 기능입니다. | **password enabled 관찰, 로그인 미실행**. `identity`, `password`를 받습니다. 네이티브 성공은 `token`, `record`이며 MFA가 필요하면 중간 `mfaId`를 받는 흐름입니다. |
| 인증 갱신 | `POST /api/collections/users/auth-refresh` | 기존 토큰을 다시 검증하고 갱신된 인증 결과를 받는 데 필요합니다. Grant가 신원을 검증하는 데도 사용합니다. | **배포의 인증 공급자는 users로 구성되어 있으나 refresh 미실행**. 기존 bearer를 사용하고 갱신 `token`, `record`를 받습니다. 만료·변조된 토큰을 새 신원으로 받아들이지 않습니다. |
| OAuth 로그인 | `POST /api/collections/users/auth-with-oauth2` | 외부 계정 제공자가 확인한 신원을 로그인에 연결할 때 필요합니다. | **현재 OAuth2 flag true, providerIds가 비어 있으므로 외부 제공자 흐름 미구성**. 소스 입력은 `provider`, `code`, `codeVerifier`, `redirectUrl`입니다. CSAP 활성 시 로컬 훅은 users OAuth를 거부합니다. 그 훅이 배포에서 실행 중인지는 미확인입니다. |
| OAuth 연결 목록 | `GET /api/collections/users/records/{id}/external-auths` | 본인 계정에 어떤 외부 신원이 연결되어 있는지 확인합니다. | **소스 기능, 현재 연결·성공 호출 미확인**. 본인 또는 네이티브 관리 자격이 필요합니다. provider 목록이 비어 있어도 기존 연결 데이터 존재 여부까지 추론하지 않습니다. |
| OAuth 연결 해제 | `DELETE /api/collections/users/records/{id}/external-auths/{provider}` | 더 이상 사용하지 않는 외부 계정 연결을 제거합니다. | **소스 기능, 미실행**. 네이티브 본인/관리 조건을 만족해야 합니다. 다른 로그인 수단의 가용성까지 자동 보장하지 않습니다. |
| 비밀번호 재설정 요청 | `POST /api/collections/users/request-password-reset` | 비밀번호를 잊어도 이메일 소유권 확인을 거쳐 계정 접근을 회복할 수 있어야 합니다. | **메일 설정·전달 미검증, 미실행**. `email`을 받으며 재설정 메일과 토큰이 필요합니다. 응답 성공과 실제 메일 배달은 다릅니다. |
| 비밀번호 재설정 확정 | `POST /api/collections/users/confirm-password-reset` | 이메일로 받은 토큰을 확인한 뒤 새 비밀번호를 설정합니다. | **소스 기능, 미실행**. `token`, `password`, `passwordConfirm`을 사용합니다. 배포에서 CSAP 정책이 켜졌는지는 OTP/MFA flag만으로 확정하지 않습니다. |
| 이메일 확인 요청 | `POST /api/collections/users/request-verification` | 프로필 이메일을 실제로 소유하는지 검증하는 데 필요합니다. | **메일 설정·전달 미검증, 미실행**. `email`을 받습니다. 비밀번호 로그인이나 Grant 조직 가입과 다른 검증입니다. |
| 이메일 확인 확정 | `POST /api/collections/users/confirm-verification` | 확인 링크의 토큰으로 `verified` 상태를 확정합니다. | **소스 기능, 미실행**. `token`을 받습니다. 임의 `verified=true` 제출을 이메일 소유권 검증과 같게 취급하지 않습니다. |
| 이메일 변경 요청 | `POST /api/collections/users/request-email-change` | 로그인 연락처를 안전하게 바꾸기 위한 확인 절차입니다. | **소스 기능, 미실행**. 기존 bearer와 `newEmail`을 사용하며 메일 설정·전달이 필요합니다. |
| 이메일 변경 확정 | `POST /api/collections/users/confirm-email-change` | 새 연락처로 받은 변경 토큰을 검증합니다. | **소스 기능, 미실행**. `token`, `password`를 사용하는 네이티브 계약입니다. |
| OTP 요청 | `POST /api/collections/users/request-otp` | 일회용 비밀번호로 추가 자격 확인이나 설정된 로그인을 진행합니다. | **OTP enabled 관찰, 발송 미실행**. `email`을 사용하며 응답 `otpId`와 전달된 코드는 다릅니다. SMTP와 정책 검증이 필요합니다. 로컬 CSAP 훅은 challenge header까지 검사하지만 그 배포 여부는 미확인입니다. |
| OTP 인증 | `POST /api/collections/users/auth-with-otp` | 발급된 OTP의 ID·코드·세션을 맞춰 최종 인증합니다. | **OTP·MFA enabled 관찰, 인증 미실행**. `otpId`, `password`(OTP 코드), MFA 흐름의 `mfaId`가 필요할 수 있습니다. MFA는 인증 방식에 중간 challenge를 결합하는 기능이며 독립적인 네이티브 `/mfa/login` 경로가 확인된 것은 아닙니다. |

예를 들어 비밀번호 로그인 입력 개념은 `identity=<본인 이메일 또는 username>, password=<본인 비밀번호>`입니다. 성공한 경우에만 반환된 token을 해당 환경에서 요구하는 bearer 계약으로 사용합니다. MFA 중간 응답은 최종 bearer가 아니므로 업무 API에 그대로 전달하지 않습니다. 토큰·비밀번호·코드를 이 안내나 인벤토리에 기록하지 않습니다.

### 직접 계정·약관·프로필 파일 경로의 소스 기준선

계정 프로필은 로그인 토큰과 별도로 이름·avatar·연락처를 관리해야 하므로 네이티브 레코드 기능을 사용합니다. 아래는 **소스에 있는 기능이며 현재 성공 권한 검증은 하지 않았습니다**. 모든 경로는 Patch Auth origin에 속하며 42개 업무 컬렉션의 권한 계약과 다릅니다.

| 직접 네이티브 메서드·경로 | 필요성·예시 | 입력·응답 / 소스 자격 조건 |
|---|---|---|
| `GET /api/collections/users/records` | 로그인한 본인의 프로필을 목록 형태로 읽습니다. | self-only 소스 규칙입니다. 임의의 전체 사용자 목록을 공개한다고 해석하지 않습니다. |
| `GET /api/collections/users/records/{id}` | 현재 본인 이름·avatar 등 프로필을 읽습니다. | 네이티브 본인 조건이 적용되며 같은 컬렉션 ID·record ID를 함께 확인합니다. 존재하지 않는 ID의 거부/404 probe는 실제 본인 레코드 성공 조회의 증거가 아닙니다. |
| `POST /api/collections/users/records` | 새 일반 사용자가 로그인 가능한 계정을 만듭니다. | 소스의 공개 가입 기능입니다. `username`, `password`, `passwordConfirm`, 선택 email·name·avatar 개념을 사용하며 가입 정책·약관 검증이 적용될 수 있습니다. 현재 공개 가입 성공은 미실행입니다. |
| `PATCH /api/collections/users/records/{id}` | 본인 이름·프로필 이미지 등을 변경합니다. | 소스의 본인 수정 조건이 필요합니다. 비밀번호를 바꾸는 경우 네이티브 기존 비밀번호 확인·정책 계약을 따라야 합니다. |
| `DELETE /api/collections/users/records/{id}` | 본인 계정 프로필을 삭제합니다. | 소스의 본인 삭제 조건을 따릅니다. 계정 삭제가 업무 레코드·Grant 기록의 일괄 삭제를 보장하지 않습니다. |
| `GET /api/files/users/{id}/{filename}` | 본인 프로필의 avatar 파일 바이트를 읽습니다. | 네이티브 파일 필드의 보호 설정과 권한 계약을 따르는 소스 기능입니다. 업무의 `.file.read`를 그대로 적용하지 않습니다. |
| `GET /api/collections/agreement_documents/records` | 가입 전에 동의할 문서·버전을 선택합니다. | 인증 소유 문서 사본이며 소스 list rule은 `is_active=true`입니다. 실제 배포 문서가 채워져 있는지 확인하지 않았습니다. |
| `GET /api/collections/agreement_documents/records/{id}` | 선택한 약관의 Markdown 본문과 버전을 읽습니다. | 소스 view rule은 활성 문서만 허용합니다. 현재 본문 조회·가입 동의 성공은 미실행입니다. |

`viewers`·`temporary_accounts`에도 `GET /api/collections/{A}/records`, `GET .../records/{id}`, `POST .../records`, `PATCH .../records/{id}`, `DELETE .../records/{id}`라는 같은 네이티브 경로 구조가 있습니다. 그러나 viewer 생성은 superuser, 임시 계정 생성·수정·삭제는 superuser 전용인 소스 규칙이므로 일반 가입 기능으로 안내하지 않습니다. 인증 소유 약관의 POST·PATCH·DELETE도 네이티브 관리자 기능이며 최종 사용자 공개 관리 기능으로 계산하지 않습니다. ID 별칭의 가능한 구조와 실배포 호출 검증은 별개입니다.

네이티브 `POST /api/files/token`은 인증 주체에게 단기 파일 토큰을 발급하여 보호된 파일을 요청할 때 사용하는 **조건부 원본 기능**입니다. 현재 배포에서 이 요청을 실행하거나 avatar 보호 설정을 확인하지 않았으므로 현재 필요한 절차로 단정하지 않습니다. 이 토큰과 Grant의 업무 `.file.read` 권한은 서로 다른 계약입니다. 이름/ID 별칭을 포함한 업무 파일 경로 20개에 이 기능을 추가하지 않습니다.

로그아웃을 위한 별도 네이티브 `POST /logout` 공개 경로는 이 근거에서 확인하지 않았습니다. 클라이언트가 토큰을 지우는 동작과 서버가 모든 세션을 즉시 회수하는 동작은 구분해야 합니다. 계정 대리 로그인·superuser 조작·토큰 서명 설정 등 관리 기능도 공개 계정 API로 간주하지 않습니다.

### 소스에서만 확인한 CSAP MFA: 중간 인증과 최종 로그인을 나누는 이유

비밀번호만 맞았다고 모든 로그인을 완료하지 않고 추가 코드 확인을 요구하면 비밀번호 유출만으로 계정을 사용하는 위험을 줄일 수 있습니다. [CSAP 설정 훅](../../../../patch-auth/pb_hooks/csap_mfa.pb.js)은 `CSAP_SECURITY_ENABLED=true`일 때 users의 auth token 30분, MFA 10분, OTP 3분·6자리를 구성합니다. `CSAP_MFA_EXEMPT_EMAILS` 예외도 별도로 적용됩니다. 이것은 **소스의 조건부 기능**이며 오늘 배포 설정을 확인하기 전에는 활성 기능이라고 주장하지 않습니다.

| 소스 메서드·경로 | 필요성과 단계 | 입력·응답 개념 / 공개 여부 |
|---|---|---|
| `POST /api/csap/mfa/start` | 비밀번호 단계에서 받은 MFA 세션을 challenge에 결합하고 코드 발송 흐름을 준비합니다. | 입력 `mfaId`; 소스 응답은 `challengeToken`, `canonicalEmail`, generation·MFA·OTP 식별자와 상태 개념입니다. 내부 전달 정보가 있으므로 공개 카탈로그를 별도로 확인해야 합니다. |
| `POST /api/csap/mfa/activate` | 실제 OTP와 challenge를 결합하여 발송·활성 상태를 확정합니다. | 입력 `challengeToken`, `generation`, `otpId`를 검증하는 소스 기능입니다. 브라우저에 직접 공개된다고 가정하지 않습니다. |
| `POST /api/csap/mfa/status` | 화면이 만료·재전송 가능 시각·남은 시도를 서버 기준으로 표시합니다. | 입력 `challengeToken`; 검증 후 `state`, `maskedEmail`, `remainingAttempts`, `serverNow`, `otpExpiresAt`, `resendAvailableAt`, `sendWindowExpiresAt`와 내부 generation·MFA·OTP 식별자 개념을 반환합니다. 공개 프록시에서 개인정보·내부 식별자를 정리했다는 근거는 없습니다. |
| `POST /api/csap/mfa/resend` | 메일 지연·코드 만료 때 제한 안에서 새 발송을 예약합니다. | 입력 `challengeToken`; challenge 상태와 발송 window·cooldown을 검증합니다. 소스 응답에는 canonicalEmail과 내부 상태 개념도 있으므로 무조건 공개용 응답으로 취급하지 않습니다. |
| `POST /api/csap/mfa/cancel` | 사용자가 중간 인증을 중단하여 challenge를 종료합니다. | 입력 `challengeToken`; 소스는 검증 후 정리하고 204를 반환합니다. 이미 완료한 모든 bearer의 로그아웃 API라고 해석하지 않습니다. |
| `POST /api/collections/users/auth-with-otp` 및 소스 ID 별칭 | 최종 코드·OTP·MFA·challenge를 함께 맞추고 인증 토큰을 반환합니다. | 로컬 훅은 `X-Patch-CSAP-Challenge` header, query `mfaId`, 본문 `otpId`·6자리 `password`를 확인합니다. 이 CSAP 추가 계약과 ID 별칭은 배포에서 실행 검증하지 않았습니다. |

구체적인 request key·header 계약은 [CSAP 구현](../../../../patch-auth/pb_hooks/csap_mfa.shared.js)의 **로컬 소스 기준선**입니다. 이 구현은 OTP 요청에도 `X-Patch-CSAP-Challenge`를 요구하며 본문 email과 challenge의 사용자를 맞춥니다. 오늘의 MFA/OTP enabled 관찰만으로 CSAP 훅·환경 변수·위 경로의 배포를 확정하지 않습니다. 인증 이미지가 두 SHA tag에 연결되어 있고 revision label이 없다는 공백을 유지합니다. 소스 상태 `send_reserved,sending,awaiting_activation,active,delivery_unknown,exhausted,authenticating,terminal`은 발송 성공 여부가 불확실한 경우와 인증 진행 중 상태를 따로 처리하려는 구조입니다. 내부 상태 레코드를 직접 CRUD하여 최종 인증을 완료하는 공개 방식은 아닙니다. 별도 계정 Exposure가 없으므로 이 경로들을 활성 Exposure 수에 포함하지 않습니다.

### 보안 정책과 가입 동의

[비밀번호 정책](../../../../patch-auth/pb_hooks/users_password_policy.shared.js)은 CSAP 활성 시 문자 종류·길이와 연속 숫자·반복·키보드 연속·개인정보 포함을 검증합니다. [잠금 훅](../../../../patch-auth/pb_hooks/csap_auth_lockout.pb.js)은 users 비밀번호 실패를 5회까지 기록하고 잠금 조건을 적용합니다. 성공 OTP 처리에서는 token key 갱신 등 후속 처리가 있습니다. 비밀번호 변경 시각도 인증 소유 상태입니다. 별도 “비밀번호 정책 API”나 “잠금 해제 API”가 공개되었다는 뜻은 아닙니다.

[가입 약관 훅](../../../../patch-auth/pb_hooks/users_agreements.pb.js)은 CSAP 활성 시 인증 소유 `agreement_documents` 가운데 활성·가입 화면 표시 문서를 읽고, 필수 문서가 모두 동의되었는지 확인합니다. 가입 입력의 `agreements`는 `document`와 `agreed=true` 개념을 갖는 배열이며, 저장할 동의 시각·버전·출처는 서버가 확정합니다. 이 검증이 필요한 이유는 체크박스 표시만으로 정확한 문서 버전에 동의했다는 기록이 남지 않기 때문입니다. 비-superuser 응답에는 동의 메타데이터를 숨기는 훅도 있습니다.

[메일 링크 훅](../../../../patch-auth/pb_hooks/auth_email_links.pb.js)의 `POST /api/auth/action-token/validate`는 `action=password-reset` 또는 `email-verification`과 `token`을 받아 `{valid: ...}`를 반환하는 소스 기능입니다. 만료된 링크에서 입력을 받기 전에 유효성을 표시하는 용도이며, 토큰 검증 성공만으로 재설정·확인을 완료하지 않습니다. 해당 경로의 공개 여부와 `PATCH_SERVICE_ORIGIN` 링크 설정은 별도로 확인해야 합니다.

## 7. 공개하지 않은 기능과 확인이 남은 부분

| 기능 | 현재 업무 공개 상태 | 구분이 필요한 이유 |
|---|---|---|
| Realtime 구독·이벤트 (`GET`·`POST /api/realtime`) | 업무 활성 카탈로그에 없습니다. | 네이티브 스트림이 존재해도 구독 주체·범위·권한 회수·스트림 수명을 따로 통제해야 합니다. 단건 read 허용은 지속 구독 허용이 아닙니다. |
| 관계를 넘는 filter/sort, 임의 조인 | 공개하지 않습니다. | 다른 레코드의 정보를 조건에 사용하려면 별도 권한·쿼리 의미가 필요합니다. `expand` 공개와 다릅니다. |
| 임의 `fields` 투영 | 공개하지 않습니다. | 검색 허용 필드 목록이 필드별 읽기 권한이라는 뜻은 아닙니다. |
| 11개 뷰 쓰기·RSD 쓰기 | 공개하지 않습니다. | 뷰는 읽기 전용이고 RSD는 네이티브 상태 전이 제약을 보호해야 합니다. |
| 단독 업무 PUT upsert | 공개하지 않습니다. | 선언된 upsert는 batch 내부의 별도 의미입니다. |
| PocketBase 컬렉션 설정·시스템 테이블·superuser 관리·백업·서명/SMTP/OAuth 비밀 설정 | 업무 공개 카탈로그에 없습니다. | 업무 CRUD와 운영자 시스템 관리를 구분해야 합니다. Exposure의 로컬 management 라우터에 보이는 경로를 오늘 배포 활성 계약으로 자동 추가하지 않습니다. |
| 인증 내부 `_mfas`, `_otps`, `csap_mfa_challenges`, `csap_mfa_send_windows` CRUD | 인증 소유 내부 상태입니다. | challenge·토큰·발송 제한을 임의로 조작하는 공개 업무 컬렉션이 아닙니다. 42개 업무 컬렉션 수에 포함하지 않습니다. |

오늘의 카탈로그 관찰은 업무·관리 438개 경로가 활성이라는 근거이며 모든 사용자에게 접근이 허용된다는 근거가 아닙니다. 별도 계정 Exposure는 없고 users password·OTP·MFA 설정은 확인했지만 실제 인증·메일 전달은 검증하지 않았습니다. OAuth 제공자 목록은 비어 있습니다. viewer·임시 계정 설정, CSAP 추가 경로, 인증 약관·가입·프로필 수정과 인증 source revision은 미확인입니다. 업무 DB 스키마 조회는 401로 실패했으므로 업무 필드는 OCI revision과 일치하는 로컬 스키마 소스 설명이며 실DB 확인값이 아닙니다. JSON 업무 payload의 상세 의미가 없는 `dashboard`, `groupconditions`, `resources`, 명령 내부 데이터와 수치 단위는 소비자 계약을 추가 확인해야 합니다. 이 문서는 그 공백을 확정된 사업 기능으로 채우지 않았습니다.
