# Certificate Gateway Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Build a certificate gateway in `certd` that supports `ephemeral` one-shot crawl execution and `stored` encrypted certificate registration, plus a Go desktop agent and a browser JS SDK that orchestrate the flow.

**Architecture:** Keep the gateway inside the existing Rust `certd` server so it can reuse current certificate, crypto, and crawl services. Add a KMS-style abstraction with an environment-variable-backed implementation for encrypted certificate storage, then layer a loopback Go desktop agent and a thin browser SDK on top of the new server APIs.

**Tech Stack:** Rust (`axum`, existing `infra` and `api` crates), Go for the desktop agent, browser JS/TypeScript for the web SDK, existing NPKI and crypto helpers, environment-variable-backed key wrapping.

---

### Task 1: Add KMS-style config parsing and provider tests

**Files:**
- Modify: `crates/infra/src/config.rs`
- Create: `crates/infra/src/kms.rs`
- Modify: `crates/infra/src/lib.rs`
- Test: `crates/infra/tests/kms_env.rs`

**Step 1: Write the failing test**

Write `crates/infra/tests/kms_env.rs` covering:

- loading `CERTD_KMS_ACTIVE_KEY_ID`
- loading multiple entries from `CERTD_KMS_MASTER_KEYS`
- rejecting missing active key
- rejecting malformed or non-base64 key material

**Step 2: Run test to verify it fails**

Run: `cargo test -p infra kms_env -- --nocapture`
Expected: FAIL because `kms.rs` and related config fields do not exist yet.

**Step 3: Write minimal implementation**

- Add `KmsConfig` to `crates/infra/src/config.rs`
- Create `crates/infra/src/kms.rs` with:
  - `KmsProvider` trait
  - `EnvKmsProvider`
  - parsing/validation helpers
- Export the module from `crates/infra/src/lib.rs`

**Step 4: Run test to verify it passes**

Run: `cargo test -p infra kms_env -- --nocapture`
Expected: PASS

**Step 5: Commit**

```bash
git add crates/infra/src/config.rs crates/infra/src/kms.rs crates/infra/src/lib.rs crates/infra/tests/kms_env.rs
git commit -m "feat: add env-backed kms provider"
```

### Task 2: Extend certificate crypto for wrapped DEK storage

**Files:**
- Modify: `crates/infra/src/crypto.rs`
- Modify: `crates/infra/src/service.rs`
- Test: `crates/infra/tests/npki_signer.rs`

**Step 1: Write the failing test**

Add a new test proving:

- a random DEK can encrypt a certificate bundle
- a wrapped DEK can be unwrapped and used to recover the original bundle
- the API does not require or retain a new "storage password"

**Step 2: Run test to verify it fails**

Run: `cargo test -p infra npki_signer -- --nocapture`
Expected: FAIL because bundle encryption is still tied only to the existing master-key flow.

**Step 3: Write minimal implementation**

- Add helper functions in `crates/infra/src/crypto.rs` for:
  - random DEK generation
  - encrypting a bundle with a supplied DEK
  - decrypting a bundle with a supplied DEK
- Update `crates/infra/src/service.rs` to prepare for KMS-backed certificate persistence

**Step 4: Run test to verify it passes**

Run: `cargo test -p infra npki_signer -- --nocapture`
Expected: PASS

**Step 5: Commit**

```bash
git add crates/infra/src/crypto.rs crates/infra/src/service.rs crates/infra/tests/npki_signer.rs
git commit -m "feat: add per-certificate dek encryption"
```

### Task 3: Add gateway schema for challenges and crawl task tokens

**Files:**
- Modify: `crates/infra/src/database.rs`
- Test: `crates/infra/tests/database_schema.rs`
- Test: `crates/infra/tests/database_seed.rs`

**Step 1: Write the failing test**

Extend schema tests to assert:

- `cert_gateway_challenges` exists with expiration and consumption columns
- `crawl_task_tokens` exists with hashed token storage and `used_at`
- `certificates` includes wrapped DEK metadata columns

**Step 2: Run test to verify it fails**

Run: `cargo test -p infra database_schema -- --nocapture`
Expected: FAIL because the new tables and columns do not exist yet.

**Step 3: Write minimal implementation**

- Extend `ensure_schema_only()` and related helpers in `crates/infra/src/database.rs`
- Add read/write helpers for:
  - challenge creation
  - challenge consumption
  - stored certificate persistence with `wrapped_dek` and `kms_key_id`
  - crawl task token issuance and one-shot consumption

