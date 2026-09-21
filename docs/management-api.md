# Management API catalog

The management catalog is the default catalog generator for Grant's
management HTTP routes. It produces a reviewable enumeration of every
business management endpoint, its action code, resource binding, and
transport policy. The catalog is code — the registered route definitions
and API handlers remain the runtime source of truth. The inventory test
(`TestManagementCatalogCoversRegisteredRoutes`) compares the catalog
against the live route registrations to detect drift.

Runtime actions and capabilities are explicitly registered data. They are
created via the API (stage, activate, replace actions) and do not grant
access implicitly. The catalog defines what _can_ be registered; the
API defines what _is_ registered.

## Scope

The catalog covers:

- **Service routes** (`/api/v1/me`, `/api/v1/providers`): current user
  and provider listing.
- **Organization management** (`/api/v1/orgs/{organizationId}/...`):
  applications, groups, members, agreements, roles, bindings, and
  organization lifecycle (deactivate/reactivate).
- **Admin routes** (`/api/v1/admin/...`): providers, provider actions,
  capability catalog staging/activation, PEP client configuration,
  application trust and provisioning verification, user activation,
  import, and backup management.

## Explicit exclusions

The following routes are intentionally excluded from the management
catalog. They are internal, infrastructure, or authorization endpoints
that do not participate in the management grant model:

- `POST /api/v1/admin/bootstrap/rauthy-owner`
- `POST /api/v1/admin/bootstrap/external-owner`
- `POST /api/v1/authz/check`, `batch-check`, `filter`
- `POST /api/v1/application-authz/check`, `batch-check`, `filter`
- `GET /api/v2/http-capabilities/catalog/{providerId}`
- `POST /api/v2/http-capabilities/decide`

Bootstrap, authorization, health, and runtime-private routes are never
part of the management catalog. They operate outside the grant model
and are protected by their own authentication and authorization
mechanisms.

## Security model

### Platform bearer access

Platform routes accept a user Bearer token only when
`GRANT_MANAGEMENT_ORGANIZATION_ID` names the trusted operator organization
and `X-Organization-ID` matches it. Reserve this organization for operators:
its role administrators can grant platform privileges. An unset value disables
platform Bearer access. Other organizations cannot grant themselves access.

Grant authenticates the user and checks application admission, trust or consent,
and the exact action against resource `grant-management/platform/grant`.
Admin routes containing an organization path still require this platform
privilege; the path identifies the organization being administered.

Internal callers can continue using `X-Grant-Master-Key`. An explicitly supplied
invalid master key cannot fall back to Bearer authentication. Exposure strips
this header and never receives the master key in its configuration.

### forward-user authorization

All management routes use `forward-user` request authorization. Grant
resolves the Bearer token to an authenticated principal. The authorization
decision checks:

1. **Active membership**: the principal must have an active membership
   in the organization identified by `{organizationId}`.
2. **Admission**: the application (if present) must be admitted to the
   organization.
3. **Trust or consent**: trusted applications skip explicit consent;
   untrusted applications need active user consent for the requested actions.
5. **RBAC**: the principal's role bindings must grant the required
   action on the target resource.

Both the coarse authorization API and the fine-grained catalog gate
must pass. Organization service methods require the existing core
actions (e.g. `organization.application.manage`) in addition to the
new fine-grained catalog action (e.g. `applications.read`). The catalog
gate is a defense-in-depth layer, not a replacement.

### Inactive organization reactivation

The `POST /api/v1/orgs/{organizationId}/reactivate` route is the only
route available to principals authenticated against an inactive
organization. All other organization-scoped routes are denied for
inactive organizations. The `inactive` auth mode resolves the
membership without requiring the organization to be active. Application
admission, trust or consent, and the exact reactivation role binding still
apply at the PDP; the backend also requires its core management permission.

### Private routes

Bootstrap, authz, health, and runtime routes are private to the
cluster. They are never part of the management catalog and are
protected according to each endpoint's own key, token, and network policy.
Health probes do not require a management permission.

## Registry structure

Each route entry contains:

| Field | Description |
|-------|-------------|
| `Method` | HTTP method (GET, POST, PUT, DELETE). |
| `Path` | Go 1.22+ ServeMux path template. |
| `ActionCode` | Canonical action code (e.g. `applications.read`). |
| `Platform` | True for admin-only `/api/v1/admin/` routes. |

## Action code format

Action IDs follow the pattern `grant-management.<actionCode>`. For
example, `applications.read` becomes `grant-management.applications.read`.

Every action is `GrantableToApplications: true` and uses `RiskLevel: high`.

A deprecated legacy `manage` action (`grant-management.manage`) is
retained for backwards catalog compatibility. No registered routes use
this action code.

## Resource types

Routes declare one of three resource types:

