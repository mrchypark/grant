# certd 런타임 모드

`certd`는 `hiqlite` 접근 방식을 `embedded`, `remote`, `split` 세 가지로 나눈다.

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

`crates/app/src/main.rs`는 `CERTD_LOCAL_NODE_ROLE`에 따라 동작을 나눈다.

- `voter`
  - 기본 public API 노드다.
  - schema migration, initialize, write/admin 처리 가능
- `learner`
  - read replica 성격의 노드다.
  - split/embedded learner에서는 initialize를 건너뛴다.
  - `crates/api/src/lib.rs`에서 write/admin 라우트는 `requires_voter_node`로 막는다.

## 관련 환경변수

```bash
export CERTD_RUNTIME_MODE=embedded   # embedded | remote | split
export CERTD_LOCAL_NODE_ROLE=voter   # voter | learner
export CERTD_READ_CONSISTENCY=local  # local | strong | prefer-strong

export CERTD_REMOTE_NODES=127.0.0.1:18100,127.0.0.1:18110
export CERTD_REMOTE_TLS=false
export CERTD_REMOTE_TLS_NO_VERIFY=false
export CERTD_REMOTE_API_SECRET=AnotherSecureSecret42
export CERTD_REMOTE_WITH_PROXY=false
```

`CERTD_REMOTE_NODES`는 `host:port` 형식을 권장한다. 기존 `http://...`, `https://...` 값이 들어와도 앱에서 scheme를 제거하고 `hiqlite`에 전달한다.

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

## healthz 의미

`GET /healthz`는 아래 상태를 같이 노출한다.

- `readReady`
  - 로컬 또는 대체 read 경로가 사용 가능한지
- `writeReady`
  - quorum write와 strong read 경로가 사용 가능한지
- `runtimeMode`
  - 현재 런타임 모드
- `defaultReadConsistency`
  - 기본 읽기 정책

## 복구 보조 동작

`HQL_BACKUP_RESTORE`가 설정된 상태에서 `embedded` 또는 `split` 로컬 노드를 기동하면, 앱은 `HQL_DATA_DIR/state_machine/lock`의 stale lock 정리를 시도한다.

이 동작은 restore 직후 재기동 시 local lock 파일 때문에 노드 시작이 막히는 경우를 줄이기 위한 것이다.

## 현재 의존성 메모

워크스페이스 루트 `Cargo.toml`은 `hiqlite`를 아래 포크 커밋으로 패치한다.

- `mrchypark/hiqlite`
- `3e2112cd140c24730a0d1493054ad007cdabc48e`

이 값이 바뀌면 런타임 관련 동작 문서도 다시 검토하는 편이 안전하다.