**Step 4: Run test to verify it passes**

Run: `cargo test -p infra database_schema -- --nocapture`
Expected: PASS

**Step 5: Commit**

```bash
git add crates/infra/src/database.rs crates/infra/tests/database_schema.rs crates/infra/tests/database_seed.rs
git commit -m "feat: add certificate gateway schema"
```

### Task 4: Implement Rust certificate gateway service logic

**Files:**
- Modify: `crates/infra/src/service.rs`
- Modify: `crates/domain/src/lib.rs`
- Test: `crates/infra/tests/authorization_integration.rs`

**Step 1: Write the failing test**

Add service-level tests for:

- challenge creation with TTL
- rejection of expired or reused challenge
- `ephemeral` proof leading directly to crawl start
- `stored` proof leading to encrypted certificate persistence
- one-shot crawl token issuance and single-use enforcement

**Step 2: Run test to verify it fails**

Run: `cargo test -p infra authorization_integration -- --nocapture`
Expected: FAIL because certificate gateway service methods do not exist yet.

**Step 3: Write minimal implementation**

- Add service methods for:
  - `create_cert_gateway_challenge`
  - `execute_ephemeral_crawl`
  - `register_certificate_from_proof`
  - `issue_crawl_task_token`
  - `consume_crawl_task_token_and_start_crawl`
- Reuse current NPKI helpers for signature and certificate inspection
- Wire the env-backed KMS provider into stored certificate save/load paths

**Step 4: Run test to verify it passes**

Run: `cargo test -p infra authorization_integration -- --nocapture`
Expected: PASS

**Step 5: Commit**

```bash
git add crates/infra/src/service.rs crates/domain/src/lib.rs crates/infra/tests/authorization_integration.rs
git commit -m "feat: add certificate gateway service flows"
```

### Task 5: Expose certificate gateway HTTP routes in the API crate

**Files:**
- Modify: `crates/api/src/lib.rs`
- Create: `crates/api/tests/cert_gateway.rs`
- Modify: `crates/api/tests/support/mod.rs`

**Step 1: Write the failing test**

Create HTTP tests covering:

- `POST /api/v1/organizations/{organizationId}/cert-gateway/challenges`
- `POST /api/v1/organizations/{organizationId}/cert-gateway/ephemeral-executions`
- `POST /api/v1/organizations/{organizationId}/certificates`
- `POST /api/v1/organizations/{organizationId}/certificates/{certificateId}/crawl-tokens`
- `POST /api/v1/organizations/{organizationId}/crawls` with a valid and reused task token

**Step 2: Run test to verify it fails**

Run: `cargo test -p api cert_gateway -- --nocapture`
Expected: FAIL because the routes and request/response contracts do not exist yet.

**Step 3: Write minimal implementation**

- Add request/response DTOs and handlers in `crates/api/src/lib.rs`
- Reuse existing request authentication and organization scoping
- Add explicit error mapping for:
  - expired challenge
  - invalid signature
  - invalid certificate state
  - reused crawl task token

**Step 4: Run test to verify it passes**

Run: `cargo test -p api cert_gateway -- --nocapture`
Expected: PASS

**Step 5: Commit**

```bash
git add crates/api/src/lib.rs crates/api/tests/cert_gateway.rs crates/api/tests/support/mod.rs
git commit -m "feat: add certificate gateway api routes"
```

### Task 6: Create the Go desktop agent skeleton

**Files:**
- Create: `desktop-agent/go.mod`
- Create: `desktop-agent/cmd/desktop-agent/main.go`
- Create: `desktop-agent/internal/httpapi/server.go`
- Create: `desktop-agent/internal/npki/service.go`
- Create: `desktop-agent/internal/npki/service_test.go`

**Step 1: Write the failing test**

Add Go tests covering:

- `GET /healthz`
- `GET /certificates` returning normalized metadata
- `POST /sign` rejecting missing password and missing challenge

**Step 2: Run test to verify it fails**

Run: `cd desktop-agent && go test ./...`
Expected: FAIL because the desktop agent module does not exist yet.

**Step 3: Write minimal implementation**

- Initialize the Go module
- Add a minimal HTTP server with loopback bind only
- Stub certificate enumeration and sign interfaces behind `internal/npki`
- Return placeholder errors for unimplemented OS-specific certificate access while keeping the HTTP contract stable

**Step 4: Run test to verify it passes**

