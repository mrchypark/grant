# TrailBase Premium Login Implementation Plan

> **For agentic workers:** REQUIRED: Use superpowers:subagent-driven-development (if subagents available) or superpowers:executing-plans to implement this plan. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Ship a premium branded `/login` experience in front of TrailBase with email/password, Google, Apple, and gated Kakao entrypoints while preserving TrailBase as the auth backend.

**Architecture:** Keep TrailBase as the token issuer and OAuth callback handler. Serve a new same-origin static login frontend from `trailbase-auth-bridge`, add bridge-owned fallback routing for `/` and `/login`, and use TrailBase auth APIs directly for password login and OAuth initiation. Gate Kakao behind explicit configuration and verification, and preserve rollback to built-in TrailBase auth UI.

**Tech Stack:** Rust 2024, Axum, TrailBase, distroless Docker image, Kubernetes manifests, static HTML/CSS/JS assets, shell-based verification

---

## File Structure

### Existing files to modify

- `crates/trailbase-auth-bridge/src/lib.rs`
  Responsibility: bridge config, TrailBase server initialization, route composition, auth-ui component bootstrap.
- `crates/trailbase-auth-bridge/Cargo.toml`
  Responsibility: bridge crate dependencies if new static-file or security helpers are needed.
- `Dockerfile.trailbase-auth-bridge`
  Responsibility: include login frontend assets in the runtime image.
- `deploy/k8s/trailbase-configmap.yaml`
  Responsibility: runtime env defaults for custom login enablement, public asset directory, fallback behavior, provider gating.
- `deploy/k8s/trailbase-deployment.yaml`
  Responsibility: mount or expose any required runtime asset/config locations if needed.

### New files to create

- `crates/trailbase-auth-bridge/static/login/index.html`
  Responsibility: branded login shell and semantic structure.
- `crates/trailbase-auth-bridge/static/login/login.css`
  Responsibility: premium visual styling, responsive layout, focus states, reduced-motion behavior.
- `crates/trailbase-auth-bridge/static/login/login.js`
  Responsibility: same-origin JSON password login, inline errors, provider button initialization, return-path handling.
- `crates/trailbase-auth-bridge/tests/login_routes.rs`
  Responsibility: bridge-level route tests for `/`, `/login`, fallback mode, and reserved-path ownership.
- `crates/trailbase-auth-bridge/tests/login_security.rs`
  Responsibility: tests for return-path normalization, Kakao gating, and login contract enforcement.
- `docs/superpowers/plans/2026-04-10-trailbase-premium-login-plan.md`
  Responsibility: this implementation plan.

### Deliberate implementation choice

- Do **not** introduce a new frontend build tool in the first pass.
- Keep the login frontend as hand-authored static HTML/CSS/JS under the bridge crate.
- Use a versioned asset prefix such as `/login-assets/` from day one to simplify caching and rollback behavior.

## Chunk 1: Bridge Routing And Runtime Contract

### Task 1: Add bridge config for custom login and fallback behavior

**Files:**
- Modify: `crates/trailbase-auth-bridge/src/lib.rs`
- Test: `crates/trailbase-auth-bridge/tests/login_routes.rs`

- [ ] **Step 1: Write the failing config parsing tests**

Write tests that assert new env-backed config fields exist for:

- custom login enabled flag
- fallback-to-built-in-auth flag
- public assets directory
- Kakao enabled flag

Expected test shape:

```rust
#[test]
fn bridge_config_reads_login_flags_from_env() {
    std::env::set_var("TRAILBASE_CUSTOM_LOGIN_ENABLED", "1");
    std::env::set_var("TRAILBASE_LOGIN_FALLBACK_ENABLED", "0");
    std::env::set_var("TRAILBASE_PUBLIC_DIR", "/app/public");
    let cfg = BridgeConfig::from_env().expect("config");
    assert!(cfg.custom_login_enabled);
    assert!(!cfg.login_fallback_enabled);
    assert_eq!(cfg.public_dir.as_deref(), Some(std::path::Path::new("/app/public")));
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test --manifest-path crates/trailbase-auth-bridge/Cargo.toml bridge_config_reads_login_flags_from_env -- --nocapture`

Expected: FAIL because `BridgeConfig` does not yet expose the new fields.

- [ ] **Step 3: Implement minimal config additions**

Add fields to `BridgeConfig`:

- `public_dir: Option<PathBuf>`
- `custom_login_enabled: bool`
- `login_fallback_enabled: bool`
- `kakao_enabled: bool`
- `default_post_login_path: String`

