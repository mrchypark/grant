# scripts

이 디렉터리는 로컬 검증, Kubernetes 배포 보조, S3 복구 객체 처리, OCI 검증을 위한 명령과 그 보조 모듈을 둔다. 운영 환경을 바꾸는 명령은 명시한 자격 증명과 대상 컨텍스트가 필요하다. 다수의 `verify-*`는 임시 디렉터리·모의 명령·로컬 컨테이너를 쓰는 fixture이지만, `verify-oci-attestations.sh`는 GHCR을 읽고, `verify-ephemeral-k8s.sh live`는 지정된 Kubernetes 클러스터를 확인한다.

## 실행 진입점

| 명령 | 목적과 사용법 | 필요한 도구/입력 |
| --- | --- | --- |
| `ci-local.sh` | 로컬 vCluster에서 이미지 빌드, 부트스트랩, 런타임 검증을 차례로 실행한다. `bash scripts/ci-local.sh` | Bash, Docker, kubectl, curl, jq, vcluster; 기본값은 로컬 `grant-ci`와 `grant` 네임스페이스 |
| `vind-cluster.sh` | vCluster Docker 드라이버의 컨텍스트를 만들거나 연결·삭제·사전 점검한다. `bash scripts/vind-cluster.sh ensure-context\|delete\|preflight` | Bash, Docker, kubectl, vcluster; `CLUSTER_NAME`, `RESET_CLUSTER`, `KEEP_CLUSTER` |
| `vind-exec.sh` | 존재하는 VIND 클러스터 컨텍스트에서 명령을 실행한다. `bash scripts/vind-exec.sh COMMAND [ARG...]` | Bash, Docker, vcluster; `CLUSTER_NAME` |
| `vind-bootstrap.sh` | VIND 안에 Grant/Rauthy 리소스와 이미지, Secret을 적용한다. 보통 `vind-exec.sh`를 통해 실행한다. | Bash, Docker, kubectl, GHCR 자격 증명; `KUBECONTEXT` 또는 `VIND_EXEC_ACTIVE=1` |
| `gke-skaffold-run.sh` | 고정된 Rauthy digest를 사용해 IED Skaffold 배포를 실행한다. `bash scripts/gke-skaffold-run.sh` | Bash, Docker, kubectl, skaffold, GHCR 자격 증명, 명시적 `KUBE_CONTEXT` |
| `ghcr-tag-preflight.sh` | GitHub Actions에서 커밋 태그가 GHCR에 존재하는지 확인하고 `GITHUB_OUTPUT`에 기록한다. | POSIX sh, curl, jq; `SOURCE_SHA`, `GHCR_USERNAME`, `GHCR_TOKEN`, GitHub Actions 환경 변수 |
| `materialize-s3-restore.sh` | Rauthy의 특정 S3 버전을 불변 복구 객체로 materialize한다. `sh scripts/materialize-s3-restore.sh [--dry-run] rauthy BUCKET SOURCE_KEY VERSION_ID SHA256 RECOVERY_ID RETAIN_UNTIL WRITER_PRINCIPAL OUTPUT` | POSIX sh, AWS CLI, jq, mktemp; `grant`는 실제 materialize를 거부하지만 `--dry-run grant ...`는 계획 metadata를 쓴다. `S3_ENDPOINT_URL`은 테스트 endpoint에만 선택 가능 |
| `check-s3-object-history.sh` | 복구 대상 key의 버전/삭제 marker 이력을 확인한다. `sh scripts/check-s3-object-history.sh BUCKET TARGET_KEY [EXPECTED_VERSION_ID]` | POSIX sh, AWS CLI, jq, mktemp |
| `replica-members` | 비공개 Rhiza member JSON에서 voter token을 제외한 replica member 배열을 만든다. `scripts/replica-members < input.json` | Go; `ClusterID`와 `Members`가 있는 stdin JSON |
| `create-recovery-pair.sh` | 은퇴한 Hiqlite recovery-pair 명령의 명시적 실패 shim이다. 언제나 종료 코드 2를 반환한다. | POSIX sh; 역사적 호출 호환성만 유지 |
| `restore-full-cluster.sh` | 은퇴한 Hiqlite full-cluster 복구 명령의 명시적 실패 shim이다. 언제나 종료 코드 2를 반환한다. | POSIX sh; 역사적 호출 호환성만 유지 |

