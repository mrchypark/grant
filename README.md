# grant

`grant`는 조직, 애플리케이션, 서비스 provider와 resource scope에 대한 권한을 관리하는 Go 1.27 서비스다. 저장소는 Rhiza v0.19.0의 native Go API를 사용하며 cgo를 사용하지 않는다. 로그인과 외부 identity 확인은 Rauthy 또는 PocketBase(patch-auth)가 담당한다.

`organizations.id`가 tenant의 기본 키이며, `role_bindings`는 `provider_id + resource_type + resource_id` 범위에 role을 부여한다. 공개 권한 API는 allow/deny를 반환한다. 폐기된 field-view 기능은 활성화하지 않는다.

## 구성

Exposure 전용 판정과 고정 `plant.get` 계약, 권한 행을 반영하지 않던 V2 snapshot 수신 API·exporter는 폐기됐다. 현재 API는 55개 경로 패턴에 84개 메서드 항목을 등록하며, 관리 Exposure 카탈로그는 이 중 69개를 공개한다. 이 개수는 지원 경로 수이며 실환경 검증 완료를 뜻하지 않는다. [범용 HTTP capability](docs/http-capabilities.md)와 실제 권한을 반영하는 V1 import는 유지한다. 내부 Provider·Application 등록과 신규 조회는 [등록 API 관리](docs/registration-api-management.md)를 따른다.

이번 정리는 의도적인 비호환 변경이다. 이전 스키마 checksum을 가진 DB는 시작 시 거부되고 이전 backup도 현재 스키마와 호환되지 않으므로 새 DB로 시작해야 한다. 기존 데이터를 삭제하거나 자동 변환하지 않는다. 폐기한 snapshot 수신 경로는 404를 반환하고 exporter 실행 파일은 더 이상 제공하지 않는다.

- `internal/grant`: 설정, 모델, identity 검증, 권한 서비스, Rhiza 저장소와 스키마
- `internal/api`: HTTP route, JSON 계약, 인증 경계와 Rauthy proxy
- `cmd/grant`: 서버와 명시적 데이터 이전 명령
- `cmd/grant-patch-data-manifest-digest`: canonical manifest digest 도구

## 개발 검증

```sh
go test ./...
go vet ./...
CGO_ENABLED=0 go build ./cmd/...
sh scripts/verify-runtime-tools.sh
sh scripts/verify-rhiza-deployment.sh
```

로컬 3-voter 검증은 별도 Docker container와 임시 디렉터리를 만든다. 현재 Kubernetes context는 사용하지 않는다.

```sh
KEEP_GRANT_FIXTURE=1 sh scripts/verify-go-three-voter.sh
```

## 실행과 배포

[로컬 개발](docs/local-development.md), [런타임 구성](docs/runtime-modes.md), [데이터 이전과 복구](docs/backup-restore.md)를 먼저 확인한다. 기존 Hiqlite 디렉터리는 Rhiza 파일과 호환되지 않으므로 별도 경로로 명시적으로 이전한다.

Kubernetes 기본 배포는 3 voter를 유지하며, 같은 Rhiza object-store bucket과 prefix를 공유한다. 운영 환경은 `deploy/overlays/production`과 별도로 공급하는 `grant-rhiza-storage` Secret을 사용한다. `deploy/overlays/ephemeral`에는 폐기 가능한 로컬 S3 fixture가 포함된다. Compose는 명시적인 단일 voter 구성이고 새 `grant_rhiza_data` volume을 사용한다. 기존 `grant_data` volume은 삭제하거나 재사용하지 않는다.

[API 계약](docs/porting-go-http-contract.md), [권한 모델](docs/authorization-model.md), [스키마](docs/database-schema.md)를 함께 관리한다. 스키마나 릴레이션 의미가 바뀌면 스키마 문서도 갱신한다. 이전 Rust/Hiqlite 운영 문서는 [history/hiqlite](docs/history/hiqlite)에 보존한다.