Read them from environment with conservative defaults:

- custom login off by default in code until explicitly enabled by deployment
- fallback on by default whenever custom login is not yet fully rolled out
- Kakao off by default
- defaults apply only when the env var is absent; malformed rollout booleans must fail startup so the bridge never silently serves the wrong auth surface

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test --manifest-path crates/trailbase-auth-bridge/Cargo.toml bridge_config_reads_login_flags_from_env -- --nocapture`

Expected: PASS

- [ ] **Step 5: Commit**

```bash
git add crates/trailbase-auth-bridge/src/lib.rs crates/trailbase-auth-bridge/tests/login_routes.rs
git commit -m "feat: add bridge login routing config"
```

### Task 2: Make TrailBase serve static login assets without shadowing reserved routes

**Files:**
- Modify: `crates/trailbase-auth-bridge/src/lib.rs`
- Test: `crates/trailbase-auth-bridge/tests/login_routes.rs`

- [ ] **Step 1: Write the failing reserved-route test**

Add a route test that asserts:

- `/api/auth/v1/login` stays owned by TrailBase or bridge auth stack
- `/_/auth/login` stays reachable in fallback mode
- static assets only come from `/login-assets/...`

Example sketch:

```rust
#[tokio::test]
async fn static_assets_do_not_shadow_reserved_auth_routes() {
    let app = build_test_router_with_login_assets().await;
    let login_api = request(&app, "POST", "/api/auth/v1/login").await;
    assert_ne!(login_api.status(), StatusCode::NOT_FOUND);
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test --manifest-path crates/trailbase-auth-bridge/Cargo.toml static_assets_do_not_shadow_reserved_auth_routes -- --nocapture`

Expected: FAIL because route ownership is not yet exercised.

- [ ] **Step 3: Pass `public_dir` into TrailBase and lock route ownership**

Update `ServerOptions` to use `public_dir` when configured. Add bridge-owned routes for:

- `/`
- `/login`
- optional redirect helpers

Do **not** serve custom HTML from `/api/**` or `/_/auth/**`.

- [ ] **Step 4: Run the route test**

Run: `cargo test --manifest-path crates/trailbase-auth-bridge/Cargo.toml static_assets_do_not_shadow_reserved_auth_routes -- --nocapture`

Expected: PASS

- [ ] **Step 5: Commit**

```bash
git add crates/trailbase-auth-bridge/src/lib.rs crates/trailbase-auth-bridge/tests/login_routes.rs
git commit -m "feat: wire public login assets into trailbase bridge"
```

### Task 3: Implement canonical `/` and `/login` fallback routing

Task 3 and Task 4 share one release boundary: the routing work is not considered complete until `/login` terminates in the minimal custom shell instead of a dead-end or temporary placeholder.

**Files:**
- Modify: `crates/trailbase-auth-bridge/src/lib.rs`
- Test: `crates/trailbase-auth-bridge/tests/login_routes.rs`

- [ ] **Step 1: Write failing redirect tests**

Cover:

- unauthenticated `/` redirects to `/login` when custom login is enabled
- `/` and `/login` redirect to `/_/auth/login` when fallback mode is enabled
- authenticated users bypass `/login` to default destination

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test --manifest-path crates/trailbase-auth-bridge/Cargo.toml login_routes -- --nocapture`

Expected: FAIL on missing redirect behavior.

- [ ] **Step 3: Implement minimal routing logic**

Add bridge route handlers that:

- normalize unauthenticated entry
- honor fallback flag
- do server-side redirects only
- avoid loops between `/login` and `/_/auth/login`

- [ ] **Step 4: Re-run the route tests**

Run: `cargo test --manifest-path crates/trailbase-auth-bridge/Cargo.toml login_routes -- --nocapture`

Expected: PASS

- [ ] **Step 5: Commit**

```bash
git add crates/trailbase-auth-bridge/src/lib.rs crates/trailbase-auth-bridge/tests/login_routes.rs
git commit -m "feat: add login fallback routing"
```

## Chunk 2: Premium Login Frontend

### Task 4: Create the premium static login shell

This task closes the open edge from Task 3 by making `/login` a real reachable entrypoint in custom-login mode.

**Files:**
- Create: `crates/trailbase-auth-bridge/static/login/index.html`
- Create: `crates/trailbase-auth-bridge/static/login/login.css`

- [ ] **Step 1: Write a minimal screenshot-less structural test or fixture assertion**

If a Rust test is practical, assert the HTML file contains:

- logo slot
- email input
- password input
- primary button
- Google, Kakao, Apple buttons

If file-content tests are too brittle, create a smoke test that requests `/login` and asserts those labels exist in the response body.

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test --manifest-path crates/trailbase-auth-bridge/Cargo.toml login_page_contains_expected_controls -- --nocapture`

Expected: FAIL because assets do not exist yet.

- [ ] **Step 3: Implement the static shell**

Create a premium branded page with:

- empty logo slot block
- heading and supporting copy
- left/right desktop composition
- single-column mobile fallback
- semantic labels and accessible controls
- asset references rooted under `/login-assets/`
- hashed or versioned asset filenames prepared for runtime serving, for example `login.[build-id].css` and `login.[build-id].js`

- [ ] **Step 4: Re-run the smoke test**

Run: `cargo test --manifest-path crates/trailbase-auth-bridge/Cargo.toml login_page_contains_expected_controls -- --nocapture`

Expected: PASS

- [ ] **Step 5: Commit**

```bash
git add crates/trailbase-auth-bridge/static/login/index.html crates/trailbase-auth-bridge/static/login/login.css crates/trailbase-auth-bridge/tests/login_routes.rs
git commit -m "feat: add premium trailbase login shell"
```

### Task 5: Implement password login with same-origin JSON fetch and inline errors

**Files:**
- Modify: `crates/trailbase-auth-bridge/static/login/index.html`
- Create: `crates/trailbase-auth-bridge/static/login/login.js`
- Modify: `crates/trailbase-auth-bridge/src/lib.rs`
- Test: `crates/trailbase-auth-bridge/tests/login_security.rs`

- [ ] **Step 1: Write the failing login contract test**

Cover:

- frontend submits JSON to `/api/auth/v1/login`
- invalid credentials return an inline error surface
- no token persistence is attempted via `localStorage` or `sessionStorage`
- login initiation requires the chosen same-origin CSRF defense and rejects requests missing it
- successful login does not leave the app vulnerable to session fixation under the chosen contract

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test --manifest-path crates/trailbase-auth-bridge/Cargo.toml login_security -- --nocapture`

Expected: FAIL because the JS does not exist yet.

- [ ] **Step 3: Implement minimal password-login JS**

Implement:

- `fetch("/api/auth/v1/login", { method: "POST", headers: { "content-type": "application/json" }, credentials: "same-origin" })`
- inline error rendering
- success path that redirects to validated `return_to` or default destination
- no browser storage of auth tokens
- one concrete login-CSRF defense path for the browser login surface

Use this concrete contract:

- bridge serves `/login` with a same-origin nonce or CSRF token embedded in the page
- `login.js` must echo that value in a dedicated header on password login
- bridge-side verification rejects missing or mismatched values before forwarding or accepting the login initiation path used by the page
- tests must prove the request is rejected without the token and accepted with it

- [ ] **Step 4: Re-run the security test**

Run: `cargo test --manifest-path crates/trailbase-auth-bridge/Cargo.toml login_security -- --nocapture`

Expected: PASS

- [ ] **Step 5: Commit**

```bash
git add crates/trailbase-auth-bridge/static/login/index.html crates/trailbase-auth-bridge/static/login/login.js crates/trailbase-auth-bridge/tests/login_security.rs
git commit -m "feat: wire trailbase password login flow"
```

### Task 6: Enforce login-CSRF and return-path normalization

**Files:**
- Modify: `crates/trailbase-auth-bridge/src/lib.rs`
- Modify: `crates/trailbase-auth-bridge/static/login/login.js`
- Test: `crates/trailbase-auth-bridge/tests/login_security.rs`

- [ ] **Step 1: Write failing CSRF and open-redirect tests**

Cover:

- cross-site or invalid-origin login initiation is rejected according to chosen defense
- `return_to=https://evil.example` is rejected
- `return_to=//evil.example` is rejected
- relative in-app paths survive

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test --manifest-path crates/trailbase-auth-bridge/Cargo.toml login_security -- --nocapture`

Expected: FAIL on missing CSRF/redirect normalization.

- [ ] **Step 3: Implement the minimal defense**

Complete the same concrete defense introduced in Task 5:

- same-origin login CSRF token/header contract
- strict server-side normalization of `return_to` / `next`

Keep validation server-trusted, not frontend-only.

- [ ] **Step 4: Re-run tests**

Run: `cargo test --manifest-path crates/trailbase-auth-bridge/Cargo.toml login_security -- --nocapture`

Expected: PASS

- [ ] **Step 5: Commit**

```bash
git add crates/trailbase-auth-bridge/src/lib.rs crates/trailbase-auth-bridge/static/login/login.js crates/trailbase-auth-bridge/tests/login_security.rs
git commit -m "feat: secure login redirect and csrf behavior"
```

## Chunk 3: Social Providers And Deployment

### Task 7: Wire Google and Apple provider buttons with rollout gating

**Files:**
- Modify: `crates/trailbase-auth-bridge/static/login/index.html`
- Modify: `crates/trailbase-auth-bridge/static/login/login.js`
- Modify: `deploy/k8s/trailbase-configmap.yaml`
- Test: `crates/trailbase-auth-bridge/tests/login_routes.rs`

- [ ] **Step 1: Write failing provider visibility tests**

Cover:

- Google and Apple buttons are hidden unless enabled by config
- enabled buttons point to `/api/auth/v1/oauth/google/login` and `/api/auth/v1/oauth/apple/login`
- provider buttons stay hidden unless provider preflight is explicitly marked complete
- rollout cannot switch a provider visible on UI-only config if required safety flag is absent

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test --manifest-path crates/trailbase-auth-bridge/Cargo.toml provider_visibility -- --nocapture`

Expected: FAIL

- [ ] **Step 3: Implement minimal provider gating**

Use bridge-rendered config or asset-config injection so Google/Apple buttons only appear when:

- provider preflight is marked complete in deployment config
- provider callback route is expected and configured

- [ ] **Step 4: Re-run tests**

Run: `cargo test --manifest-path crates/trailbase-auth-bridge/Cargo.toml provider_visibility -- --nocapture`

Expected: PASS

- [ ] **Step 5: Commit**

```bash
git add crates/trailbase-auth-bridge/static/login/index.html crates/trailbase-auth-bridge/static/login/login.js deploy/k8s/trailbase-configmap.yaml crates/trailbase-auth-bridge/tests/login_routes.rs
git commit -m "feat: gate google and apple login buttons"
```

### Task 8: Add Kakao as a hidden-by-default `oidc0` path

**Files:**
- Modify: `crates/trailbase-auth-bridge/static/login/index.html`
- Modify: `crates/trailbase-auth-bridge/static/login/login.js`
- Modify: `deploy/k8s/trailbase-configmap.yaml`
- Test: `crates/trailbase-auth-bridge/tests/login_security.rs`

- [ ] **Step 1: Write failing Kakao-gating test**

Cover:

- Kakao button remains hidden when `TRAILBASE_KAKAO_ENABLED=0`
- when enabled, button targets `/api/auth/v1/oauth/oidc0/login`
- Kakao cannot be enabled without explicit config flag
- Kakao cannot be enabled unless OIDC proof flag and linking-policy compatibility flag are both present

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test --manifest-path crates/trailbase-auth-bridge/Cargo.toml kakao_gate -- --nocapture`

Expected: FAIL

- [ ] **Step 3: Implement minimal Kakao visibility and config path**

Expose Kakao only through:

- dedicated env/config flag
- `oidc0` provider slot assumptions documented in deployment config comments
- explicit verification flags that represent completed issuer/claim/linking proof

Do not attempt full Kakao provider code here.

- [ ] **Step 4: Re-run tests**

Run: `cargo test --manifest-path crates/trailbase-auth-bridge/Cargo.toml kakao_gate -- --nocapture`

Expected: PASS

- [ ] **Step 5: Commit**

```bash
git add crates/trailbase-auth-bridge/static/login/index.html crates/trailbase-auth-bridge/static/login/login.js deploy/k8s/trailbase-configmap.yaml crates/trailbase-auth-bridge/tests/login_security.rs
git commit -m "feat: add gated kakao login entrypoint"
```

### Task 9: Update Docker and deployment wiring for static assets and rollback

**Files:**
- Modify: `Dockerfile.trailbase-auth-bridge`
- Modify: `deploy/k8s/trailbase-configmap.yaml`
- Modify: `deploy/k8s/trailbase-deployment.yaml`

- [ ] **Step 1: Write a failing smoke checklist entry**

Add plan-local verification targets for:

- runtime image contains login assets
- deployment env exposes public-dir and fallback flags
- rollback can redirect to built-in UI without rebuilding app code

- [ ] **Step 2: Run local build smoke**

Run: `docker build -f Dockerfile.trailbase-auth-bridge -t trailbase-auth-bridge:local .`

Expected: FAIL or missing assets until Dockerfile is updated.

- [ ] **Step 3: Implement minimal Docker/deployment changes**

Update Dockerfile to copy static login assets into the runtime image under a concrete layout such as:

- `/app/public/login/index.html`
- `/app/public/login-assets/login.<build-id>.css`
- `/app/public/login-assets/login.<build-id>.js`

Update config map/deployment to set:

- custom login enable flag
- fallback flag
- public asset dir
- default post-login path
- provider visibility flags

- [ ] **Step 4: Re-run build smoke**

Run: `docker build -f Dockerfile.trailbase-auth-bridge -t trailbase-auth-bridge:local .`

Expected: PASS

- [ ] **Step 5: Commit**

```bash
git add Dockerfile.trailbase-auth-bridge deploy/k8s/trailbase-configmap.yaml deploy/k8s/trailbase-deployment.yaml
git commit -m "feat: package premium login assets and rollout flags"
```

## Chunk 4: Verification And Cleanup

### Task 10: Add end-to-end bridge verification for login page behavior

**Files:**
- Modify: `crates/trailbase-auth-bridge/tests/login_routes.rs`
- Modify: `crates/trailbase-auth-bridge/tests/login_security.rs`

- [ ] **Step 1: Add failing end-to-end assertions**

Cover:

- `/login` returns branded page
- `/` follows canonical routing
- fallback mode returns built-in auth route redirects
- invalid `return_to` is rejected
- provider buttons appear only when configured
- versioned asset paths under `/login-assets/` are what the HTML shell actually serves
- rollback mode serves fresh fallback HTML rather than stale custom shell content

- [ ] **Step 2: Run the test suite**

Run: `cargo test --manifest-path crates/trailbase-auth-bridge/Cargo.toml -- --nocapture`

Expected: FAIL until all assertions are satisfied.

- [ ] **Step 3: Fix remaining integration issues**

Make minimal changes only where tests prove behavior is still off.

- [ ] **Step 4: Re-run full bridge tests**

Run: `cargo test --manifest-path crates/trailbase-auth-bridge/Cargo.toml -- --nocapture`

Expected: PASS

- [ ] **Step 5: Commit**

```bash
git add crates/trailbase-auth-bridge/tests/login_routes.rs crates/trailbase-auth-bridge/tests/login_security.rs crates/trailbase-auth-bridge/src/lib.rs crates/trailbase-auth-bridge/static/login/index.html crates/trailbase-auth-bridge/static/login/login.css crates/trailbase-auth-bridge/static/login/login.js
git commit -m "test: verify premium trailbase login flow"
```

### Task 11: Run deployment-facing verification commands

**Files:**
- Modify if needed: `deploy/k8s/trailbase-configmap.yaml`
- Modify if needed: `deploy/k8s/trailbase-deployment.yaml`

- [ ] **Step 1: Run format and tests**

Run:

```bash
cargo test --manifest-path crates/trailbase-auth-bridge/Cargo.toml -- --nocapture
```

Expected: PASS

- [ ] **Step 2: Build the bridge image**

Run:

```bash
docker build -f Dockerfile.trailbase-auth-bridge -t trailbase-auth-bridge:local .
```

Expected: PASS

- [ ] **Step 3: Validate manifests**

Run:

```bash
kubectl kustomize deploy/k8s >/tmp/trailbase-k8s.yaml
```

Expected: PASS with rendered YAML

- [ ] **Step 4: Commit final cleanup if needed**

```bash
git add crates/trailbase-auth-bridge Dockerfile.trailbase-auth-bridge deploy/k8s
git commit -m "chore: finalize trailbase premium login rollout"
```

## Notes For Execution

- Keep the built-in TrailBase auth UI accessible until rollback has been exercised manually.
- Do not enable Kakao in deployment config until a real provider verification run is complete.
- Prefer the smallest bridge change that preserves TrailBase as the only token issuer.
- If a task reveals that current TrailBase behavior cannot enforce the social-linking policy, stop and add the required backend enforcement before exposing provider buttons.
- Do not expose Google, Apple, or Kakao buttons from UI-only checks. Provider rollout requires callback-success, verified-email behavior, and fail-closed linking checks to be satisfied first.

## Plan-Specific Review Targets

- confirm whether `trailbase::ServerOptions` exposes enough routing knobs for `public_dir` + redirect handlers without a fork
- confirm whether login-CSRF defense should be implemented in bridge middleware or via a dedicated login nonce endpoint
- confirm whether authenticated-user bypass of `/login` can be tested with the existing bridge test harness

Plan complete and saved to `docs/superpowers/plans/2026-04-10-trailbase-premium-login-plan.md`. Ready to execute?