| Resource type | Condition | ID extractor |
|---------------|-----------|--------------|
| `platform` | Platform routes (`Platform: true`) | `constant`, value `grant` |
| `organization` | Routes containing `{organizationId}` | `pathSegment`, name `organizationId`, format `asciiToken` |
| `service` | All other routes (me, providers) | `constant`, value `grant` |

### organization ID format

The `asciiToken` format accepts: `A-Z`, `a-z`, `0-9`, `.`, `-`, `_`,
`:` — maximum 256 bytes. This preserves existing transport identifiers
such as `Ops_1` and `tenant.EU:42`. Path segment validation applies the
same `asciiToken` bounds at the transport layer; encoded segments are
not permitted unless declared as path parameters.

## Query policies

GET routes use `mode: exact` with a single optional parameter:

| Parameter | Format | MaxBytes | Required |
|-----------|--------|----------|----------|
| `consistency` | `text` | 32 | false |

The `consistency` parameter accepts any case/whitespace variant of the
backend value (e.g. `strong`, ` Strong `, `STRONG`). Non-GET routes
(mutations) use `mode: none` — query strings are rejected.

## Request and response policies

- **Body routes** (POST/PUT with JSON body): `body: json`,
  `maxBytes: 2097152` (2 MiB), `contentTypes: ["application/json"]`.
- **Bodyless routes** (GET, DELETE, bodyless POST/PUT):
  `body: none`, `maxBytes: 0`.
- **Forward headers**: every route forwards `x-organization-id`.
  JSON body routes additionally forward `content-type`.
- **Authorization**: all routes use `forward-user`.
- **Response**: `maxBytes: 4194304` (4 MiB), `redirects: deny`,
  `errorStatuses: [400, 401, 403, 404, 409, 422, 500, 503]`.

## Generation and digest

`ManagementCatalog(generation, previous)` produces an immutable
`StageHttpCapabilityGeneration` with:

- `schemaVersion: 1`
- `generation`: must be 1 without a previous, or `previous + 1`.
- `catalogDigest`: SHA-256 of the canonical generation JSON.
- Each capability has a `contractDigest`: SHA-256 of its canonical JSON.

The catalog is deterministic: identical inputs always produce identical
digests. The CLI tool `grant-management-catalog` emits either the stage
payload or the action definitions for review.

## Migration instructions

### Staging before activation

1. Generate the stage payload with `grant-management-catalog -generation N -previous-generation P`
   for `N=P+1`; omit the previous generation for the initial generation 1.
2. Review the output for correctness.
3. POST the stage payload to
   `POST /api/v1/admin/providers/grant-management/http-capability-generations`.
4. Verify the staged generation digest matches the expected value.
5. Activate with
   `POST /api/v1/admin/providers/grant-management/http-capability-active`,
   providing `targetGeneration` and `targetCatalogDigest`.

Never activate a catalog without staging and digest pinning first. The
activation endpoint requires the exact `targetCatalogDigest` to prevent
unintended catalog switches.

### Preserving old manage action and action unions

The deprecated `grant-management.manage` action is included in the
action definitions output (`-actions` flag). When replacing provider
actions, include the legacy action to preserve backwards compatibility
with existing bindings that reference it:

```sh
grant-management-catalog -generation 1 -actions > actions.json
# Merge with existing provider actions, preserving the legacy manage
# Wrap the merged array for the registration API (provider is in the URL).
jq '{actions: map(del(.providerId))}' actions.json > actions-request.json
```

The action set is a union: new fine-grained actions are added alongside
existing coarse actions. Existing coarse actions (e.g.
`organization.application.manage`) are not removed. Both layers
coexist; the coarse layer is the existing authorization gate, and the
fine-grained catalog layer is the defense-in-depth gate.

### Existing bindings

Existing role bindings reference action IDs. When migrating to the new
catalog:

- Bindings referencing `grant-management.manage` continue to work
  because the action is retained.
- New bindings should reference the fine-grained action IDs
  (e.g. `grant-management.applications.read`).
- Platform bindings use the `platform` resource type with resource ID
  `grant` (constant).
- Organization bindings use the `organization` resource type with
  resource ID bound to the `{organizationId}` path segment.
- Service bindings (me, providers) use the `service` resource type
  with resource ID `grant` (constant).

## Usage

```sh
# Stage generation 1
grant-management-catalog -generation 1

# Emit actions for review
grant-management-catalog -generation 1 -actions

# Advance to generation 2
grant-management-catalog -generation 2 -previous-generation 1
```

## Runtime access model

The catalog defines the universe of management capabilities. At
runtime:

1. Actions must be registered via
   `PUT /api/v1/admin/providers/grant-management/actions`.
2. A capability generation must be staged and activated.
3. Applications must be admitted, and either trusted or explicitly consented.
4. Role bindings must grant the required action on the target resource.

No step is implicit. The catalog is a generator; the API is the
control plane.
