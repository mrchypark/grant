# K3d Certd TrailBase Chaos Design

목표는 `k3d` 환경에서 `certd`를 실제 목표 토폴로지인 `3 voter + 3 learner_only`로 운영하고, 인증 계층인 `TrailBase`를 별도 단일 서비스로 붙여 장애 모델을 명확히 검증하는 것이다.

핵심 전제는 다음과 같다.

- `certd`는 고가용성 대상이다.
- `TrailBase`는 고가용성보다 빠른 복구 대상이다.
- `TrailBase`는 `PVC`를 쓰지 않는다.
- `TrailBase` 데이터는 로컬 SQLite에 기록하되 `Litestream`으로 MinIO에 지속 복제한다.
- `TrailBase` 재기동 시에는 `Litestream restore`로 최신 스냅샷을 복원한다.
- `certd`는 TrailBase introspection 결과를 짧게 캐시하므로, TrailBase 장애 중에도 이미 인증된 토큰은 일정 TTL 동안 API 요청을 계속 통과시킨다.
- 새로운 로그인과 refresh는 TrailBase 가용성에 의존한다.

이 설계는 `TrailBase`를 quorum 대상에 넣지 않고, `certd`와 인증 계층의 장애 성격을 분리하는 데 목적이 있다.

## 권장 토폴로지

권장 구조는 다음과 같다.

- `certd-voter`
  - `StatefulSet` 3개
  - `emptyDir`
  - `HQL_NODE_ID_FROM=k8s`
  - `CERTD_RUNTIME_MODE=embedded`
  - `CERTD_LOCAL_NODE_ROLE=voter`
- `certd-learner-{1,2,3}`
  - `Deployment` 3개
  - `emptyDir`
  - `CERTD_RUNTIME_MODE=split`
  - `CERTD_LOCAL_NODE_ROLE=learner`
  - `CERTD_REMOTE_NODES`는 voter quorum API endpoint를 가리킴
- `minio`
  - `StatefulSet`
  - artifact와 hiqlite backup, TrailBase Litestream backup을 모두 저장
- `trailbase`
  - `Deployment` 1개
  - SQLite 로컬 파일
  - init 단계에서 `Litestream restore`
  - 런타임 중 `Litestream replicate`
  - `certd`가 호출하는 `/api/certd/introspect` 제공

이 구조를 택하는 이유는 다음과 같다.

1. `certd` 데이터 plane은 Raft quorum으로 보호해야 한다.
2. learner는 read offload용이므로 quorum 밖에서 별도 실패 도메인으로 다루는 것이 맞다.
3. `TrailBase`는 인증 원천이지만, 제품 핵심 데이터는 아니므로 HA보다 빠른 복구와 운영 단순성이 더 중요하다.
4. `PVC` 없는 `TrailBase + Litestream`은 `k3d`와 실제 클러스터 모두에서 동일한 운영 모델을 유지하기 쉽다.

## 역할 분리

### certd

`certd`는 다음을 담당한다.

- 조직/권한/발전소/수집/문서 도메인
- hiqlite quorum
- learner 로컬 read
- TrailBase bearer token 기반 principal 확인
- introspection 결과 캐시

즉 `certd`는 계속 `AuthZ + domain`이다.

### TrailBase

`TrailBase`는 다음만 담당한다.

- 로그인 UI
- auth API
- refresh/session
- auth bridge
- 사용자 원본 레코드

즉 `TrailBase`는 `AuthN`이다.

## 인증 가용성 모델

이 설계의 핵심은 인증 계층의 장애를 제품 핵심 데이터 plane과 다르게 다루는 것이다.

정책은 다음과 같다.

- `TrailBase up`
  - 로그인 가능
  - refresh 가능
  - introspection 가능
  - `certd` API 정상
- `TrailBase down`
  - 새 로그인 불가
  - refresh 불가
  - 아직 cache TTL 안의 기존 bearer token은 `certd`에서 허용
  - cache TTL 이후에는 인증된 API도 실패

즉 목표는 `RTO 우선`이다.

이 모델을 채택한 이유는:

- `TrailBase` 장애 시 인증 서비스는 잠시 흔들릴 수 있지만,
- 이미 로그인한 사용자의 제품 사용을 곧바로 끊지 않기 위해서다.

## Litestream 모델

`TrailBase`는 `PVC`를 두지 않는다.

권장 동작:

1. init container 또는 startup 단계에서 `Litestream restore` 실행
2. 로컬 SQLite 파일이 비어 있거나 pod가 교체되면 MinIO의 최신 snapshot에서 복구
3. main process로 `TrailBase` 실행
4. sidecar 또는 동시 프로세스로 `Litestream replicate` 실행

