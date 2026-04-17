# certd

`certd`는 공동인증서로 `rps.energy.or.kr` 데이터를 수집하는 Rust 서비스다.

핵심 원칙:

- 발전량, 이용률, REC, 문서 메타데이터는 모두 `hiqlite`에 저장한다.
- JSON 산출물 파일은 영구 저장하지 않는다.
- PDF만 별도 artifact 저장소에 보관한다.
- 데이터 조회용 API는 저장된 결과만 읽고, 실제 동기화는 write 경로에서만 수행한다.

## Workspace

- `crates/domain`
  - 도메인 타입과 공용 모델
- `crates/infra`
  - 설정, 인증서 처리, 수집기, 데이터 접근, artifact 저장소, 서비스 레이어
- `crates/api`
  - Axum HTTP 라우터와 API 계약
- `crates/app`
  - `certd` 실행 바이너리
- `crates/trailbase-auth-bridge`
  - 별도 TrailBase auth bridge 서비스
- `desktop-agent`
  - macOS/Windows loopback desktop agent
- `packages/web-sdk`
  - 브라우저용 certificate gateway SDK
- `scripts/`
  - `vind` 기반 로컬 검증, bootstrap, chaos, backup/restore 보조 스크립트

## 빠른 시작

로컬 검증은 `vind` 경로만 공식 지원한다.

```bash
docker buildx build --load -t certd:local .
docker buildx build --load -f Dockerfile.trailbase-auth-bridge -t trailbase-auth-bridge:local .
bash ./scripts/ci-chaos-local.sh
```

세부 준비물과 phase별 실행 흐름은 [docs/local-development.md](./docs/local-development.md)를 본다.

## 문서

- [아키텍처](./docs/architecture.md)
- [권한 모델](./docs/authorization-model.md)
- [런타임 모드와 읽기 일관성](./docs/runtime-modes.md)
- [로컬 개발과 API smoke](./docs/local-development.md)
- [운영과 검증](./docs/operations-and-validation.md)
- [Certificate Gateway](./docs/rps-certificate-gateway.md)
- [데이터베이스 스키마](./docs/database-schema.md)
- [백업과 복구](./docs/backup-restore.md)
- [RPS 데이터 수집 정리](./docs/rps-data-acquisition-guide.md)
- [TrailBase Auth Bridge README](./crates/trailbase-auth-bridge/README.md)

## 참고

- `crates/app/src/main.rs`는 시작 시 스키마 마이그레이션을 수행하고, voter/learner 역할에 따라 라우터를 선택한다.
- `crates/api/src/lib.rs`는 공용 read 라우트와 voter 전용 write/admin 라우트를 나눠서 노출한다.
- 스키마를 바꾸면 반드시 [docs/database-schema.md](./docs/database-schema.md)도 같이 갱신해야 한다.
