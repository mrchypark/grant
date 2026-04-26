# REPass Asset Registry Schema Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Move REPass from a Korean RPS plant-centered schema to an asset-centered schema with registry/profile/source foundations.

**Architecture:** `assets.id` becomes the canonical facility and authorization resource. Korean RPS identifiers move into `asset_profiles` and registry mapping tables. Permission cache tables move from plant naming to asset naming.

**Tech Stack:** Rust workspace, hiqlite/SQLite DDL in `crates/infra/src/database.rs`, serde domain models in `crates/domain`, integration tests in `crates/infra/tests`, schema documentation in `docs/database-schema.md`.

---

## File Structure

- `crates/domain/src/lib.rs`: add `Asset`, `AssetProfile`, registry/source/certificate/credential structs and rename resource enum semantics to include `Asset`.
- `crates/infra/src/database.rs`: replace `plants` schema with `assets` plus new v2 tables; rename effective permission cache tables to asset names; add persistence/query helpers.
- `crates/infra/tests/database_schema.rs`: assert v2 tables and columns exist.
- `crates/infra/tests/authorization_integration.rs`: update authorization behavior from plant resource to asset resource.
- `crates/api/src/lib.rs`: later slice, expose asset endpoints or compatibility handlers.
- `docs/database-schema.md`: document actual schema and relation rationale.

## Task 1: Schema Foundation

**Files:**
- Modify: `crates/infra/tests/database_schema.rs`
- Modify: `crates/infra/src/database.rs`
- Modify: `docs/database-schema.md`

- [x] **Step 1: Write failing schema test**

Add assertions that `assets`, `asset_profiles`, `registries`, `registry_accounts`, `registry_projects`, `data_sources`, `generation_data`, `energy_certificates`, `auth_credentials`, `credential_bindings`, `country_capabilities`, `effective_organization_asset_actions`, `effective_user_asset_actions`, and `effective_user_asset_visibility` exist.

- [x] **Step 2: Run test to verify it fails**

Run: `cargo test -p infra --test database_schema ensure_schema_creates_authorization_tables`

Expected: FAIL because asset-centered tables do not exist yet.

- [x] **Step 3: Implement schema DDL**

Add the v2 tables to `Database::ensure_schema_only()`. Since the service is not launched, remove the canonical dependency on `plants` in new code paths rather than preserving plant as the source of truth.

- [x] **Step 4: Run test to verify it passes**

Run: `cargo test -p infra --test database_schema ensure_schema_creates_authorization_tables`

Expected: PASS.

## Task 2: Domain Types

**Files:**
- Modify: `crates/domain/src/lib.rs`
- Modify: `crates/domain/tests/model_roundtrip.rs`

- [x] **Step 1: Write failing model roundtrip assertions**

Add JSON roundtrip assertions for `Asset`, `AssetProfile`, `Registry`, `RegistryAccount`, `RegistryProject`, `DataSource`, `GenerationDatum`, `EnergyCertificate`, `AuthCredential`, and `CredentialBinding`.

- [x] **Step 2: Run test to verify it fails**

Run: `cargo test -p domain --test model_roundtrip`

Expected: FAIL because the new types are missing.

- [x] **Step 3: Add domain structs**

Add serde-compatible structs with `camelCase` API fields and snake_case enum variants where used.

- [x] **Step 4: Run test to verify it passes**

Run: `cargo test -p domain --test model_roundtrip`

Expected: PASS.

## Task 3: Authorization Resource Rename

**Files:**
- Modify: `crates/domain/src/lib.rs`
- Modify: `crates/infra/src/database.rs`
- Modify: `crates/infra/tests/authorization_integration.rs`
- Modify: `crates/api/tests/organization_authorization.rs`

- [x] **Step 1: Write failing authorization tests**

Update behavior tests to bind `RoleBindingResourceType::Asset` and assert visibility/action rows are written to `effective_*_asset_*`.

- [x] **Step 2: Run test to verify it fails**

Run: `cargo test -p infra --test authorization_integration`

Expected: FAIL because validation and cache rebuild still use plant tables.

- [x] **Step 3: Implement asset resource validation**

Validate asset bindings against `assets.owner_organization_id`, agreement grantor, and grantee organization. Rebuild effective asset action and visibility tables.

- [x] **Step 4: Run focused tests**

Run: `cargo test -p infra --test authorization_integration`

Expected: PASS.

## Task 4: KR RPS Compatibility Mapping

**Files:**
- Modify: `crates/infra/src/service.rs`
- Modify: `crates/infra/src/database.rs`
- Modify: `crates/infra/tests/authorization_integration.rs`

- [x] **Step 1: Write failing persistence test**

Assert that an RPS-collected facility persists an `assets` row and a `kr_rps` `asset_profiles` row whose `external_id` equals the old `facility_code`.

- [x] **Step 2: Run test to verify it fails**

Run: `cargo test -p infra --test authorization_integration`

Expected: FAIL because RPS sync still writes only `plants`.

- [x] **Step 3: Implement KR RPS mapping**

Map `BridgeFacility.code` to `asset_profiles.external_id`, write core facility fields into `assets`, and write KR-specific values into profile metadata.

- [x] **Step 4: Run focused tests**

Run: `cargo test -p infra --test authorization_integration`

Expected: PASS.

## Task 5: Documentation

**Files:**
- Modify: `docs/database-schema.md`
- Modify: `docs/authorization-model.md`

- [x] **Step 1: Update schema documentation**

Document `asset_id` as the canonical facility key and explain where country/registry identifiers live.

- [x] **Step 2: Update authorization documentation**

Replace plant visibility semantics with asset visibility semantics and document additional registry/credential execution checks.

- [x] **Step 3: Verify docs mention no stale canonical plant rule**

Run: `rg -n "plant_id.*canonical|canonical plant|effective_.*plant|resource_type.*plant" docs`

Expected: no stale statements that claim plant is the global canonical resource.

## Verification

Run before completion:

```bash
cargo test -p domain --test model_roundtrip
cargo test -p infra --test database_schema
cargo test -p infra --test authorization_integration
cargo test -p api --test organization_authorization
```
