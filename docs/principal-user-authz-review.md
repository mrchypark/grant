# 사용자 독립 인가 설계 리뷰 보고서

- 상태: **단계별 구현 사용자 승인 · Exposure 합의 문서 반영 · 구현 검증 대기**
- 대상: [principal-user-authz-spec.md](principal-user-authz-spec.md)
- 범위: 문서 계약과 로컬 소스의 정합성이다. 구현·빌드·런타임은 평가하지 않았다.

## 초기 리뷰의 출처와 한계

상위 세션은 EGM 리뷰 결과로 Major 수정이 필요하다고 전달했다.
초기 두 fresh persona의 전체 문서 리뷰 기록은 이 세션에 제공되지 않았다.
따라서 두 독립 리뷰를 직접 수행했거나 원문 전체를 확인했다고 기록하지 않는다.
아래 두 관점은 전달된 지적을 분류하는 기준이며 새로운 에이전트 실행 결과가 아니다.

- 관점 A: 인증·권한 경계. 사용자 증명, audience, principal digest, 요청 결속을 검토한다.
- 관점 B: 운영·일관성. 관리 CAS 복구, voter snapshot, migration, 후속 작업을 검토한다.

## 통합 지적 6개와 수정 추적

| ID | 초기 심각도·문제 | 수정한 계약·위치 | 문서 재점검 |
|---|---|---|---|
| F1 | Major: runtime catalog로 inactive PEP의 CAS token을 얻을 수 없다. | ‘기존 경로 재사용과 CAS’: 같은 관리 경로에 master/private GET 하나, 상태·ETag, missing 404, 생성 If-None-Match, 정확한 strong ETag 공식을 정의했다. | 전달 지적 해소 |
| F2 | Major: 사용자 digest와 단일 audience 의미가 불명확하다. | ‘사용자 principal’·‘계약 결속’: 기존 EXACT ONE audience 검사를 보존하고 ExposurePrincipalDigest(u,org,"")의 정확한 bytes를 정의했다. | 전달 지적 해소 |
| F3 | Major: voter 전체 요청 snapshot을 보장할 구체적 경로가 없다. | ‘계약 결속’: 직접 linearizable SELECT 1의 AppliedSlot → readAtFence → QueryRaw read scope를 명시했다. 모든 authority 조회에 같은 slot, 최대 3회·총 3초를 적용한다. | 전달 지적 해소 |
| F4 | Major: stale revision 검출이 응답에만 의존하고 이미 실행한 write가 재생될 수 있다. | ‘최소 필드’·‘계약 결속’: 입력 pepPolicyRevision을 추가하고 같은 snapshot에서 비교한다. 후속 stale은 거부하며 새 작업만 새로 인가한다. | 전달 지적 해소 |
| F5 | Major: 과거 schema checksum 변경과 revision 저장 범위가 위험하다. | ‘마이그레이션’: 과거 SQL·checksum 보존, 다음 번호 append-only migration, legacy backfill, enum·NOT NULL·typeof integer·양의 int64·overflow 거부를 명시했다. | 전달 지적 해소 |
| F6 | Major: UserInfo 성공을 사람 사용자 보장으로 오인할 수 있다. | ‘사용자 principal’: issuer 운영자 계약과 사용자 positive·client-credentials negative 벡터를 활성화 전 필수로 정했다. 부재 시 해당 배포의 user 모드는 미지원이다. | 전달 지적 해소 |

F2에는 별도로 전달된 복수 audience 지원 금지와 digest 공식을 통합했다.
모드 null은 legacy 저장값만 application으로 backfill하고 현대 PUT에서는 거부한다.
사람 증명은 새 승인 서비스·플래그가 아니며 issuer 소유자가 확인해야 할 배포 조건이다.

## 소스로 확인한 근거

- `auth_goauthy.go`: goauthyAudiences는 길이 1과 설정 resource의 정확한 일치를 요구한다.
- `service_core.go`: 기존 digest는 domain·각 문자열의 BE64 byte 길이·bytes를 순서대로 hash한다.
- `service_replica.go`: localReplicaRead는 voter를 우회하지만 readAtFence 자체는 최대 3회를 수행한다.
- `runtime.go`: QueryRaw의 기존 read scope는 replica와 s.db.Query 모두에 AppliedSlot 검사를 적용한다.
- `service_replica.go`: serveReadFence는 직접 s.db.Query의 linearizable SELECT 1로 fence를 얻는다.
- `store_schema.go`: 현재 migration은 1..5이며 native migration checksum은 불변 계획에 의존한다.
- `service_capability_evidence.go`: 기존 sha256Text의 결과에는 sha256: 접두사가 있다.

