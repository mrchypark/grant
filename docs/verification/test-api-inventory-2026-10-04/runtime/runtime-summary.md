# IED 테스트 배포 런타임 증거

2026-10-04에 명시한 컨텍스트 `gke_patch2-the-new-era_asia-northeast3_ied-cluster`로 접근해 현재 배포와 활성 레지스트리를 확인했습니다. 주요 증거의 관찰 범위는 UTC 11:37:03–11:46:34, KST 20:37:03–20:46:34이며, 파일마다 실제 관찰 시각과 수집 방법 및 출처를 기록했습니다. 클러스터와 업무 상태는 변경하지 않았습니다.

## 현재 배포와 접근 주소

현재 대상은 Active 상태의 `patch-data-grant-exposure-go-v1` 네임스페이스입니다. 이름에 `grant|exposure|patch`가 포함된 네임스페이스만 조회한 결과 `patch`, `patch-anomaly`, `patch-data-grant-exposure-go-v1`, `patch-notification`이 발견됐습니다. 과거 검증의 명시된 대상에서 Grant voter·learner, Exposure data·management, Patch Data, Patch Auth가 모두 실행 중임을 확인했으므로 이 배포를 선택했습니다. 다른 네임스페이스의 워크로드는 조사하지 않았으므로 클러스터 전체에서 유일한 관련 배포라는 뜻은 아닙니다.

Deployment `data-stack`, `grant-voter`, `patch-auth`는 각각 desired 1, Ready 1이며 observedGeneration이 현재 generation과 일치합니다. 실행 중인 Pod 3개는 모두 Ready이고, 주 컨테이너 6개도 모두 Ready이며 재시작 횟수는 각각 0입니다. 이 6개는 Litestream init sidecar를 포함하지 않은 애플리케이션 컨테이너 수입니다.

| 현재 Ready Pod | 주 컨테이너 |
| --- | --- |
| `data-stack-6d5b859ff9-pgmpc` | patch-data, grant-learner, exposure-data |
| `grant-voter-6bd88657fc-8j6vw` | grant-voter, exposure-management |
| `patch-auth-78ff85b9c5-tnwq6` | patch-auth |

목록에 남은 비실행 Pod 5개는 위 현재 Ready 집합과 구분합니다. Failed 3개는 `Evicted` 상태이며, Succeeded 2개는 종료된 Pod입니다. 이들을 현재 서비스의 Ready 복제본 부족으로 계산하지 않았습니다. 각 서비스 EndpointSlice에는 Ready endpoint가 1개 있습니다.

| 접근 목적 | 클러스터 내부 HTTP base URL | Service → 실제 포트 |
| --- | --- | --- |
| 데이터 Exposure | `http://exposure.patch-data-grant-exposure-go-v1.svc.cluster.local:8082` | exposure 8082 → 8082 |
| Grant learner·데이터 PEP catalog | `http://grant.patch-data-grant-exposure-go-v1.svc.cluster.local:8080` | grant 8080 → 8086 |
| 관리 Exposure | `http://grant-management.patch-data-grant-exposure-go-v1.svc.cluster.local:8084` | grant-management 8084 → 8084 |
| Patch Auth native | `http://patch-auth.patch-data-grant-exposure-go-v1.svc.cluster.local:8091` | patch-auth 8091 → 8091 |
| Grant 내부 runtime | `http://grant-voter-runtime.patch-data-grant-exposure-go-v1.svc.cluster.local:8087` | runtime 8087 → 8087; peer UDP 8088 |

선택한 네임스페이스에는 Ingress 0개, Gateway 0개, HTTPRoute 0개가 있으며 Service 5개는 모두 ClusterIP입니다. 이 조사에서 확인한 외부 공개 base URL은 없습니다. 다른 네임스페이스의 공유 ingress나 클러스터 외부 경로까지 부재를 증명한 것은 아닙니다.

Patch Data native는 `data-stack` Pod 안의 8090에서 실행되며 별도 Service가 없습니다. Grant voter HTTP API는 voter 컨테이너의 `127.0.0.1:8080`에 바인딩되어 있습니다. Exposure data admin의 8083, management admin의 8085도 health/readiness용 Pod 포트입니다. 증거 수집에는 직접 소유한 loopback port-forward만 사용했고, 조사 후 모두 종료했습니다. 저장한 로컬 주소를 지속적인 배포 주소로 해석하면 안 됩니다.

## 이미지와 소스 커밋

