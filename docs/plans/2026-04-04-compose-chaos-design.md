# Compose Chaos Design

## Goal

`docker-compose.empty.yml` 및 `docker-compose.prod.yml` 기준의 로컬 단일-node 토폴로지에서,
실패를 숨기지 않고 아래를 검증한다.

- 어떤 장애에서 API가 즉시 중단되는가
- 어떤 장애에서 인증만 중단되는가
- 어떤 장애에서 object storage만 중단되는가
- 컨테이너 재기동 후 데이터와 상태가 어떻게 복구되는가

## Topology

- `certd`
  - 단일 프로세스
  - embedded `hiqlite`
  - 제품 API의 단일 진입점
- `trailbase`
  - auth provider
  - `/auth/*`, `/api/auth/v1/*`는 `certd`가 facade로 노출
- `minio`
  - artifacts + hiqlite backup object storage

이 구성은 HA가 아니다.
따라서 `certd` 장애에서는 API 전체가 중단되는 것이 정상이다.

## Failure Classes

### 1. `certd` restart / crash

기대:

- `/healthz` 실패
- `/auth/*` facade도 실패
- 제품 API 전체 중단
- 재기동 후 `plants`와 hiqlite 데이터는 volume에서 복구

관찰 포인트:

- 다운타임 길이
- 재기동 후 `GET /api/v1/me`
- 재기동 후 `GET /api/v1/plants`

### 2. `trailbase` restart / crash

기대:

- `certd` health 자체는 유지될 수 있음
- 신규 로그인은 실패
- introspection이 필요한 API는 실패
- 짧은 auth cache TTL 동안 일부 요청은 일시적으로 살아있을 수 있음

관찰 포인트:

- `/auth/login` 실패 여부
- 기존 bearer token으로 `/api/v1/me`가 즉시 실패하는지, cache 만료 후 실패하는지
- `trailbase` 복구 후 로그인 재개 여부

### 3. `minio` restart / crash

기대:

- `certd` health와 `plants`/profile read는 유지될 수 있음
- artifacts 업로드와 backup 생성은 실패
- 문서 다운로드는 object가 필요한 경우 실패 가능

관찰 포인트:

- `/healthz`
- `GET /api/v1/plants`
- `POST /api/v1/admin/backups`
- `GET /api/v1/admin/backups`

### 4. volume wipe

기대:

- `trailbase` volume wipe: auth users 사라짐
- `certd` volume wipe: organizations/plants/권한 사라짐
- `minio` volume wipe: artifacts/backups 사라짐

관찰 포인트:

- bootstrap 재실행 가능 여부
- empty stack 재구성 가능 여부

## Baseline Scenario

1. stack up
2. user seed
3. owner bootstrap
4. login
5. `/api/v1/me`
6. empty `plants`
7. plant create

## Chaos Scenario Set

### Scenario A: `certd` restart

1. baseline 성공 확인
2. `docker-compose restart certd`
3. `certd` health 회복 대기
4. 기존 token으로 `/api/v1/me`
5. `/api/v1/plants`

성공 기준:

- 재기동 후 상태 복구
- 생성한 plant 유지

### Scenario B: `trailbase` restart

1. baseline 성공 확인
2. `docker-compose restart trailbase`
3. 즉시 `/auth/login`
4. 기존 token으로 `/api/v1/me`
5. cache TTL 이후 다시 `/api/v1/me`
6. `trailbase` 복구 후 재로그인

성공 기준:

- 장애 동안 auth 경계가 명확히 실패
- 복구 후 재로그인 가능

### Scenario C: `minio` restart

1. baseline 성공 확인
2. `docker-compose restart minio`
3. `minio` health 회복 대기
4. `/api/v1/plants`
5. backup create/list

성공 기준:

- object storage 의존 기능만 영향을 받음
- 복구 후 backup 재개 가능

### Scenario D: full stack restart

1. baseline 성공 확인
2. `docker-compose down`
3. `docker-compose up -d`
4. login
5. `/api/v1/me`
6. `/api/v1/plants`

성공 기준:

- volume persistence가 유지되면 plant와 권한 복구

## Known Non-Goals

- quorum survival
- leader election
- learner read-only routing
- multi-node failover

이건 `k3d` / K8s HA 토폴로지에서 따로 검증해야 한다.
