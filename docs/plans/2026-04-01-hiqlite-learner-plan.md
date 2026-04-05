# Hiqlite Learner-Only Runtime Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** `certd`에 `local read + remote write/strong read` 런타임을 추가해 learner-only sidecar 구조를 수용할 수 있게 만든다.

**Architecture:** `app` 계층에서 `hiqlite` 로컬/원격 클라이언트를 조합하는 런타임을 만들고, `infra::Database`가 읽기 일관성 정책에 따라 `query_map()` 또는 `query_consistent_map()`를 선택하게 바꾼다. API는 확장된 health 상태를 노출하고, 서비스는 기본 조회 정책을 명시적으로 사용한다.

**Tech Stack:** Rust, axum, hiqlite, serde, tokio

---

### Task 1: 런타임/일관성 설정 추가

**Files:**
- Modify: `/Users/cypark/Documents/work/cert/crates/infra/src/config.rs`
- Test: `/Users/cypark/Documents/work/cert/crates/infra/tests/config_env.rs`

**Step 1: Write the failing test**

- `AppConfig::from_env()`가 기본 `runtime_mode=embedded`, `default_read_consistency=local`을 반환하는 테스트를 추가한다.
- `CERTD_RUNTIME_MODE=split`, `CERTD_READ_CONSISTENCY=prefer-strong`를 해석하는 테스트를 추가한다.

**Step 2: Run test to verify it fails**

Run: `cargo test -p infra config_env -- --nocapture`

**Step 3: Write minimal implementation**

- `RuntimeMode`
- `ReadConsistency`
- `HiqliteConfig`
- 환경변수 파서 추가

**Step 4: Run test to verify it passes**

Run: `cargo test -p infra config_env -- --nocapture`

### Task 2: Health 상태 모델 확장

**Files:**
- Modify: `/Users/cypark/Documents/work/cert/crates/domain/src/lib.rs`
- Test: `/Users/cypark/Documents/work/cert/crates/domain/tests/model_smoke.rs`

**Step 1: Write the failing test**

- `HealthStatus`가 `read_ready`, `write_ready`, `runtime_mode`, `default_read_consistency`를 포함하는 테스트를 추가한다.

**Step 2: Run test to verify it fails**

Run: `cargo test -p domain model_smoke -- --nocapture`

**Step 3: Write minimal implementation**

- `HealthStatus` 구조체 확장
- 기본 생성 헬퍼 추가

**Step 4: Run test to verify it passes**

Run: `cargo test -p domain model_smoke -- --nocapture`

### Task 3: Database 읽기 전략 분리

**Files:**
- Modify: `/Users/cypark/Documents/work/cert/crates/infra/src/database.rs`
- Test: `/Users/cypark/Documents/work/cert/crates/infra/tests/database_runtime.rs`

**Step 1: Write the failing test**

- 로컬 읽기 전략은 `query_map()`을, strong 읽기 전략은 `query_consistent_map()`을 선택하는 테스트를 추가한다.
- `prefer-strong`은 원격 읽기 불가 시 로컬로 폴백하는 테스트를 추가한다.

**Step 2: Run test to verify it fails**

Run: `cargo test -p infra database_runtime -- --nocapture`

**Step 3: Write minimal implementation**

- `DatabaseClient` 추상화 또는 동등한 테스트 가능한 경계 추가
- `read_rows_with()` / `read_row_with()` 추가
- 기존 조회 메서드들을 명시적 읽기 정책으로 전환

**Step 4: Run test to verify it passes**

Run: `cargo test -p infra database_runtime -- --nocapture`

### Task 4: app 런타임 조립 추가

**Files:**
- Modify: `/Users/cypark/Documents/work/cert/crates/app/src/main.rs`
- Modify: `/Users/cypark/Documents/work/cert/crates/infra/src/lib.rs`
- Modify: `/Users/cypark/Documents/work/cert/crates/infra/src/database.rs`

**Step 1: Write the failing test**

- 런타임 조립 로직이 `embedded`, `remote`, `split` 모드별로 필요한 클라이언트 구성을 선택하는 테스트를 추가한다.

**Step 2: Run test to verify it fails**

Run: `cargo test -p infra runtime -- --nocapture`

**Step 3: Write minimal implementation**

- 로컬/원격 client 조립기 추가
- `Database::new(...)`가 단일 client 대신 runtime wrapper를 받도록 변경

**Step 4: Run test to verify it passes**

Run: `cargo test -p infra runtime -- --nocapture`

### Task 5: API health 응답과 서비스 연결

**Files:**
- Modify: `/Users/cypark/Documents/work/cert/crates/api/src/lib.rs`
- Modify: `/Users/cypark/Documents/work/cert/crates/infra/src/service.rs`

**Step 1: Write the failing test**

- `/healthz`가 확장된 상태를 반환하는 테스트를 추가한다.
- 서비스 조회 메서드가 기본적으로 `local` 읽기 정책을 사용한다는 테스트를 추가한다.

**Step 2: Run test to verify it fails**

Run: `cargo test -p api -- --nocapture`

**Step 3: Write minimal implementation**

- health handler 확장
- 서비스에서 읽기 전략 호출 연결

**Step 4: Run test to verify it passes**

Run: `cargo test --workspace -- --nocapture`

### Task 6: README와 운영 문서 정리

**Files:**
- Modify: `/Users/cypark/Documents/work/cert/README.md`
- Modify: `/Users/cypark/Documents/work/cert/docs/backup-restore.md`

**Step 1: Update docs**

- `embedded`, `remote`, `split` 런타임 모드 설명 추가
- `read_ready` / `write_ready` 의미 정리
- learner-only는 향후 `hiqlite` patch 전제임을 명시

**Step 2: Verify docs**

Run: `cargo test --workspace -- --nocapture`

