# Design Specification: Bounded Resource Reverse Lookup (Unified `/filter`)

## 1. Problem & Goal

External services integrating with `grant` use `/api/v1/authz/filter`, `/api/v1/application-authz/filter`, and `/api/v2/providers/{providerId}/authz/filter`. Previously, these endpoints only accepted up to 1,000 candidate `resources` per request. When a service needs to discover which container resources (e.g., `workspace`, `store`, `namespace`, `organization`) or explicitly shared individual `resource_id`s a user can access—without scanning its entire database first—it needs a bounded reverse-lookup mode.

This specification extends the existing **`/filter` endpoints** to support **paginated, bounded reverse lookup** (when `resource: { provider, type }` is supplied instead of `resources`) returning distinct `resource_id`s granted via active `role_bindings` (and implicit owner authority on `grant`/`grant-management` `organization` resources) in `grant`, and enables `/api/v1/authz/*` on `learner` nodes with `readScope` fencing.

---

## 2. Non-Goals & Boundary Invariants

1. **No Unbounded List**: Every reverse-lookup request is strictly bounded (`limit` default `100`, maximum `1000`) with deterministic SQLite `BINARY` (bytewise UTF-8) keyset pagination (`cursor` / `next_cursor`).
2. **Explicit Bindings Only**: Reverse lookup evaluates only `role_bindings` stored in `grant` (plus the synthetic `membership_role = 'owner'` branch for `grant`/`grant-management` `organization` resources). It does **not** enumerate implicit child resources stored only in external service databases, and does **not** support wildcard (`*`) matching.
3. **Coarse Allow/Deny Preserved**: No field-level or `resource.view` projection is added.

---

## 3. HTTP API Contract

### 3.1 Endpoints

| Endpoint | Auth | Consistency & Fencing | Learner Node |
|---|---|---|---|
| `POST /api/v1/authz/filter` | `master` (`x-grant-master-key`) | `?consistency=strong` supported on voter; `localReplicaRead` (`readScope` fencing) on learner | **Allowed** (`masterAuthorizationRead`) |
| `POST /api/v1/application-authz/filter` | `application` (`x-grant-app-auth-key` + Bearer/Gateway + `x-organization-id`) | `ReadStrong` + `localReplicaRead` (`readScope` fencing on learner) | **Allowed** (`applicationAuthorizationRead`) |
| `POST /api/v2/providers/{providerId}/authz/filter` | `application` (`x-grant-app-auth-key` + Goauthy `Bearer` + `x-organization-id`; no Gateway) | `ReadStrong` inside `capabilityAuthorizationRead` (`readScope` 3s retry + final `SELECT 1` proof) | **Allowed** (`providerUserAuthorizationRead`) |

### 3.2 Request Payloads (Reverse-Lookup Mode)

`resources` and `resource` are mutually exclusive (`400` if both provided; `422` if neither provided). `limit` (`1..1000`, default `100`) and `cursor` are only allowed when `resource` is provided (`400` otherwise).

**`POST /api/v1/authz/filter`**:
```json
{
  "subject": {
    "type": "user",
    "id": "user-123",
    "org_id": "org-1"
  },
  "action": "example-service.workspace.read",
  "resource": {
    "provider": "example-service",
    "type": "workspace"
  },
  "limit": 100,
  "cursor": "ws-0042"
}
```
- Reuses `api.subject` (`type`, `id`, `org_id`/`orgId`). `subject.type` must be `"user"` (values `"group"` or `"organization"` are rejected with `400`).

**`POST /api/v1/application-authz/filter`**:
```json
{
  "action": "example-service.workspace.read",
  "resource": {
    "provider": "example-service",
    "type": "workspace"
  },
  "limit": 100,
  "cursor": "ws-0042"
}
```

**`POST /api/v2/providers/{providerId}/authz/filter`**:
```json
{
  "resourceType": "workspace",
  "action": "example-service.workspace.read",
  "limit": 100,
  "cursor": "ws-0042"
}
```
- Path parameter `providerId` is validated via `grant.ValidateProviderID` (`422 invalid providerId` if invalid) and duplicate `x-organization-id` headers are rejected with `400`.

**Decoding & Validation Rules**:
- All three endpoints use strict JSON decoding (`closed = true`, rejecting unknown fields and trailing tokens with `422` / `400`).
- `providerId`, `resourceType`, `action` (and `subject.type`, `subject.id`, `subject.orgId` on v1 `authz`) are required non-empty strings (`422` on missing/empty).
- `action` is the canonical `actions.id` (e.g. `example-service.workspace.read`), **not** `actions.code`.
- `limit` (`*int` with `optional:"true"`): omitted or `null` defaults to `100`. Values `<= 0` when explicitly provided or `> 1000` return `400 Bad Request`.
- `cursor` (`*string` with `optional:"true"`): omitted, `null`, or `""` starts from the beginning; non-empty values are used verbatim (untrimmed) as an opaque keyset token.

### 3.3 Response Payloads

**v1 (`authz` and `application-authz`)**:
```json
{
  "resource_ids": ["ws-0043", "ws-0044"],
  "next_cursor": "ws-0044"
}
```

**v2 (`providers/{providerId}/authz/lookup-resources`)**:
```json
{
  "resource_ids": ["ws-0043", "ws-0044"],
  "next_cursor": "ws-0044",
  "pepPolicyRevision": 12
}
```
- Matches `UserFilterResult` (`json:"resource_ids"`, `pepPolicyRevision`).
- `resource_ids` is always a JSON array (`[]` when empty, never `null`), sorted ascending in SQLite `BINARY` order.
- `next_cursor` has `omitempty` and is omitted when `<= limit` rows are returned (last/empty page) on both v1 and v2.
- On v2, if `out.PEPPolicyRevision <= 0 || out.ResourceIDs == nil`, the handler returns `503` (`"incomplete provider user authorization result"`).