## 검증과 fixture

| 명령 | 검증 대상과 사용법 | 격리/의존성 |
| --- | --- | --- |
| `verify-independent-environment.sh` | Disposable independent unit + S3/GCS/Azure process DR entry point: `sh scripts/verify-independent-environment.sh [all\|unit\|dr]`, default `all`. | POSIX sh, Git, Go; DR also needs local Docker. See [verification environment](../docs/verification-environment.md). No cluster access. |
| `verify-independent-object-dr.sh` | All-three-provider clean-close process recovery and authority continuity. | Owned local Docker emulators, fake credentials, disposable data; retained evidence and verified cleanup. Called by the independent entry point. |
| `verify-runtime-tools.sh` | Dockerfile의 런타임 도구와 서버 entrypoint 계약을 확인한다. `sh scripts/verify-runtime-tools.sh [DOCKERFILE]` | 임시 디렉터리, POSIX sh |
| `verify-rhiza-deployment.sh` | 세 overlay의 Rhiza 배포 형태를 렌더링해 확인한다. `sh scripts/verify-rhiza-deployment.sh` | POSIX sh, kubectl |
| `verify-go-cli-contract.sh` | cgo 없이 두 Go 실행 파일을 빌드하고 stdout 계약을 확인한다. `sh scripts/verify-go-cli-contract.sh` | POSIX sh, Go |
| `verify-go-three-voter.sh` | 로컬 S3 호환 스토리지와 세 Grant 프로세스로 quorum, 복구, snapshot restore를 검증한다. `sh scripts/verify-go-three-voter.sh` | POSIX sh, 로컬 Unix Docker context, Go, curl, jq; 사용 중인 storage image는 스크립트에서 확인하고 `KEEP_GRANT_FIXTURE=1`을 선택할 수 있음 |
| `verify-go-container.sh` | 빌드된 AMD64 이미지의 보안 설정과 HTTP smoke를 확인한다. `IMAGE_NAME=... sh scripts/verify-go-container.sh` | POSIX sh, 로컬 Docker, curl, jq |
| `verify-ephemeral-k8s.sh` | `static` 기본값은 native Rhiza manifest와 fixture를 확인하고, `legacy-static`은 보존된 역사적 manifest 검사를 실행하며, `live`는 지정된 Kubernetes 클러스터의 disposable overlay를 확인한다. `sh scripts/verify-ephemeral-k8s.sh [static\|legacy-static\|live]` | POSIX sh; static/legacy-static은 kubectl, live는 클러스터와 관련 도구 |
| `verify-vind-bootstrap.sh` | VIND image import와 bootstrap 순서를 Docker mock으로 확인한다. `bash scripts/verify-vind-bootstrap.sh` | Bash, 임시 디렉터리 |
| `verify-lifecycle-values.sh` | lifecycle gate의 파일 값·FIFO 거부 계약을 fixture로 확인한다. `sh scripts/verify-lifecycle-values.sh` | POSIX sh |
| `verify-s3-object-history.sh` | S3 history 및 materialize 동작을 모의 AWS CLI로 확인한다. `sh scripts/verify-s3-object-history.sh` | POSIX sh, jq, 임시 디렉터리 |
| `verify-oci-attestations.sh` | GHCR에서 OCI provenance와 digest 계약을 읽어 확인한다. `sh scripts/verify-oci-attestations.sh` | POSIX sh, curl, Docker, jq, mktemp, openssl; GHCR 입력 변수와 네트워크 |
| `verify-oci-attestations-tests.sh` | OCI attestation verifier의 리다이렉트·자격 증명 처리 fixture를 실행한다. `sh scripts/verify-oci-attestations-tests.sh` | POSIX sh, Go, curl, jq, openssl |
| `verify-ghcr-tag-preflight.sh` | GHCR preflight의 HTTP·신호 처리 경로를 mock으로 검증한다. `sh scripts/verify-ghcr-tag-preflight.sh` | POSIX sh, 임시 디렉터리 |
| `verify-docs.sh` | 모든 Markdown 링크와 데이터베이스 스키마 문서 inventory를 확인한다. `sh scripts/verify-docs.sh` | POSIX 기본 명령, find, grep, sed, sort, tr, comm, mktemp |
| `oci-redirect-fixture` | OCI verifier 테스트가 쓰는 loopback TLS/HTTP redirect 서버다. 직접 실행 시 `--ready PATH --log PATH --body PATH`가 필요하다. | Go; `verify-oci-attestations-tests.sh` 전용 fixture |