Run: `cd desktop-agent && go test ./...`
Expected: PASS

**Step 5: Commit**

```bash
git add desktop-agent/go.mod desktop-agent/cmd/desktop-agent/main.go desktop-agent/internal/httpapi/server.go desktop-agent/internal/npki/service.go desktop-agent/internal/npki/service_test.go
git commit -m "feat: scaffold desktop certificate agent"
```

### Task 7: Create the browser JS SDK skeleton

**Files:**
- Create: `packages/web-sdk/package.json`
- Create: `packages/web-sdk/src/index.ts`
- Create: `packages/web-sdk/src/client.ts`
- Create: `packages/web-sdk/src/agent.ts`
- Create: `packages/web-sdk/src/index.test.ts`

**Step 1: Write the failing test**

Add SDK tests for:

- `createChallenge()`
- `runEphemeralExecution()`
- `registerStoredCertificate()`
- rejection when agent health check fails

**Step 2: Run test to verify it fails**

Run: `cd packages/web-sdk && npm test`
Expected: FAIL because the SDK package does not exist yet.

**Step 3: Write minimal implementation**

- Add a small SDK surface that:
  - calls the Rust gateway APIs
  - forwards password and challenge to the local desktop agent
  - distinguishes `stored` and `ephemeral` flows
- Keep signing inside the agent client boundary; the SDK should only transport inputs and outputs

**Step 4: Run test to verify it passes**

Run: `cd packages/web-sdk && npm test`
Expected: PASS

**Step 5: Commit**

```bash
git add packages/web-sdk/package.json packages/web-sdk/src/index.ts packages/web-sdk/src/client.ts packages/web-sdk/src/agent.ts packages/web-sdk/src/index.test.ts
git commit -m "feat: scaffold certificate gateway web sdk"
```

### Task 8: Document and verify the end-to-end flow

**Files:**
- Modify: `README.md`
- Create: `docs/rps-certificate-gateway.md`
- Test: `crates/api/tests/cert_gateway.rs`
- Test: `crates/infra/tests/authorization_integration.rs`

**Step 1: Write the failing test**

Add or extend integration tests to document the final expected E2E:

- create challenge
- sign via agent contract
- execute `ephemeral`
- register `stored`
- issue a one-shot crawl token
- start a crawl with the token and reject reuse

**Step 2: Run test to verify it fails**

Run: `cargo test -p api cert_gateway -- --nocapture && cargo test -p infra authorization_integration -- --nocapture`
Expected: FAIL until the documented E2E contract and final wiring are complete.

**Step 3: Write minimal implementation**

- Update `README.md` with the certificate gateway summary
- Add `docs/rps-certificate-gateway.md` with:
  - architecture
  - environment variables
  - desktop agent local API
  - JS SDK usage
  - `stored` and `ephemeral` examples
- Fill any missing route or service wiring found during E2E verification

**Step 4: Run test to verify it passes**

Run: `cargo test -p api cert_gateway -- --nocapture && cargo test -p infra authorization_integration -- --nocapture`
Expected: PASS

**Step 5: Commit**

```bash
git add README.md docs/rps-certificate-gateway.md crates/api/tests/cert_gateway.rs crates/infra/tests/authorization_integration.rs
git commit -m "docs: add certificate gateway e2e guide"
```

## Suggested execution order

1. Complete Tasks 1-5 first so the Rust server contract is stable.
2. Build Task 6 after the server routes are fixed enough to target.
3. Build Task 7 only after the desktop agent HTTP contract is stable.
4. Finish with Task 8 so docs reflect the final contracts, not an intermediate draft.

## Verification checklist

- `cargo test -p infra kms_env -- --nocapture`
- `cargo test -p infra database_schema -- --nocapture`
- `cargo test -p infra authorization_integration -- --nocapture`
- `cargo test -p api cert_gateway -- --nocapture`
- `cd desktop-agent && go test ./...`
- `cd packages/web-sdk && npm test`

## Risks to watch during implementation

- NPKI 서명 검증 경계가 현재 collector용 signer와 완전히 같지 않을 수 있다.
- `ephemeral` 경로가 편의상 저장형과 동일한 persistence path를 타지 않도록 주의해야 한다.
- loopback desktop agent는 CORS, 포트 충돌, 단일 인스턴스 정책을 명확히 해야 한다.
- 환경변수 기반 KMS 구현은 개발/온프레 전용으로 두고, 인터페이스를 cloud KMS에 종속시키지 말아야 한다.
