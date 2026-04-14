# certd 아키텍처

`certd`는 공동인증서를 사용해 `rps.energy.or.kr` 데이터를 수집하고, 수집 결과를 `hiqlite`와 PDF artifact 저장소로 분리 저장한다.

## 핵심 원칙

- 메타데이터는 `hiqlite`에 정규화해서 저장한다.
- JSON 산출물 파일은 영구 저장하지 않는다.
- PDF만 artifact 저장소에 둔다.
- read API는 저장된 결과만 조회한다.
- write, sync, admin 경로는 voter 노드에서만 처리한다.

## 코드 기준 구성 요소

- `crates/domain`
  - `Plant`, `SyncRun`, `GenerationRecord`, `RecRecord`, `HealthStatus` 같은 도메인 모델을 제공한다.
- `crates/infra`
  - 설정 로딩, 공동인증서 처리, RPS 수집기, `hiqlite` 런타임, artifact 저장소, 서비스 레이어를 가진다.
- `crates/api`
  - `/healthz`, `/api/v1/plants/*`, `/api/v1/organizations/*`, backup/admin, certificate gateway HTTP 라우트를 제공한다.
- `crates/app`
  - 환경변수로 설정을 읽고 `AppService`를 구성한 뒤, voter/learner 역할에 맞는 Axum 라우터를 띄운다.
- `crates/trailbase-auth-bridge`
  - 별도 TrailBase auth 서비스로 동작하며 `/api/certd/introspect`를 제공한다.
- `desktop-agent`
  - 로컬 루프백 HTTP 서버로 인증서 탐색, 사용자 승인, challenge 서명을 담당한다.
- `packages/certd-web-sdk`
  - 브라우저에서 `stored`/`ephemeral` certificate gateway 흐름을 조합한다.

## 요청 흐름

1. `crates/app/src/main.rs`가 설정을 읽고 `AppService`를 초기화한다.
2. `crates/api/src/lib.rs`가 공용 read 라우트와 voter 전용 write/admin 라우트를 조립한다.
3. `AppService`가 `infra::database::Database`와 `ArtifactStore`를 통해 메타데이터와 PDF를 각각 저장한다.
4. 인증 관련 경로는 내부 TrailBase auth bridge와 desktop agent, web SDK를 조합해 certificate gateway를 구성한다.

## 저장 구조

- 메타데이터
  - `hiqlite`
  - 실제 테이블과 관계는 [database-schema.md](./database-schema.md)에 정리한다.
- 문서 artifact
  - 기본 backend는 filesystem
  - 선택 backend는 S3 호환 object store

현재 문서 설계상 다음 원칙을 따른다.

- 권한 전개와 visibility는 전용 `effective_*` 테이블에 저장한다.
- business, plant, sync, generation, REC, documents 같은 핵심 조회는 모두 DB에서 바로 읽는다.
- PDF binary만 외부 저장소에서 내려받는다.

## 배포 토폴로지

권장 토폴로지는 다음과 같다.

- `certd-voter`
  - `StatefulSet` 3개
  - quorum write 경로와 public API 담당
- `certd-learner-{1,2,3}`
  - `Deployment` 3개
  - `split + learner` read replica
- `trailbase`
  - 별도 `Deployment`
  - auth/admin UI와 introspection 담당
- `minio`
  - PDF artifact와 `hiqlite` backup 저장소

요약 경계:

- public API는 voter quorum 경로를 직접 탄다.
- learner는 sidecar 또는 보조 read/authz 경로로 본다.
- auth 평면은 TrailBase bridge로 분리한다.

## 관련 문서

- certificate gateway 상세: [rps-certificate-gateway.md](./rps-certificate-gateway.md)
- 권한 모델 상세: [authorization-model.md](./authorization-model.md)
- 런타임 모드 상세: [runtime-modes.md](./runtime-modes.md)
- 백업/복구 상세: [backup-restore.md](./backup-restore.md)
