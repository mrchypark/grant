# Korean Certificate Source Expansion Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Expand Korean certificate discovery and registration so the desktop agent can handle NPKI/GPKI bundle sources, removable media discovery, and PKCS#12 end-to-end flows.

**Architecture:** Replace the current single-purpose NPKI service with a source-adapter based certstore service that preserves the existing HTTP API shape while expanding discovery and signing backends. Keep browser and gateway flows stable, add richer certificate metadata, and relax KM bundle requirements where PKCS#12 cannot provide separate KM material.

**Tech Stack:** Go desktop agent, Rust API/infra, TypeScript web SDK/example, Go/Rust/TS tests

---

### Task 1: Document and scaffold the generalized certstore model

**Files:**
- Modify: `desktop-agent/internal/npki/service.go`
- Modify: `docs/rps-certificate-gateway.md`
- Test: `desktop-agent/internal/npki/service_test.go`

**Step 1: Write the failing test**

- Add tests that expect source-aware certificate ids and metadata fields for bundle sources.
- Add tests that expect `GPKI` roots to be discovered the same way `NPKI` roots are discovered.

**Step 2: Run test to verify it fails**

Run: `go test ./desktop-agent/internal/npki -run 'Test(List|Discover)'`
Expected: FAIL because current metadata and discovery only understand NPKI bundle directories.

**Step 3: Write minimal implementation**

- Introduce shared metadata/source types in `service.go`.
- Refactor discovery so 4-file bundle detection is storage-agnostic and source-aware.
- Preserve existing sign behavior for bundle sources.

**Step 4: Run test to verify it passes**

Run: `go test ./desktop-agent/internal/npki -run 'Test(List|Discover)'`
Expected: PASS

**Step 5: Review gate**

- Implementer subagent self-review
- Spec reviewer subagent for Task 1 scope only
- Code quality reviewer subagent for Task 1 diff only

### Task 2: Expand removable media and root discovery

**Files:**
- Modify: `desktop-agent/internal/npki/service.go`
- Test: `desktop-agent/internal/npki/service_test.go`

**Step 1: Write the failing test**

- Add tests for mounted removable roots that contain `GPKI` or top-level `.p12/.pfx` files.
- Add tests that inaccessible roots are skipped without aborting the whole list operation.

**Step 2: Run test to verify it fails**

Run: `go test ./desktop-agent/internal/npki -run 'TestDiscover(DefaultRoots|Mounted|PartialFailure)'`
Expected: FAIL because current root discovery only enumerates `NPKI`.

**Step 3: Write minimal implementation**

- Generalize mounted root scanning to include `NPKI`, `GPKI`, and `.p12/.pfx`.
- Keep best-effort behavior for unreadable paths.

**Step 4: Run test to verify it passes**

Run: `go test ./desktop-agent/internal/npki -run 'TestDiscover(DefaultRoots|Mounted|PartialFailure)'`
Expected: PASS

**Step 5: Review gate**

- Implementer subagent self-review
- Spec reviewer subagent for Task 2 scope only
- Code quality reviewer subagent for Task 2 diff only

### Task 3: Add PKCS#12 discovery, metadata extraction, and signing

**Files:**
- Modify: `desktop-agent/internal/npki/service.go`
- Test: `desktop-agent/internal/npki/service_test.go`
- Test: `desktop-agent/internal/httpapi/server_test.go`

**Step 1: Write the failing test**

- Add unit tests for discovering `.p12/.pfx`, parsing metadata, validating passwords, and signing a challenge.
- Add HTTP tests that `/certificates` returns PKCS#12 metadata and `/sign` works with a PKCS#12-backed `certificateId`.

**Step 2: Run test to verify it fails**

Run: `go test ./desktop-agent/internal/npki ./desktop-agent/internal/httpapi -run 'Test(PKCS12|ServerListsCertificates|ServerReturnsBundleOnSign)'`
Expected: FAIL because PKCS#12 source handling does not exist.

**Step 3: Write minimal implementation**

- Add PKCS#12 adapter logic for metadata extraction and challenge signing.
- Convert PKCS#12 results into the existing proof bundle shape.
- If KM material cannot be extracted, carry empty values and mark metadata accordingly.

