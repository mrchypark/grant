# Grant 관리 UI

`grant-admin`은 Grant 관리 API를 사용하는 별도 브라우저 애플리케이션이다. 기본 주소는 `127.0.0.1:8095`이며 `/healthz`는 프로세스 상태를 확인한다. 템플릿과 정적 자산은 실행 파일에 포함되어 작업 디렉터리의 assets 경로를 요구하지 않는다. 기존 Grant API 포트와 프로세스 설정은 바꾸지 않는다. 실행 파일은 Docker 이미지에도 `/usr/local/bin/grant-admin`으로 들어가며, 컨테이너 기본 `ENTRYPOINT`는 계속 `grant`다. UI만 실행하려면 entrypoint를 `/usr/local/bin/grant-admin`으로 지정한다.

## 로그인과 권한 경계

브라우저 로그인은 Goauthy confidential OAuth client의 Authorization Code 흐름과 PKCE를 사용한다. Goauthy에 `${GRANT_ADMIN_PUBLIC_URL}/oauth/callback`을 client redirect URI로 등록해야 한다. UI에는 `openid profile email goauthy.read` 범위가 기본 요청되며 `GRANT_ADMIN_GOAUTHY_SCOPES`로 바꿀 수 있다. Goauthy는 사용자 인증과 OAuth client 및 요청 scope의 허용을 담당한다. `GRANT_ADMIN_GOAUTHY_RESOURCE`는 authorization 요청에 포함되는 resource indicator이며 Grant가 기대하는 access-token audience와 일치해야 한다. 조직 동의, Grant application admission, Grant roles와 permissions는 Grant에서 관리한다.

로그인 성공은 사용자 세션을 만들 뿐, Grant 플랫폼 권한을 부여하지 않는다. Grant API는 자체 권한 검사를 계속 수행한다. UI 로그인에 성공해도 관리 API가 허용하지 않은 작업은 실패한다. master key는 사용하지 않는다.

## 실행

다음 값은 형식 예시이며 실제 client secret이 아니다. 운영에서는 secret을 환경변수 또는 secret manager로 공급한다.

```sh
export GRANT_ADMIN_BIND_ADDR=127.0.0.1:8095
export GRANT_ADMIN_API_URL=http://127.0.0.1:8080
export GRANT_ADMIN_MANAGEMENT_ORGANIZATION_ID=org-admin
export GRANT_ADMIN_PUBLIC_URL=http://127.0.0.1:8095
export GRANT_ADMIN_GOAUTHY_ISSUER=http://127.0.0.1:8081
export GRANT_ADMIN_GOAUTHY_CLIENT_ID=grant-admin
export GRANT_ADMIN_GOAUTHY_CLIENT_SECRET=dummy-secret
export GRANT_ADMIN_GOAUTHY_RESOURCE=https://grant.example.test/management
go run ./cmd/grant-admin
```

`GRANT_ADMIN_API_URL`은 관리 API가 노출된 Exposure URL 또는 Grant base URL이다. 이 값과 `GRANT_ADMIN_MANAGEMENT_ORGANIZATION_ID`, `GRANT_ADMIN_PUBLIC_URL`, Goauthy issuer/client ID/client secret은 필수다. API URL은 loopback 호스트(`localhost` 또는 loopback IP)에 한해 로컬 개발용 HTTP를 허용하며, 그 외에는 HTTPS를 사용해야 한다. HTTP인 cluster API에는 loopback port-forward를 사용하거나 TLS를 적용한다. Goauthy resource는 선택 사항이며, 지정할 때는 HTTPS URL이어야 한다. bind 주소는 `127.0.0.1:8095`가 기본이다. 외부에서 접근 가능한 배포에서는 TLS가 적용된 공개 root URL을 `GRANT_ADMIN_PUBLIC_URL`에 지정하고, 정확히 같은 callback URL을 Goauthy에 등록한다. Goauthy client의 `default_aud`가 Grant가 기대하는 audience로 설정되어 있으면 `GRANT_ADMIN_GOAUTHY_RESOURCE`를 생략한다. 명시적으로 지정할 때는 Grant가 기대하는 audience와 같은 HTTPS URL을 사용하고, Goauthy의 전역 resource allowlist와 client audience 정책에서도 해당 URL을 허용해야 한다.

```sh
docker run --rm \
  --entrypoint /usr/local/bin/grant-admin \
  --env-file ./grant-admin.env \
  -e GRANT_ADMIN_BIND_ADDR=0.0.0.0:8095 \
  -p 8095:8095 \
  grant:latest
```

세션은 단일 프로세스의 메모리에 최대 8시간 보관되며 재시작하면 다시 로그인해야 한다. 여러 인스턴스를 실행할 때는 sticky routing이 필요하다.

## 화면 범위와 API 권한

화면은 provider/action/HTTP capability 카탈로그의 stage·activate 작업과 users, members/enrollment, organizations, groups, agreements, applications, roles, bindings 관리로 나뉜다. 카탈로그 배포 단계와 Grant의 조직·사용자 권한 관리는 서로 다른 관리 경계다.

새 API는 두 개다. `GET /api/v1/admin/users`는 `admin.users.read` 권한으로 Grant 사용자 directory를 읽고, `GET /api/v1/admin/providers/{providerId}/http-capability-generations/{generation}`는 `admin.catalog.read` 권한으로 지정한 HTTP capability generation을 읽는다. Stage 성공 후 일반 form 제출은 HTTP `303`으로 generation 읽기 화면에 이동한다. HTMX 요청은 HTTP `204`와 `HX-Redirect` 헤더를 받아 같은 화면으로 이동한다. 화면에는 generation, catalog digest, capability 내용이 표시된다. 이 GET 주소는 새로고침해도 stage POST를 재실행하지 않으므로 활성화 전에 결과를 확인할 수 있다.

관리 API catalog에는 71개 management capability가 있으며 전체 API에는 88개 route가 있다. master 전용 first-party action admission API는 management catalog에서 제외된다. management route를 추가하는 경우 generated management catalog를 다시 게시하고 필요한 route 권한을 부여해야 gateway가 요청을 허용한다. UI가 이를 자동으로 넓게 승인하거나 권한을 대신 만들지 않는다.

현재 UI는 감사 추적이나 유효 권한 설명을 제공하지 않는다. 작업 결과만으로 감사 기록 또는 권한 전개 설명이 제공된다고 간주하면 안 된다.
