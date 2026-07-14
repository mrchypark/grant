# grant 런타임 모드

`grant`는 `hiqlite` 접근 방식을 `embedded`, `remote`, `split` 세 가지로 나눈다.

## 모드

- `embedded`
  - 로컬 `hiqlite` 노드만 사용한다.
  - 개발과 단일 노드 운영용이다.
- `remote`
  - 로컬 상태 없이 원격 quorum cluster만 사용한다.
- `split`
  - 로컬 읽기용 client와 원격 quorum client를 같이 사용한다.
  - learner sidecar는 일반 조회/authz를 로컬로 처리하고, voter는 full write/admin surface를 가진다.

## voter 와 learner

`crates/app/src/main.rs`는 `GRANT_LOCAL_NODE_ROLE`에 따라 동작을 나눈다.

- `voter`
  - 기본 public API 노드다.
  - schema migration, initialize, write/admin 처리 가능
- `learner`
  - read replica 성격의 노드다.
  - 모든 runtime mode에서 schema migration과 bootstrap/initialize를 건너뛴다.
  - `/readyz`에서 현재 read 경로에 필요한 schema가 실제로 조회 가능한지 read-only로 확인한다.
  - `crates/api/src/lib.rs`에서 write/admin 라우트는 `requires_voter_node`로 막는다.

## 관련 환경변수

```bash
export GRANT_RUNTIME_MODE=embedded   # embedded | remote | split
export GRANT_LOCAL_NODE_ROLE=voter   # voter | learner
export GRANT_READ_CONSISTENCY=local  # local | strong | prefer-strong

export GRANT_REMOTE_NODES=127.0.0.1:18100,127.0.0.1:18110
export GRANT_REMOTE_TLS=false
export GRANT_REMOTE_TLS_NO_VERIFY=false
export GRANT_REMOTE_API_SECRET=AnotherSecureSecret42
export GRANT_REMOTE_WITH_PROXY=false
```

`GRANT_REMOTE_NODES`는 `host:port` 형식을 권장한다. 기존 `http://...`, `https://...` 값이 들어와도 앱에서 scheme를 제거하고 `hiqlite`에 전달한다.

## 읽기 일관성

- `local`
  - 로컬 SQLite 또는 로컬 learner 복제본 우선
- `strong`
  - quorum 강일관성 읽기
  - write 경로가 죽어 있으면 실패
- `prefer-strong`
  - quorum을 우선 시도하고 실패하면 로컬로 폴백

HTTP read 계약은 더 좁다.

- 기본값은 local read다.
- 읽기 요청에서만 `?consistency=strong`을 붙여 quorum read를 명시적으로 요청할 수 있다.
- `?consistency=strong`은 실패 시 실패하며, 로컬로 숨은 폴백을 하지 않는다.
- `consistency` query는 현재 `strong`만 허용하고 다른 값은 `400`이다.

## healthz와 readyz 의미

`GET /healthz`는 아래 상태를 같이 노출한다.

- `readReady`
  - 로컬 또는 대체 read 경로가 사용 가능한지
- `writeReady`
  - quorum write와 strong read 경로가 사용 가능한지
- `runtimeMode`
  - 현재 런타임 모드
- `defaultReadConsistency`
  - 기본 읽기 정책

`GET /readyz`는 같은 상태 body를 반환하지만, 사용할 수 있는 read 경로에서 필수 application schema를 read-only query로 확인한 경우에만 `200`을 반환한다. schema가 없거나 아직 복구/migration되지 않았으면 `503`이다. readiness 검사는 migration이나 bootstrap write를 수행하지 않는다. Kubernetes readiness와 lifecycle peer 확인은 `/readyz`를 사용하고 startup/liveness는 `/healthz`를 사용한다.

## 복구 보조 동작

`HQL_BACKUP_RESTORE`가 설정된 상태에서 `embedded` 또는 `split` 로컬 노드를 기동하면, 앱은 `HQL_DATA_DIR/state_machine/lock`의 stale lock 정리를 시도한다.

이 동작은 restore 직후 재기동 시 local lock 파일 때문에 노드 시작이 막히는 경우를 줄이기 위한 것이다.

Kubernetes production에서는 Grant와 Rauthy 모두 3 voter `emptyDir` 구성이다. StatefulSet ordinal `0`, `1`, `2`는 `HQL_NODE_ID_FROM=k8s`에 의해 hiqlite node ID `1`, `2`, `3`으로 대응한다. `HQL_NODES`의 DNS 순서와 이 대응은 변경하면 안 된다.

production init lifecycle gate는 다음 mode를 사용한다.

- `steady`: 빈 Pod는 다른 ordinal의 direct cluster-readiness가 확인되어야 시작한다. 값이 없거나 잘못되어도 이 mode다.
- `fresh`: local 개발 profile에서만 허용한다. production gate는 fresh bootstrap을 거부하고 승인된 전체 restore만 허용한다.
- `restore`: ordinal 0만 시작할 수 있으며 32자 이하 lowercase DNS recovery ID가 lifecycle state와 restore authorization에서 같아야 한다. Grant shell supervisor는 원자적 `pending/completed` marker와 제한된 로컬 `/healthz`·`/readyz` 확인으로 replay를 차단한다. 같은-ID stale `pending`은 authorization이 유지된 동안 재시도하고 `completed`만 일반 시작으로 전환한다. Rauthy static launcher는 `/app/data`의 원자적 `pending/completed` 상태와 연속 hiqlite `/ready` 확인을 사용하지만, Rauthy `pending` container restart는 자동으로 재실행하지 않는다. Secret이 없어도 같은 recovery의 `completed` 상태는 일반 시작할 수 있지만, authorization과 completed 상태가 모두 없으면 fail closed한다.

Lifecycle mode와 recovery ID 파일은 shell gate와 Rauthy launcher 모두 정확히 한 줄만 허용한다. 값 뒤에는 LF 또는 CRLF 하나를 선택적으로 붙일 수 있지만, 선행·후행·내부 space/tab, bare CR, 빈 값, 추가 줄은 허용하지 않는다.

나머지 ordinal은 `fresh`나 `restore`에서도 ready peer를 기다린 뒤 learner로 가입하고 voter로 승격된다. schema migration과 seed는 voter 경로에서만 실행하며 learner는 voter 복구와 migration이 완료된 후 가입시킨다. 전체 복구의 partition rollout과 checkpoint는 [backup-restore.md](./backup-restore.md)를 따른다.

## 현재 의존성 메모

워크스페이스 루트 `Cargo.toml`은 `hiqlite`를 아래 포크 커밋으로 패치한다.

- `mrchypark/hiqlite`
- `3e2112cd140c24730a0d1493054ad007cdabc48e`

이 값이 바뀌면 런타임 관련 동작 문서도 다시 검토하는 편이 안전하다.
