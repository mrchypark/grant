# Principal 인가 설계 참조

원문 확인일: **2026-11-04 (Nov 4)**. RFC Editor의 RFC 4개와 Envoy 공식 문서 2개를 `curl -fsSL`로 직접 조회했다. 아래의 ‘설계에 사용’은 채택할 개념이며, 구현 완료나 표준 준수 판정을 뜻하지 않는다.

## 참조 표

| 1차 출처와 정확한 절 | 원문의 메커니즘 | 설계에 사용 | 채택하지 않는 범위 |
|---|---|---|---|
| [RFC 7662 §2.2 — Introspection Response](https://www.rfc-editor.org/rfc/rfc7662.html#section-2.2) | `active`는 필수이며 `sub`, `client_id`, `aud`, `iss`, `scope` 등은 선택 항목이다. 비활성 응답은 `active: false`이며 추가 정보는 제공하지 않는 것이 권고된다. | opaque Goauthy 토큰은 신뢰하는 검증 결과로 해석한다. 토큰 활성 여부, 사용자 식별, Grant application admission을 서로 다른 판단으로 구분한다. | `active: true`만으로 사용자 신원이나 Grant application의 위임 행위 권한을 확정하지 않는다. 선택 항목을 RFC의 필수 항목으로 바꾸거나 JWT 형식을 요구하지 않는다. 전체 introspection 프로토콜 준수를 주장하지 않는다. |
| [RFC 8707 §2 — Resource Parameter](https://www.rfc-editor.org/rfc/rfc8707.html#section-2) | `resource`는 접근 대상의 식별자다. 발급 서버는 대상에 맞는 audience 제한을 적용하는 것이 권고되며, 제한은 JWT의 `aud` 또는 introspection 응답의 `aud`로 전달할 수 있다. | ‘누구인가’와 ‘토큰 수신 대상(resource server/API)이 어디인가’를 분리한다. audience는 서버가 신뢰하는 토큰 수신 대상과 대조한다. 이는 Grant application admission과 별개다. | OAuth 발급 요청에 `resource`를 추가하거나 발급 서버를 변경하는 작업은 포함하지 않는다. 임의의 앱 ID를 RFC의 URI 파라미터와 동일시하지 않는다. |
| [RFC 9068 §2.2 — Data Structure](https://www.rfc-editor.org/rfc/rfc9068.html#section-2.2), [§4 — Validating JWT Access Tokens](https://www.rfc-editor.org/rfc/rfc9068.html#section-4) | JWT 액세스 토큰 프로파일에서 `sub`와 `client_id`를 구분한다. 사용자 관여 grant의 `sub`는 사용자, client credentials grant의 `sub`는 클라이언트 앱을 가리키는 것이 권고된다. §4는 프로파일의 `typ`, 발급자, audience, 서명, 만료 검증을 규정한다. | 사용자 principal, 호출 클라이언트, 토큰 수신 대상(resource server/API)을 별개 개념으로 정의한다. `sub`라는 이름만으로 사람이라고 단정하지 않고 검증된 토큰 종류와 발급 맥락을 확인한다. | **opaque Goauthy 토큰에 RFC 9068 JWT 프로파일을 강제하지 않는다.** 필수 JWT claim, `at+jwt`, JWKS, 로컬 서명 검증을 일괄 도입하지 않는다. |
| [RFC 8725 §3.8 — Validate Issuer and Subject](https://www.rfc-editor.org/rfc/rfc8725.html#section-3.8), [§3.9 — Use and Validate Audience](https://www.rfc-editor.org/rfc/rfc8725.html#section-3.9) | JWT의 발급자와 키의 결속, 유효한 subject 또는 발급자–subject 쌍을 검증한다. 같은 발급자가 여러 대상에 JWT를 발급하면 audience를 포함하고 수신자가 검증해야 한다. | 신뢰하는 발급 경로에서 검증한 발급자와 subject의 쌍을 principal로 사용하고, 토큰 수신 대상(resource server/API) 검증을 별도 조건으로 둔다. opaque 토큰에는 이 분리 원칙을 설계상 참고한다. | 이 JWT 규범을 opaque 토큰에 직접 적용하거나, subject·audience 확인만으로 JWT 검증 전체가 충족된다고 주장하지 않는다. |
| [Envoy — External authorization architecture](https://www.envoyproxy.io/docs/envoy/latest/intro/arch_overview/security/ext_authz_filter) | 트래픽 문맥을 외부 인가 서비스에 전달한다. 서비스 장애 시 허용 여부는 `failure_mode_allow`로 정하며 기본값은 `false`다. | 기존 PEP가 요청을 집행하고 별도의 판단 결과를 받는 역할 분리를 참고한다. 서버 모드와 실패 처리 정책은 명시적인 계약으로 둔다. | Envoy 배포, 별도 프록시 계층, gRPC 서비스 도입을 요구하지 않는다. Envoy의 기본값이 기존 시스템에도 이미 적용되어 있다고 가정하지 않는다. |
| [Envoy — HTTP External Authorization filter](https://www.envoyproxy.io/docs/envoy/latest/configuration/http/http_filters/ext_authz_filter) | HTTP 필터가 외부 HTTP 또는 gRPC 서비스에 인가를 질의하고 요청의 허용·거부를 집행한다. 메타데이터 전달도 지원한다. | 기존 PEP와 기존 HTTP API를 그대로 설계의 접점으로 삼아, 검증된 사용자 문맥과 Grant application의 위임 행위 자격·동의 판단을 함께 다룬다. | **ext_authz는 설계상의 영감이며 wire compliance를 주장하지 않는다.** Envoy `CheckRequest`/`CheckResponse`, 헤더 변환, 상태 코드 규약과의 호환성을 약속하지 않는다. |

## 설계에 반영할 경계

- **사용자 principal**은 검증된 발급자와 subject의 쌍 `(issuer, sub)`으로 항상 식별해야 한다(MUST). 단일 발급자 환경에서도 이 쌍을 유지한다. 이는 Grant 설계의 요구사항이며 RFC가 opaque 토큰에 직접 부과하는 규범은 아니다. Grant application admission 결과나 `client_id`로 사용자 신원을 대신 만들지 않는다.
- **Grant application admission**은 Grant가 관리하는 특정 application 범위에서 호출 클라이언트가 사용자 대신 요청한 위임 행위를 수행할 자격과 필요한 사용자 동의를 갖추었는지에 대한 추가 인가 판단이다. 토큰 수신 대상(resource server/API)의 audience 검증과 별개이며, 올바른 audience나 토큰 활성 여부, 사용자 식별 성공만으로 충족되지 않는다. 이 판단만으로 개별 리소스 권한까지 보장하지 않는다.
- **단순한 접점**은 기존 PEP와 기존 HTTP API다. 서버가 명시적으로 선택한 모드에 따라 사용자 principal이 필요한지, Grant application admission을 어떻게 평가하는지 정한다. 토큰 모양이나 claim 누락을 보고 더 약한 모드로 자동 전환하지 않는다. 구체적인 모드 이름과 HTTP 필드는 이 참조 조사에서 정의하지 않는다.

## Goauthy v0.3.9 사용자 발급 계약: OWNER-VERIFIED

2026-11-05에 전달된 Goauthy 소유자 세션 `01a0e3c9`의 확인을 기록한다. 근거는 공식 저장소의 발행 태그 [Goauthy v0.3.9](https://github.com/mrchypark/goauthy/tree/v0.3.9)에 대한 소유자의 소스 확인과 기존 historical CI 결과다. 이 문서 담당자는 Goauthy 파일을 직접 조사하거나 시험을 재실행하지 않았다. 아래 파일·행과 시험 결과는 모두 **OWNER-VERIFIED**로 인용하며, 새로운 cloud 실행 결과가 아니다.

| OWNER-VERIFIED 근거 | 소유자가 확인한 동작 | 사용자 증명의 의미 |
|---|---|---|
| v0.3.9 `server.go:644–646` | `client_credentials`에서 `openid`를 요청하면 400으로 거부한다. | machine 토큰에 사용자용 `openid`를 부여하지 않는다. |
| v0.3.9 `userinfo.go:33–95` | UserInfo는 유효한 access token, `openid`, 저장된 session의 비어 있지 않은 `Subject`, 활성 user를 요구한다. | JWT claim만이 아니라 저장된 사용자 session과 활성 사용자 계정을 확인한다. |
| v0.3.9 `machine.go:19–67` | machine JWT의 `sub=clientID`는 저장된 session의 `Subject`를 채우지 않는다. `clientID=userID` 문자열 충돌도 이 조건을 충족시키지 않는다. | machine subject를 사용자 계정 증명으로 오인하지 않는다. |

기존 시험에 대한 소유자 확인도 같은 태그의 근거에 포함된다.

- `UserInfoValidGETAndPOST`: 사용자 토큰의 GET·POST가 200이다.
- `UserInfoRejectsInvalidTokenAndTransport`: machine·expired·revoked·non-openid 토큰을 401로 거부한다.
- `OIDCDoesNotIssueWithoutOpenIDOrForClientCredentials`: `openid` 누락 또는 `client_credentials`의 OIDC 발급을 400으로 거부한다.

여기서 ‘human proof’는 **사용자 계정인 resource owner의 토큰임을 확인하는 증거**다. 사람이 현재 키보드 앞에 있다는 증명이나 실시간 사용자 참여·현존 증명을 뜻하지 않는다. v0.3.9의 사용자 발급 계약 선행 조건은 소유자 확인으로 충족되었다. 실제 신뢰 issuer 배포가 어느 버전인지와 해당 배포의 cloud PASS는 여전히 미검증이다. 이 근거로 새 trust 플래그·client·application을 추가하지 않는다.

## 출처와 확인의 한계

이 문서는 위 6개 1차 출처의 지정 절과 별도로 전달된 OWNER-VERIFIED v0.3.9 계약 근거를 구분한다. RFC 7662의 선택 메타데이터가 실제 운영 배포 응답에 존재하는지는 확인하지 않았다. 원래의 출처 조사에서는 다른 저장소 파일을 조사하거나 구현·런타임·상호운용 시험을 수행하지 않았다. Envoy의 `latest` URL은 변경될 수 있으므로 내용 확인 시점은 위 날짜로 한정한다.

후속 상태로 Grant PR #14가 merge되어 [v0.8.0](https://github.com/mrchypark/grant/releases/tag/v0.8.0)과 Exposure v0.3.0이 발행되었으며 [최종 PR CI](https://github.com/mrchypark/grant/actions/runs/41950428624) 및 [단계 1 process interop을 포함한 전체 수동 CI](https://github.com/mrchypark/grant/actions/runs/41950457179)가 통과했다. 이는 출처 조사 당시의 pending CI 상태를 갱신하는 근거다. fixture는 통제된 issuer 응답을 사용하며, Goauthy 소유자가 확인한 v0.3.9 계약과 별개로 실제 issuer 배포 버전·cloud 동작, live IED 및 단계 2 remote learner는 여전히 미검증이다. 이번 문서 갱신에서 시험을 재실행하지 않았다.