---

## 4. Store Query & Evaluation Semantics

Implemented in `internal/grant/store_authz.go`:

### 4.1 Caller & Mode Pre-Checks
- **`v1/authz`**: No pre-auth lookup; if `users`, `service_providers`, `organizations`, or `actions` gates fail in SQL, returns `200 {"resource_ids": []}` (does not support `allow_inactive_self_reactivation`).
- **`v1/application-authz`**: Resolves caller identity (`Bearer` or `TrustedGateway`), verifies active local `users` + `organizations` + `applications` (`rauthy_client_id`, or `ExternalCheck.ApplicationID` / `Goauthy.ApplicationID`), failing closed (`401`/`403`) on invalid/inactive user, org, or app.
- **`v2/providers/{providerId}/authz`**: Requires `AuthProviderMode() == "goauthy"` (`403`), validates `configuredPEP` (`authorization_mode = 'user'`, active/unexpired, `policy_revision > 0`), pre-validates `SELECT id, provider_id FROM actions WHERE id = ?` (`404` if missing, `400` if `action.ProviderID != providerId`), and verifies final `SELECT 1` `readScope` proof (`scope.pinned && !scope.changed && scope.slot >= scope.fence`, `503` otherwise).

### 4.2 SQL Query & Guards
1. **Eligibility Gates**:
   - `users.id = $userId AND users.active = 1`
   - `service_providers.id = $providerId AND service_providers.active = 1`
   - `organizations.id = $organizationId AND organizations.inactivated_at IS NULL`
   - When `$providerId = 'grant' AND $resourceType = 'organization'`, exclude inactivated target organizations (`target_org.inactivated_at IS NULL`).
2. **RBAC + Owner Union** (using `valid_agreements` CTE from `agreementBoundsSQL` and leveraging `idx_role_bindings_resource_lookup`, `idx_organization_memberships_user_org_status`, `idx_group_memberships_member_group`, `idx_role_actions_action_role`):
   - `subject_type = 'organization'` (`rb.state = 'active'`, `rb.subject_organization_id = $organizationId`, `rb.agreement_id IS NULL OR valid_agreements` active window, active `organization_memberships`, system-owner role guard `rb.role_id NOT IN ('role-system-bootstrap-owner', 'role-system-management-owner') OR EXISTS (... membership_role = 'owner')`).
   - `subject_type = 'user'` (same guards + active `organization_memberships` + `rb.agreement_id IS NULL OR valid_agreements`).
   - `subject_type = 'group'` (same guards + `group_memberships` + active `organization_memberships` + `rb.agreement_id IS NULL OR valid_agreements`).
   - **Synthetic Owner Branch** (matching `store_authz.sql:165-186`): When `$providerId IN ('grant', 'grant-management') AND $resourceType = 'organization'`, an active `organization_memberships` row with `membership_role = 'owner'` and matching `role_actions` + `actions` grants `resource_id = $organizationId`.
   - Every RBAC branch joins `role_actions ra ON ra.role_id = rb.role_id AND ra.action_id = $actionId` and `actions act ON act.id = ra.action_id AND act.provider_id = rb.provider_id`.
3. **Application / Consent Intersection (`v1/application-authz` only)**:
   - Intersects with active `applications` (`organization_id = $organizationId AND active = 1 AND provisioning_status = 'ready'`), `application_actions` (`action_id = $actionId`), and either `first_party = 1 AND consent_policy = 'auto_approve'` or active `application_grants` + `application_grant_actions`.
   - (`v2/providers/{providerId}/authz` does **not** intersect `applications`/`consent`, matching `UserFilter`).
4. **Keyset Pagination**:
   - Filter `rb.provider_id = $providerId AND rb.resource_type = $resourceType` and `($cursor = '' OR rb.resource_id > $cursor)` (SQLite `BINARY`).
   - `SELECT DISTINCT resource_id ... ORDER BY resource_id ASC LIMIT $limit + 1`; if `len(rows) == limit + 1`, truncate to `limit` and set `next_cursor = rows[limit-1]`.

---

## 5. Inventory, Wiring, & Documentation Updates

- **API Wiring**:
  - `internal/api/authz.go`: Register `/api/v1/authz/lookup-resources` and `/api/v1/application-authz/lookup-resources`; update `applicationAuthorizationBody` type switch.
  - `internal/api/provider_user_authz.go`: Register `/api/v2/providers/{providerId}/authz/lookup-resources`; update `providerUserAuthorizationRead` and `providerUserAuthorizationBody` type switch.
  - `internal/api/http.go`: Update `applicationAuthorizationRead` to include `/api/v1/application-authz/lookup-resources`.
- **Inventory & Tests**:
  - `internal/api/testdata/routes-voter.txt` & `internal/api/management_inventory_test.go` (`excluded` map)
  - `internal/api/http_test.go` (`TestLearnerOnlyPublishesPDP` asserting `503` on `/api/v1/authz/lookup-resources` for learners)
  - `internal/api/provider_user_authz_test.go` (boundary + real-replica tests) & `internal/grant/service_user_authz_test.go` / store pagination tests
- **Docs**:
  - Update `docs/SPEC.md`, `docs/authorization-model.md`, and `docs/provider-user-authorization.md` (updating the "exactly two private read operations" statement to include `lookup-resources`).