| 컴포넌트 | 실행 이미지의 immutable digest | 소스 증거 |
| --- | --- | --- |
| Grant voter·learner | `sha256:2d5f035e4feab74cbfd662f68f799bea595852881df207fec64148851d459296` | OCI revision `16020fe78bc4a1908f8cc105f0a0c728f5bbce4e`; tag `20260922-16020fe` |
| Exposure data·management | `sha256:6af2e07288a8977ac707aff7ee4a5a9336ffabc9f5931140b952298e9703af1c` | tag `sha-d7807e6395513702ea958eaad35dfa5238f17136`; OCI revision label 없음 |
| Patch Data | `sha256:e0a3c463ab15b93297b6e3045d3fba3842bb5b8201cf4c0a6d0d10f2eb17113b` | OCI revision `b7258579b92808b2f1c56795034214946224a602`; tag `20260909-b725857` |
| Patch Auth | `sha256:89adab664fa7f7be9457d8a5d1b1e31ca890e618b84cbcced60ed89975dfb335` | 같은 digest에 아래 두 SHA 태그가 연결됨; OCI revision label 없음 |

Patch Auth의 태그는 `sha-5130e33e8457e94395ce69180faf863cbf2f8dd6`와 `sha-bc973ffe0c84a6f4728aa183890033f9d44f27ef`입니다. 커밋 하나를 확정하지 않았습니다. 해당 이미지 digest를 기준으로 두 소스 후보의 라우트 정의를 비교해야 합니다. Registry 태그는 변경 가능하므로 관찰 당시 digest와의 연결을 나타내는 증거이며, OCI revision label과 동일한 수준의 증거로 취급하지 않습니다.

로컬 Grant HEAD `3d9d4018bfc044421caffba4001901052c1569a3`는 배포된 Grant 커밋과 다릅니다. 배포 API를 설명할 때 로컬 HEAD의 문서나 소스만 사용해서는 안 됩니다.

## 현재 활성 카탈로그

ConfigMap 3개의 이름과 키만 확인했으며 catalog ConfigMap은 없었습니다. 활성 catalog는 현재 Grant에서 기존 PEP 서비스 자격증명으로 `GET /api/v2/http-capabilities/catalog/{providerId}`를 호출해 직접 수집했습니다. 자격증명은 파일이나 출력에 저장하지 않았습니다. 기존 master 자격증명으로 읽은 admin active catalog와도 generation, digest, capability 목록을 대조했습니다.

| Provider | generation | capability | enabled=true | 고유 method/path | response resource |
| --- | ---: | ---: | ---: | ---: | ---: |
| patch-data | 1 | 369 | 369 | 369 | 42 |
| grant-management | 5 | 69 | 69 | 69 | 0 |

데이터 digest는 `sha256:c731b83d70142bed04cc68e829cedc08acaa357645feb295097e25de371a99b4`이고 관리 digest는 `sha256:41a7d3ac23e7c3a4f820b553077cb9e01bd0eb38e9105b33196b7af8a1d66a2e`입니다. 두 값은 각 Exposure Deployment의 expected catalog digest와 일치합니다. 활성 capability는 합계 438개이며 이는 전체 native·internal API 수가 아닙니다.

데이터 method별 수는 GET 178, HEAD 10, POST 61, PATCH 60, DELETE 60입니다. 관리 method별 수는 GET 22, POST 14, PUT 15, PATCH 7, DELETE 11입니다. Provider actions 정의는 데이터 138개, 관리 76개입니다. 액션 정의 수와 method/path 계약 수는 서로 다른 집계입니다.

`catalog-data.json`과 `catalog-management.json`은 `.observation`과 `.catalog` 구조를 사용합니다. 계약 행은 `.catalog.capabilities[]`이며 `method`, `pathTemplate`, `enabled`, `contractDigest`, `actionId`, request/query/response 및 resource 계약을 보존합니다. 데이터의 `.catalog.responseResources[]`는 42개 컬렉션의 리소스 식별자와 카탈로그 관계 계약입니다. 이것을 현재 데이터베이스 필드 스키마 전체로 해석하면 안 됩니다.

## account·인증 영역과 문서 차이

선택한 배포에는 account Exposure 컨테이너나 Service가 없습니다. 기존 master로 조회한 `patch-auth`와 `patch-auth-account` Provider active catalog는 각각 404 `service provider not found`였습니다. 두 이름 외의 모든 Provider가 없다고 주장하지 않으며, 별도 account/auth 활성 카탈로그의 capability 수는 미확인으로 남깁니다.

Patch Auth native의 `GET /api/collections/users/auth-methods`는 200을 반환했습니다. password, OTP, MFA가 enabled이고 password identityFields는 email과 username입니다. OAuth2도 enabled이지만 응답 Provider ID 목록은 비어 있으므로 실제 외부 Provider 로그인이 가능하다고 단정하지 않습니다. provider URL, client 정보, verifier, 자격증명은 저장하지 않았습니다. 합성한 존재하지 않는 15자 사용자 ID `inventory000001`을 조회한 GET은 404였습니다. 로그인, token refresh, OTP 발송, 비밀번호 재설정은 수행하지 않았습니다.