MinIO bucket 예시:

- `trailbase-backups`

복구 목표:

- pod kill 후 수십 초 내 auth 복구
- 새 pod에서도 사용자/세션 원본이 복구됨

주의할 점:

- refresh/session의 의미를 어디까지 보존할지 확인해야 한다.
- TrailBase 세션 저장 방식이 SQLite 기반이면 restore 시 세션 복구가 가능하지만, TTL과 revoke semantics는 별도 확인이 필요하다.

## 카오스 시나리오

### 1. voter 단일 장애

행동:

- `certd-voter-0`, `1`, `2` 중 하나 삭제

기대 결과:

- quorum 유지
- write 가능
- strong read 가능
- learner read 가능

검증:

- `/healthz`
- plant list/detail
- sync delta
- backup trigger

### 2. leader 장애

행동:

- 현재 leader voter를 식별해 삭제

기대 결과:

- 재선출 발생
- 짧은 write interruption만 존재
- 재합류 후 정상

관측:

- leader 전환 시간
- write 실패 구간 길이

### 3. learner 단일/전체 장애

행동:

- learner deployment 재시작 또는 replica 0

기대 결과:

- read offload 감소
- quorum write 무영향
- learner 복구 후 local read 회복

### 4. voter 전체 재기동

행동:

- `StatefulSet` 전체 restart

기대 결과:

- quorum 재형성
- restore 없이 재기동 가능
- 필요시 backup/restore 경로 검증 가능

### 5. TrailBase pod kill

행동:

- `trailbase` pod 삭제

기대 결과:

- 새 로그인 실패
- cache TTL 이내 기존 bearer token API 성공
- TTL 경과 후 기존 token API 실패
- pod 복구 후 로그인 재개

이 시나리오가 현재 설계의 핵심이다.

### 6. TrailBase volume wipe

행동:

- pod 재생성 + 로컬 SQLite 유실 가정

기대 결과:

- Litestream restore로 auth 데이터 복구
- 로그인 및 `/api/v1/me` 재개

### 7. MinIO 장애

행동:

- MinIO pod 재시작 또는 정지

기대 결과:

- `certd` core API는 계속 동작
- artifact 다운로드/backup/Litestream replicate는 실패 가능
- MinIO 복구 후 backup과 replication 재개

주의:

- backup API가 실제 비동기 실패를 숨기지 않도록 관측을 강화해야 한다.

## 현재 설계에서 반드시 확인할 리스크

### 1. TrailBase는 인증 SPOF

이건 의도된 트레이드오프다.

대신 다음을 보장해야 한다.

- 빠른 restart
- restore 자동화
- 짧은 auth cache
- 명확한 장애 알림

### 2. cache TTL이 너무 짧거나 길 수 있음

- 너무 짧으면 TrailBase 짧은 장애에도 사용자 API가 즉시 실패한다.
- 너무 길면 revoke/disable 반영이 늦다.

따라서 TTL은 카오스 결과로 조정해야 한다.

### 3. MinIO가 auth 복구 경로의 의존성

`TrailBase restore`와 `Litestream replicate`가 모두 MinIO에 의존하므로,
MinIO 장애 시에는 auth 복구 능력도 떨어진다.

### 4. backup API 응답 의미 왜곡

현재 비동기 S3 push 실패가 즉시 surface되지 않으면 운영자가 성공으로 오해할 수 있다.

## 권장 성공 기준

최소 성공 기준은 다음과 같다.

- voter 1개 장애에서도 write 지속
- learner 전체 장애에서도 write 지속
- TrailBase 장애 중 새 로그인은 실패하지만 기존 token API는 TTL 동안 성공
- TrailBase 재기동 후 auth UI와 `/api/auth/v1/login` 복구
- TrailBase local DB 유실 시 Litestream restore로 복구
- MinIO 장애가 `certd` 핵심 read/write API를 직접 멈추지 않음

## 대안과 비채택 이유

### TrailBase 다중화

- 장점: auth SPOF 완화
- 단점: SQLite 기반 세션/스토리지 동기화가 복잡
- 지금 단계에서는 과함

### TrailBase + PVC

- 장점: restore 없이 재시작 가능
- 단점: 노드 종속성과 스토리지 운영 부담 증가
- PVC 없는 운영 모델을 원하므로 비채택

### certd가 TrailBase 없이 JWT 직접 검증

- 장점: auth cache와 인증 경계가 단순
- 단점: TrailBase 정책과 강결합
- 현재는 auth bridge 경로를 유지하는 편이 더 안전