## helper와 Go 모듈

| 파일 | 역할 | 호출자/의존성 |
| --- | --- | --- |
| `k8s-secret-helpers.sh` | GHCR pull secret과 Grant Secret을 파일에서 안전하게 적용하는 함수 모음이다. | `ci-local.sh`, `gke-skaffold-run.sh`, `vind-bootstrap.sh`; POSIX sh, Docker, kubectl |
| `vind-image-helpers.sh` | VIND containerd로 이미지 import와 오류 redaction을 수행하는 Bash 함수 모음이다. | `vind-bootstrap.sh`, `verify-vind-bootstrap.sh`; Bash, Docker |
| `cached-go-module.sh` | Go launcher가 모듈 해시별 캐시 바이너리를 빌드·실행하는 공용 실행 helper다. 첫 실행은 빌드하고, 같은 환경·소스의 다음 실행은 캐시를 재사용한다. | `oci-redirect-fixture`, `replica-members`; POSIX sh, Go, shasum 또는 sha256sum |
| `oci-redirect-fixture-src/` | `go.mod`, `main.go`로 이루어진 redirect fixture의 독립 Go module이다. | `oci-redirect-fixture`가 빌드한다 |
| `replica-members-src/` | `go.mod`, `go.sum`, `main.go`로 이루어진 Rhiza replica member 변환기의 독립 Go module이다. | `replica-members`가 빌드한다 |

## 통합 지점과 이번 정리

GitHub Actions의 `.github/workflows/e2e-local.yml`은 shell 문법/ShellCheck 목록과 로컬 검증 진입점을 관리하고, `rauthy-launcher-image.yml`은 GHCR preflight 및 OCI attestation verifier를 호출한다. 문서의 일반 로컬 진입점은 `README.md`, `docs/local-development.md`, `docs/runtime-modes.md`에 있다.

이번 정리는 서로 다른 두 Go 명령의 중복된 캐시 빌드 launcher만 `cached-go-module.sh`로 통합했다. 명령 이름, 각 Go module, 캐시 경로 형식, 환경 변수(`CODEX_GO_SCRIPT_CACHE`, `CODEX_GO_BUILD_PKG`, `CACHED_GO_REBUILD`, `CACHED_GO_DEBUG`)는 유지하고, 두 module의 `go build`는 모두 `-trimpath`를 사용하도록 맞췄다. 소스 파일·Go 환경·build package가 바뀌면 해시가 달라져 새 바이너리를 만들고, `CACHED_GO_REBUILD=1`은 강제로 다시 빌드한다. 실제 실행 경로에서 사용되지 않던 AWS principal helper와 그 전용 verifier도 함께 제거했다. VIND/GKE 배포 경로, S3 처리와 검증 fixture, 그리고 은퇴한 Hiqlite 명령 shim은 호출 계약이 달라 통합하지 않았다.

`sh scripts/verify-standalone-deployment.sh` renders the standalone overlay, checks
the release image/init/PVC contract, exercises rendered ConfigFromEnv through a
temporary Go overlay, and runs TestRestoreLocalStorageEntrypoint. It does not
build/import an image or apply to a cluster. Requires kubectl, yq, jq and Go.

`IMAGE_NAME=grant:container-verify sh scripts/verify-standalone-container.sh` runs
init/main, same-DataDir reuse and clean data-loss recovery through the real API.
Requires a local native AMD64 Docker daemon, curl and jq. It uses only owned data
volumes and an independent filesystem archive, removes its containers/volumes,
and preserves logs in `LOG_DIR` (or a printed temporary directory). Archive root
`/archive/root` and its sibling owner anchor share the writable `/archive` volume;
the container root filesystem remains read-only. It does not
establish cloud-provider parity or crash recovery.

`sh scripts/verify-standalone-container-ownership.sh` checks the container contract's
cleanup with a small fake Docker command: name/volume collisions, successful
creation followed by startup failure, and changed ownership. Requires POSIX sh;
it never contacts a Docker daemon. CI runs it before the real container contract.