`docs/registration-api-management.md`의 “추가 GET 3개를 IED에 배포하지 않았다”는 문장은 현재 배포를 설명하는 증거와 충돌합니다. 현재 management generation 5에는 아래 3개가 모두 enabled=true로 등록되어 있습니다.

- `GET /api/v1/admin/providers/{providerId}/actions`
- `GET /api/v1/admin/providers/{providerId}/http-capability-active`
- `GET /api/v1/orgs/{organizationId}/providers/{providerId}/actions`

앞의 두 GET은 실제 voter에 기존 master 자격증명으로 호출해 정의와 활성 catalog를 읽었습니다. 세 번째 GET은 합성 조직 ID와 조직 헤더만 전달했을 때 401 missing bearer token을 반환했습니다. 세 번째 GET의 정상 사용자 권한 응답은 검증하지 않았습니다.

2026-09-12의 route-summary는 과거 검증 결과입니다. 이번 데이터 capability 수가 과거와 같더라도 그 파일을 현재 registry 대신 사용하지 않았습니다. 특히 현재 management catalog는 generation 5의 69개이며 과거 README의 관리 경로 3개와 다릅니다.

## 읽기 전용 probe와 한계

`health-denial-probes.json`에는 19개 GET이 있습니다. health 6개와 readiness 4개는 모두 200이고, credential 없는 경계 검사 7개는 모두 401입니다. 잘못된 PocketBase ID의 422는 입력값 검증으로, 조직 헤더가 없는 요청의 400은 요청 문맥 검증으로 따로 기록했습니다. 데이터 인증 경계는 유효한 15자 소문자·숫자 합성 ID `inventory000001`로 다시 호출해 401을 확인했습니다.

Learner readiness는 readReady=true, writeReady=false이며 local read consistency를 표시합니다. Voter는 readReady=true, writeReady=true와 strong consistency를 표시합니다. 두 Exposure의 readiness도 true입니다. 별도 native account metadata GET 1개는 200, nonexistent account GET 1개는 404였으며 이 두 요청은 위 19개 집계에 포함하지 않았습니다.

438개 계약을 모두 정상 권한과 업무 입력값으로 호출한 검증은 아닙니다. 현재 활성 레지스트리와 대표 읽기·거부 응답을 확인한 조사입니다. 업무 변경, 권한 변경, 배포, OTP·이메일 전송은 하지 않았습니다.

현재 42개 업무 컬렉션의 필드 정의는 수집하지 못했습니다. 기존 Secret의 upstream-token으로 native `GET /api/collections?perPage=500`을 호출했지만 401을 반환했습니다. Exposure 컨테이너에는 `cat` 실행 파일이 없어 마운트된 런타임 토큰을 직접 읽는 시도도 실패했습니다. 새 로그인이나 refresh는 하지 않았으며 `collection-definitions-data.json`은 수집 실패 상태와 원인을 담습니다. 고객 레코드나 원본 데이터베이스는 수집하지 않았습니다.

## 증거 파일과 수집 방식

`runtime-status.json`은 워크로드 상태, 현재 Ready 집합, 과거 비실행 집합, 서비스 및 catalog 수를 통합합니다. `cluster-workloads.json`, `matching-namespaces.json`, `network-access.json`, `exposure-configuration.json`은 선택한 필드만 남긴 K8s 증거입니다. `image-*.json`은 immutable digest의 OCI metadata이고 `image-tags-*.json`은 digest로 제한한 Registry 태그입니다.

`catalog-admin-*.json`은 voter admin active 정의이고 `provider-actions-*.json`은 액션 정의입니다. `patch-auth-auth-methods.json`, `patch-auth-nonexistent-record.json`, `catalog-account-status.json`은 직접 인증 API와 account catalog의 확인 범위를 기록합니다. `verification.json`은 JSON과 수 및 catalog 일치 검사 결과입니다.

이 위임 작업은 추가 서브에이전트 호출 도구가 없어 직접 수행했습니다. 조사와 정형 수집에는 POSIX sh·kubectl·curl·jq 및 read-only OCI inspect를 사용했고, 한국어 작성에는 fluent-korean 지침을 적용했습니다. 임시 수집 스크립트만 사용했으며 영구 인덱스는 만들지 않았습니다. 반복할 경우 자격증명을 출력하지 않는 동일한 allowlist 기반 수집 절차를 재사용하는 것이 적절합니다. Yeoul은 사전 검색만 했고 공유 메모리에는 쓰지 않았습니다.
