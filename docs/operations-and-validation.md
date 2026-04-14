# 운영과 검증

이 문서는 compose 예시, Keploy 기반 외부 시스템 회귀, `vind` chaos 토폴로지처럼 운영과 검증에 가까운 내용을 모은다.

## production compose 예시

릴리즈용 이미지는 별도 compose와 env 파일 예시를 사용한다.

- `docker-compose.prod.yml`
- `.env.prod.example`

시작 전 `.env.prod.example`를 복사해 실제 값으로 채운다.

```bash
cp .env.prod.example .env.prod
docker-compose -f docker-compose.prod.yml --env-file .env.prod up -d --build
```

이 구성은 다음을 전제로 한다.

- `certd`는 `8080`
- 내부 `trailbase`는 외부에 publish하지 않음
- MinIO bucket은 `cert-artifacts`, `cert-backups`
- 운영 비밀값은 `.env.prod`에서만 주입

## Keploy 외부 시스템 E2E

`certd`의 핵심 회귀 리스크는 `POST /sync`가 `kcert.energy.or.kr`, `rps.energy.or.kr`를 직접 호출하는 구간이다. Keploy는 이 흐름을 한 번 실호출로 기록하고, 이후에는 외부 시스템 없이 replay test를 돌리는 용도로 붙인다.

원칙:

- `record`는 실제 공동인증서 로그인과 RPS 조회를 한 번 수행한다.
- `test`는 기록된 testcase/mock으로 replay만 수행한다.
- JSON 산출물 파일은 만들지 않는다.
- PDF는 runtime artifact 저장소에만 저장한다.

자산:

- 설정: `.keploy/keploy.yml`
- record: `scripts/keploy-record.sh`
- replay: `scripts/keploy-test.sh`
- k8s replay: `scripts/keploy-k8s-test.sh`
- 시나리오 드라이버: `scripts/keploy-scenario.sh`

준비:

- Docker daemon
- `certd:local` 이미지
- native Keploy CLI
- `./secrets/cert` 아래 공동인증서 4개 파일
- `CERT_PASSWORD` 환경변수

설치:

```bash
curl -sL https://keploy.io/install.sh | bash -s -- -platform zsh -noRoot
~/.keploy/bin/keploy --help
```

record:

```bash
export CERT_PASSWORD='...'
bash ./scripts/keploy-record.sh
```

replay:

```bash
bash ./scripts/keploy-test.sh
```

k3d 또는 `vind`에 이미 배포된 `certd`를 상대로 replay만 돌리려면:

```bash
bash ./scripts/keploy-k8s-test.sh
```

시나리오만 직접 실행하려면:

```bash
export CERT_PASSWORD='...'
bash ./scripts/keploy-scenario.sh
```

## vind TrailBase + certd chaos topology

현재 로컬 검증은 `vind`가 cluster lifecycle을 소유하고, 그 안에서 `certd` 데이터 평면과 TrailBase 인증 평면을 분리해서 본다.

- `certd-voter`
  - `StatefulSet` 3개
  - `emptyDir`
  - quorum과 backup/restore 담당
- `certd-learner-{1,2,3}`
  - `Deployment` 3개
  - `split + learner`
  - read path 담당
- `trailbase`
  - 단일 `Deployment`
  - Litestream restore-then-run
  - MinIO로 local SQLite fast recovery
- `minio`
  - `cert-artifacts`
  - `cert-backups`

운영 흐름:

1. TrailBase user seed
2. `certd` bootstrap owner 연결
3. plant/list/read/sync e2e
4. quorum 및 auth cache chaos
5. `hiqlite` backup 또는 restore
6. TrailBase restore check

권장 명령:

```bash
bash ./scripts/ci-chaos-local.sh
```

phase를 수동으로 나눠 확인하려면:

```bash
bash ./scripts/vind-cluster.sh ensure-context
bash ./scripts/vind-exec.sh bash ./scripts/vind-bootstrap.sh
bash ./scripts/vind-exec.sh bash ./scripts/k3d-e2e.sh
bash ./scripts/vind-exec.sh bash ./scripts/vind-chaos.sh
```

## 현재 검증 기준

로컬 인증서 기준 실동작 검증 항목:

- `PUT /certificate` 성공
- `POST /sync` with `mode=full` 성공
- `POST /sync` with `mode=delta` 성공
- `GET /plants/{facilityCode}` 응답 정상
- `GET /generation`, `GET /rates`, `GET /rec`, `GET /documents` 응답 정상
- `GET /documents/{id}/download` PDF 응답 정상

restore와 disaster recovery 절차는 [backup-restore.md](./backup-restore.md)를 기준으로 본다.