## 최종 문서 재점검과 남은 상태

수정 후 설계 전체를 다시 읽고 전달된 6개 지적에 대한 수정 추적을 확인했다.
문서 재점검 범위에서는 F1..F6을 해소로 기록한다. 런타임 PASS를 뜻하지 않는다.
사용자 모드의 기대 revision·응답 echo·원래 후속 revision과 재생 금지 조건이 서로 일치한다.
voter의 직접 fence와 기존 함수 재사용을 명시했으며 Local loopback 네트워크는 요구하지 않는다.
이 항목은 Exposure 합의 반영 전의 문서 재점검 기록이다. 현재 승인·합의 상태는 아래 최신 메모를 따른다.
당시 두 fresh persona의 최종 독립 재리뷰는 아직 확인되지 않았으며, 후속 전달 기록은 다음 절에 구분한다.
issuer 계약·운영자 벡터·양쪽 릴리스 capability 2 적격성도 아직 확인되지 않았다.
구현·빌드·런타임·상호운용 시험은 실행하지 않았으며 테스트 통과를 주장하지 않는다.

## 최종 독립 재검토

2026-11-04의 기존 전달 기록에 따르면, 당시 수정 명세 261행 전체를 두 fresh-context 리뷰어가 다시 읽었다. 이번 문서 작업에서 해당 리뷰를 재실행하거나 원문 기록을 새로 확인한 것은 아니다.

- 이해도 리뷰: CLEAR. CAS 조회, 정확한 principal digest, snapshot 절차, 후속 revision 변경 처리의 기존 Major 4건 해소.
- 설계·구현 준비 리뷰: CLEAR. 관리 GET, 사용자 발급 정책의 증거 조건, voter snapshot 경계, append-only migration의 기존 Major 4건 해소.
- 서로 겹친 지적은 본 보고서의 통합 항목에 매핑한다. 전체 구현·프로토콜 호환·운영 시험 통과를 뜻하지 않는다.
- 당시 남아 있던 작성자 승인 상태는 아래 최신 메모로 갱신한다. Goauthy 사용자 발급 계약 증거와 Grant·Exposure 양쪽 구현/계약 벡터 실행은 별도로 필요하다.
- 독립 리뷰는 fork_context:false로 시작했다. 호스트 AGENTS.md의 배제까지 입증한 것은 아니다.

## 최신 Exposure 합의 반영 메모

사용자는 단계별 구현을 승인했다. 이번 작업은 명세와 이 보고서만 수정했으며 코드·빌드·런타임·상호운용 시험을 수행하지 않았다. 상위 세션이 구현 후 문서를 검토한다. 앞선 CLEAR는 당시 설계에 대한 전달 기록이며 이번 변경이나 구현의 PASS로 확대하지 않는다.

- 첫 연동은 기존 compact JWT Goauthy 사용자 토큰이다. Exposure opaque 지원 확장·JWKS·서명 검증은 추가하지 않는다. 힌트는 신뢰하지 않고 Grant introspection·UserInfo 결과로 대조한다.
- legacy application catalog·decision은 새 필드를 전부 생략한다. user catalog만 mode·revision을 제공하며 user decide는 wireVersion 2와 pepPolicyRevision을 요구한다. application v2는 새 decoder가 명시적으로 지원할 때만 허용하고 revision도 검사한다.
- mode·revision은 인증된 private envelope에 결속한다. catalog/contract hash와 기존 digest 공식은 변경하지 않는다. Grant가 내부 ID 기반 principalDigest를 계산하고 Exposure는 형식 및 같은 작업의 initial/follow-up 일치를 확인한다.
- user에서 application으로 전환하는 rollback은 관리자의 명시적 CAS만 허용한다. 자동 null fallback은 없다.
- 기존 키는 전역 GRANT_APP_AUTH_KEY다. configured PEP ID와 provider registry, lifecycle 검사를 유지하며 provider별 키 발급을 추가하지 않는다. 이전의 provider별 키라는 해석을 정정한다.
- 현재 GRANT_GOAUTHY_APPLICATION_ID 필수 문자열은 application 레코드 생성이나 IdP client 등록을 뜻하지 않는다. user 모드는 앱 레코드·앱 binding·trust를 추가하지 않는다.
- store 담당자의 typed mode·revision·append-only migration 작업은 수정하지 않았다. record scope도 앱 admission만 분리하고 사용자·조직·각 record RBAC를 유지하는 서비스 범위로 명세에 추가했다.

legacy strict-parser 호환, application v2 revision 검증, token hint 불일치 거부, initial/follow-up 결속은 앞으로 입증할 수용 조건이다. 구현 완료·시험 통과·배포 적격성은 아직 판정하지 않는다.