**Step 4: Run test to verify it passes**

Run: `go test ./desktop-agent/internal/npki ./desktop-agent/internal/httpapi -run 'Test(PKCS12|ServerListsCertificates|ServerReturnsBundleOnSign)'`
Expected: PASS

**Step 5: Review gate**

- Implementer subagent self-review
- Spec reviewer subagent for Task 3 scope only
- Code quality reviewer subagent for Task 3 diff only

### Task 4: Relax gateway contracts for optional KM material

**Files:**
- Modify: `packages/certd-web-sdk/src/agent.ts`
- Modify: `crates/api/src/lib.rs`
- Modify: `crates/infra/src/crypto.rs`
- Modify: `crates/infra/src/service.rs`
- Test: `crates/api/tests/cert_gateway.rs`
- Test: `crates/infra/tests/authorization_integration.rs`

**Step 1: Write the failing test**

- Add Rust tests that register and execute proof bundles without KM fields.
- Add API contract tests that accept omitted or empty KM material for PKCS#12-style requests.

**Step 2: Run test to verify it fails**

Run: `cargo test -p infra authorization_integration -- --nocapture`
Run: `cargo test -p api cert_gateway -- --nocapture`
Expected: FAIL because KM fields are currently treated as always present.

**Step 3: Write minimal implementation**

- Make proof bundle KM fields optional across TS, API mapping, crypto decoding, and service validation.
- Preserve existing behavior for 4-file bundles.

**Step 4: Run test to verify it passes**

Run: `cargo test -p infra authorization_integration -- --nocapture`
Run: `cargo test -p api cert_gateway -- --nocapture`
Expected: PASS

**Step 5: Review gate**

- Implementer subagent self-review
- Spec reviewer subagent for Task 4 scope only
- Code quality reviewer subagent for Task 4 diff only

### Task 5: Update approval UX, browser example, and product docs

**Files:**
- Modify: `desktop-agent/internal/approval/prompter_darwin.go`
- Modify: `desktop-agent/internal/localization/localization.go`
- Modify: `packages/certd-web-sdk/examples/browser/main.ts`
- Modify: `docs/rps-certificate-gateway.md`
- Test: `desktop-agent/internal/approval/prompter_darwin_test.go`
- Test: `packages/certd-web-sdk/src/index.test.ts`

**Step 1: Write the failing test**

- Add tests that expect richer labels or metadata usage in approval/default selection.
- Add example/SDK tests that handle extended certificate list metadata.

**Step 2: Run test to verify it fails**

Run: `go test ./desktop-agent/internal/approval -run Test`
Run: `pnpm --filter certd-web-sdk test`
Expected: FAIL because approval and example output do not know about source metadata.

**Step 3: Write minimal implementation**

- Improve approval labels to distinguish local bundle vs PKCS#12 vs removable media.
- Render richer certificate metadata in the browser example while keeping SDK high-level calls stable.
- Update gateway docs for supported Korean source types and removable media behavior.

**Step 4: Run test to verify it passes**

Run: `go test ./desktop-agent/internal/approval -run Test`
Run: `pnpm --filter certd-web-sdk test`
Expected: PASS

**Step 5: Review gate**

- Implementer subagent self-review
- Spec reviewer subagent for Task 5 scope only
- Code quality reviewer subagent for Task 5 diff only

### Task 6: Final verification and integration review

**Files:**
- Verify only: workspace diffs above

**Step 1: Run focused verification**

Run: `go test ./desktop-agent/internal/npki ./desktop-agent/internal/httpapi ./desktop-agent/internal/approval`
Run: `cargo test -p infra authorization_integration`
Run: `cargo test -p api cert_gateway`
Run: `pnpm --filter certd-web-sdk test`

**Step 2: Run broader regression where feasible**

Run: `cargo test`
Expected: PASS or clear statement of any unrelated failures

**Step 3: Review gate**

- Final spec reviewer subagent on the whole diff
- Final code quality reviewer subagent on the whole diff

**Step 4: Summarize residual risk**

- Note any unimplemented source types
- Note any platform-specific limits for removable media or PKCS#12 parsing
