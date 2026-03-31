# Rust Native Collector Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Rust 서비스가 공동인증서 로그인과 RPS 수집을 직접 수행한다.

**Architecture:** 기존 Rust API/DB 계층은 유지하고, `infra`에 Rust signer와 Rust collector를 추가해 `AppService::sync`가 직접 호출하도록 바꾼다. 수집 결과는 기존 `hiqlite` 테이블과 PDF 저장소에 그대로 적재한다.

**Tech Stack:** Rust, axum, hiqlite, reqwest, openssl, x509-parser, filesystem/S3 artifact store

---

### Task 1: Rust signer 골격 추가

**Files:**
- Create: `/Users/cypark/Documents/work/cert/crates/infra/src/npki.rs`
- Modify: `/Users/cypark/Documents/work/cert/crates/infra/src/lib.rs`
- Test: `/Users/cypark/Documents/work/cert/crates/infra/tests/npki_signer.rs`

### Task 2: NPKI 개인키 복호화 이식

**Files:**
- Modify: `/Users/cypark/Documents/work/cert/crates/infra/src/npki.rs`
- Test: `/Users/cypark/Documents/work/cert/crates/infra/tests/npki_signer.rs`

### Task 3: PKCS#7 signed/enveloped payload 생성

**Files:**
- Modify: `/Users/cypark/Documents/work/cert/crates/infra/src/npki.rs`
- Test: `/Users/cypark/Documents/work/cert/crates/infra/tests/npki_signer.rs`

### Task 4: Rust RPS collector 추가

**Files:**
- Create: `/Users/cypark/Documents/work/cert/crates/infra/src/rps.rs`
- Test: `/Users/cypark/Documents/work/cert/crates/infra/tests/rps_parsing.rs`

### Task 5: 로그인/설비/발전량/REC/documents 포팅

**Files:**
- Modify: `/Users/cypark/Documents/work/cert/crates/infra/src/rps.rs`
- Test: `/Users/cypark/Documents/work/cert/crates/infra/tests/rps_parsing.rs`

### Task 6: 이전 collector 제거 및 service 연결

**Files:**
- Modify: `/Users/cypark/Documents/work/cert/crates/infra/src/service.rs`
- Modify: `/Users/cypark/Documents/work/cert/crates/infra/src/lib.rs`

### Task 7: Dockerfile / README / 빌드 정리

**Files:**
- Modify: `/Users/cypark/Documents/work/cert/Dockerfile`
- Modify: `/Users/cypark/Documents/work/cert/README.md`

### Task 8: E2E 검증

**Run:**
- `cargo test --workspace`
- 로컬 full sync
- 로컬 delta sync
- `GET /documents/{id}/download`
- backup local / s3 확인
