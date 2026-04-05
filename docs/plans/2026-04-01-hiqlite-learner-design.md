# Hiqlite Learner-Only Runtime Design

**Goal:** `certd`가 `hiqlite`를 quorum voter와 local read replica로 분리해, 로컬 읽기와 원격 쓰기를 동시에 지원하도록 런타임을 재구성한다.

## 배경

현재 `certd`는 `hiqlite::start_node()`가 반환한 단일 `Client`를 그대로 사용한다.

- 모든 쓰기는 quorum 경로로 간다.
- 모든 읽기는 `query_consistent_map()`으로 leader 기준 강일관성 읽기를 사용한다.
- 이 구조는 안전하지만, learner-only sidecar나 local read 경로를 붙일 수 없다.

우리가 원하는 구조는 다음과 같다.

- `StatefulSet` 기반의 안정적인 `voter` 노드
- `learner-only` 로컬 복제본
- `GET`은 기본적으로 로컬 읽기
- `POST/PUT/PATCH/DELETE`는 quorum으로 원격 쓰기
- 필요한 경우에만 strong read

## 목표 상태

### 1. 런타임 역할 분리

런타임 역할은 명시적으로 분리한다.

- `embedded`
  - 현재와 같은 단일 임베디드 노드
  - 로컬 상태 보유
  - 개발/단일 노드 운영용
- `remote`
  - 로컬 상태 없음
  - 모든 요청을 quorum cluster로 전달
- `split`
  - 로컬 임베디드 클라이언트 + 원격 quorum 클라이언트 동시 보유
  - 로컬 read, 원격 write/strong read 담당

`split`은 learner-only sidecar와 가장 잘 맞는다.

### 2. 읽기 일관성 모델

클라이언트가 원하는 읽기 의미를 명시한다.

- `local`
  - 로컬 SQLite에서 읽음
- `strong`
  - quorum으로 강일관성 읽기
  - `write_ready=false`면 실패
- `prefer_strong`
  - 가능하면 quorum 읽기
  - 불가능하면 로컬 폴백

이 구분이 중요하다. `write_ready=false`에서 로컬 폴백을 허용하면 그 요청은 더 이상 strong read가 아니다.

### 3. readiness 분리

readiness는 두 축으로 나눈다.

- `read_ready`
  - 로컬 learner가 읽기 가능한 상태
- `write_ready`
  - quorum cluster에 쓰기/strong read를 보낼 수 있는 상태

`split` 모드에서는 두 값이 다를 수 있다.

예:

- `read_ready=true`, `write_ready=false`
  - 일반 조회는 계속 가능
  - 쓰기와 strong read는 실패

## 코드 설계

### 1. 런타임 클라이언트 래퍼

`infra`에 `CertdRuntime` 또는 동등한 래퍼를 추가한다.

이 래퍼는 내부적으로 다음을 가진다.

- `local: Option<Client>`
- `remote: Option<Client>`
- `role/runtime_mode`
- `default_read_consistency`

호출 규칙:

- 쓰기: 항상 `remote` 우선, 없으면 `local`
- 일반 읽기: `local` 우선
- strong read: `remote.query_consistent_map()`
- fallback: 정책에 따라 `local.query_map()`

### 2. Database 계층의 의미 분리

현재 `Database`는 `Client` 하나만 들고 있고, 읽기/쓰기 전략을 구분하지 않는다.

이를 아래처럼 바꾼다.

- `read_rows(...)`
- `read_row(...)`
- `read_rows_with(consistency, ...)`
- `execute(...)`

핵심은 각 메서드가 어떤 일관성 수준을 사용하는지 명확해지는 것이다.

### 3. API 계층의 의도 전달

API는 읽기 의도를 명시할 수 있어야 한다.

기본값:

- `GET`: `local`
- 관리성/중요 조회: `prefer_strong`

확장:

- 헤더 `x-consistency: local|strong|prefer-strong`
- 또는 내부 서비스 메서드 수준에서 라우팅

1차 구현에서는 헤더를 추가하지 않고 내부 기본 정책만 도입해도 충분하다.

### 4. 상태 노출

`/healthz`는 단순 `ok` 대신 런타임 상태를 담아야 한다.

- `status`
- `readReady`
- `writeReady`
- `runtimeMode`
- `defaultReadConsistency`

이렇게 해야 learner-only/remote/split 모드에서 실제로 어떤 동작을 하는지 외부에서 판단할 수 있다.

## 운영 모델

### voter

- `StatefulSet`
- stable ordinal 기반 `node_id`
- 기본 `3`개 고정
- scale in/out은 membership 절차와 함께 수행

### learner-only

- local read replica
- promote 금지
- quorum 불참
- 자유 증감 가능

`hiqlite` upstream에 learner-only 모드가 없으므로, 이 부분은 장기적으로 vendor patch 또는 fork가 필요하다.

다만 `certd`는 먼저 런타임/읽기 전략 분리를 구현해, patch가 들어왔을 때 바로 붙을 수 있는 구조로 바꾼다.

## 단계적 구현 전략

### 1단계

- `certd` 내부 런타임 모드 추가
- `Database` 읽기/쓰기 전략 분리
- `HealthStatus` 확장
- strong/local/prefer-strong 정책 도입

### 2단계

- `remote` 모드 실제 운영 가능하게 구성
- `split` 모드용 환경변수와 원격 접속 설정 추가

### 3단계

- `hiqlite` learner-only patch
- `read_ready`를 learner 상태와 연결
- 필요 시 operator 도입

## 비목표

- 이번 단계에서 `hiqlite` 자체 fork를 바로 구현하지 않는다.
- 이번 단계에서 Kubernetes operator를 바로 추가하지 않는다.
- 이번 단계에서 모든 API에 외부 공개 consistency 파라미터를 강제하지 않는다.

## 성공 기준

- `certd`가 런타임 모드를 환경변수로 구분해 기동할 수 있다.
- `Database`가 로컬 읽기와 강일관성 읽기를 명시적으로 구분한다.
- `healthz` 응답만 봐도 read/write 가능 상태를 판단할 수 있다.
- 향후 learner-only patch가 들어와도 `certd` 앱 구조를 다시 뜯지 않아도 된다.
