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
  organization lifecycle (deactivate/reactivate). Includes fine-grained
  per-action membership for roles and applications.
- **Admin routes** (`/api/v1/admin/...`): providers, provider actions
  (replace, create, update, delete), capability catalog staging/activation,
  application trust and verified identity binding, user listing and activation.

## Explicit exclusions

The following routes are intentionally excluded from the management
catalog. They are private, infrastructure, or authorization endpoints
that do not participate in the management grant model:

- `POST /api/v1/admin/bootstrap/owner` (unified provider-neutral bootstrap)
- `PUT /api/v1/admin/orgs/{organizationId}/apps/{applicationId}/actions/{actionId}` (master-only operational first-party admission; never exposed through the admin UI registry)
- `POST /ops/backups`, `GET /ops/backups` (private backup ops)
- `GET/PUT /ops/providers/{providerId}/enforcers/{pepClientId}` (private, master-only PEP policy read/registration; excluded from the management catalog)
- `POST /ops/migrations/patch-data-authority` (private import)
- `POST /api/v1/authz/check`, `batch-check`, `filter`
- `POST /api/v1/application-authz/check`, `batch-check`, `filter`
- `GET /api/v2/http-capabilities/catalog/{providerId}`
- `POST /api/v2/http-capabilities/decide`

Bootstrap, authorization, health, backup, import, enforcer registration,
and runtime-private routes are never part of the management catalog.
They operate outside the grant model and are protected by master-key
or network policy. Bootstrap is provider-neutral and does not create
applications automatically.

## Security model

### Platform bearer access

Platform routes accept a user Bearer token only when
`GRANT_MANAGEMENT_ORGANIZATION_ID` names the trusted operator organization
and `X-Organization-ID` matches it. Reserve this organization for operators:
its role administrators can grant platform privileges. An unset value disables
platform Bearer access. Other organizations cannot grant themselves access.

Grant authenticates the user and checks the exact action against resource
`grant-management/platform/grant`. Legacy application mode additionally checks
application admission and trust or consent; the released native user selector
below omits only those application checks.
Admin routes containing an organization path still require this platform
privilege; the path identifies the organization being administered.

Internal callers can continue using `X-Grant-Master-Key`. An explicitly supplied
invalid master key cannot fall back to Bearer authentication. Exposure strips
this header and never receives the master key in its configuration.

<a id="candidate-native-platform-user-authorization-under-review"></a>

### Released native platform user authorization

The former native gap was that `NativeAuthorizePlatformManagement` always
performed an application check: a generic real-PDP/business-mock test could
succeed with zero applications while Grant's own upstream handler still failed.
The Grant v0.8.0 fix selects user authorization only from the active,
explicit `user` policy for the canonical `grant-management` PEP matching
`GRANT_HTTP_CAPABILITY_PEP_CLIENT_ID`. It uses fenced Goauthy user, active
membership and exact platform RBAC checks without an application record.
The configured ManagementOrg guard above remains mandatory in both native
authorization and the user PDP, even when another organization has a matching
role binding. Missing PEP, noncanonical provider or explicit application mode
retains legacy native application admission; none implicitly selects user.
A disabled canonical user PEP denies access rather than falling back.

This is Grant's own control-plane policy using its existing canonical management
provider constant, not a selector based on business application names, and does
not automatically rename providers. It adds
no request headers, flags, approval step or automatic application provisioning.
For this management path, Exposure forwards the original Bearer and organization
to Grant upstream, never a master key or AppAuthKey; AppAuthKey is used only for
the private PDP call. The native management selector and the user PDP's matching
ManagementOrg guard passed final CI and the phase 1 process fixture with
Exposure v0.3.0; see the [full CI run](https://github.com/mrchypark/grant/actions/runs/41950457179).
This does not establish live IED, deployed Goauthy or remote learner verification.
See the [principal specification](principal-user-authz-spec.md) and
[actual upstream interop evidence](http-capabilities.md#released-grant-native-upstream-verification).

### forward-user authorization

All management routes use `forward-user` request authorization. Grant
resolves the Bearer token to an authenticated principal. The authorization
decision checks the following; the released native platform user path above
omits only application admission and trust/consent, never membership or RBAC:

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

Bootstrap, authz, health, backup, import, enforcer registration,
and runtime routes are private to the cluster. They are never part
of the management catalog and are protected by master-key or network
policy. Health probes do not require a management permission.

- Bootstrap (`POST /api/v1/admin/bootstrap/owner`): provider-neutral,
  does not create applications automatically.
- Backups (`/ops/backups`): private operational endpoint.
- Import (`/ops/migrations/patch-data-authority`): private migration endpoint.
- Enforcer registration (`/ops/providers/{providerId}/enforcers/{pepClientId}`):
  private, master-only GET/PUT. GET returns stored policy and `ETag`
  even for inactive/expired PEPs or absent active catalogs. PUT accepts
  `authorizationMode`, `lifecycle` and optional `validUntil`; creation requires
  `If-None-Match: *`, updates require `If-Match` (missing 428, stale 412).

The released server policy defaults to `application`; explicit `user` needs
no application record but retains user, membership and RBAC checks. Legacy
stored null defaults during migration; modern PUT null is rejected. No OAuth
client/application provisioning or per-PEP key issuance is added: runtime
authentication keeps global `GRANT_APP_AUTH_KEY` and configured PEP identity.
User catalog mode/revision and decision `wireVersion: 2`/`pepPolicyRevision`
are supported by the released Grant v0.8.0/Exposure v0.3.0 pair; legacy responses
omit new fields. Published v0.7.0 does not support this mode. See the
[principal specification](principal-user-authz-spec.md) and
[released capability contract](http-capabilities.md#released-principal-authorization-mode)
for the contract and phase 1 evidence; live deployment qualification is separate.

## Released upgrade and coexistence boundaries

Restore does **not** rename providers or map action prefixes. Legacy PEPs receive
only the compatibility defaults `authorizationMode: application` and revision 1.
The existing `grant` provider's six bootstrap coarse actions and any existing
custom actions are preserved; bootstrap does not create `grant-management`. Do not delete or rewrite
those records as a prerequisite for user mode.

Create the canonical `grant-management` provider, its exact actions/role grants,
catalog and a new globally unique PEP ID additively alongside existing records.
An old PEP ID remains globally owned and cannot be reused for another provider,
even after disabling or deleting it. A runtime has only one configured
`GRANT_HTTP_CAPABILITY_PEP_CLIENT_ID`: storage coexistence does not provide
simultaneous runtime routing for both identities. Coordinate the Grant/Exposure
configuration and catalog cutover, retaining the ManagementOrg and exact role
checks described above. No prefix mapping or automatic application provisioning
is part of this migration.

Use the existing [private bootstrap and registration APIs](registration-api-management.md#초기-등록과-운영-전용-api)
and the [catalog registration sequence](#staging-before-activation) with their
documented master/private boundaries. `grant-management-catalog` exists as CLI
source, but the released runtime image does **not** ship that executable.
Use source-pinned [release assets](https://github.com/mrchypark/grant/releases/tag/v0.8.0)
when published: `management-actions-g1.json`, `management-catalog-g1.json`,
`SOURCE.json` and `SHA256SUMS`. Before use, verify the payload checksums against
`SHA256SUMS` and the source revision in `SOURCE.json` against the intended
release/source; otherwise hold the rollout. These names do not establish
successful publication or distribution PASS. Availability of a checked-in
manifest is not established here. The producer workflow
[`export-management-catalog.yml`](../.github/workflows/export-management-catalog.yml)
can export these files for an exact source revision and generation; the runtime
image is not their producer.

Apply the [archive/schema compatibility constraints](backup-restore.md) to the
actual source and destination. The defaults above do not promise arbitrary
v0.4.1 archive restore compatibility or authorize an in-place archive conversion,
namespace deletion or replacement of existing data.

### Apply producer artifacts through HTTP

Verify `SOURCE.json` identifies the intended release source and generation, and
verify the downloaded payloads against `SHA256SUMS` before applying anything.
Use the operator/master boundary for provider action, staging and activation
requests below; PEP policy remains private master-only.

| Artifact | HTTP use |
|----------|----------|
| `management-actions-g1.json` | Action definition array, not a raw bulk PUT body. |
| `management-catalog-g1.json` | Complete generation-1 stage request; send unchanged. |
| `SOURCE.json`, `SHA256SUMS` | Source/generation provenance and payload verification. |

1. GET `/api/v1/admin/providers/grant-management/actions` and retain its exact
   `ETag`. Review and merge the existing array with the generated action array,
   preserving unrelated actions and resolving ID/code/metadata conflicts rather
   than overwriting them. Existing `grant` actions are not moved into this set.
2. For individual POST `/api/v1/admin/providers/grant-management/actions`, send
   one generated action object with `providerId` removed. POST uses a closed DTO:
   that extra field is rejected with 422. For whole-set PUT to the same path,
   remove `providerId` from each reviewed action and wrap the array as
   `{"actions":[...]}`. PUT currently ignores that extra field, but it is not part
   of the request DTO. Project the reviewed array onto the accepted fields:

   ```sh
   jq '{actions: map({id, code, name, description, riskLevel,
     visibilityGranting, grantableToApplications, consentLabel,
     consentDescription})}' reviewed-merged-actions.json > actions-request.json
   ```

   Send the PUT with `Content-Type: application/json` and `If-Match` containing
   the GET ETag. Missing preconditions return 428; stale values return 412.
   After 412, GET and review the merge again instead of retrying unconditionally.
3. POST the unchanged stage JSON to
   `/api/v1/admin/providers/grant-management/http-capability-generations`.
   Generation 1 is only for a provider with no stored generations. Otherwise
   obtain a new producer artifact for the next generation after the latest
   stored generation, including staged generations. The catalog digest binds
   the generation; the producer also supplies its expected previous generation.
   Regenerate, never edit g1 into gN.
4. GET `/api/v1/admin/providers/grant-management/http-capability-generations/N`
   and verify the stored content and digest. Complete exact role/resource grants,
   applicable native coarse permissions and application requirements, and active
   unexpired PEP setup before activation. Staging does not change the active pointer.
5. GET `/api/v1/admin/providers/grant-management/http-capability-active`, then POST
   to that path with `expectedGeneration` and `expectedCatalogDigest` from the
   observed active state, plus `targetGeneration` and the verified
   `targetCatalogDigest`. Use null expectations only when no active catalog exists.
   Resolve a concurrent-state conflict by rereading; do not drop the CAS expectations.

### Authorization layers and evidence limits

User-mode PDP omits caller-application admission/consent; it does not remove a
native endpoint's coarse RBAC checks. Ordinary tenant routes may require both
an existing `grant` action such as `organization.group.manage` on the tenant
organization and the corresponding canonical `grant-management` route action.
Existing `grant` bindings are not canonical aliases. A canonical binding is
reusable only for the same provider, resource type and exact resource ID, with
the required role actions and subject scope.

The actual zero-application native-upstream interop covers the platform catalog
read route, not the entire canonical catalog. Ordinary tenant authentication
does not inherently resolve a caller application, but application-management
operations and onboarding are not uniform zero-application flows. Full-catalog
apps=0 behavior is not proven by the platform-read test.

## Registry structure

Each route entry contains:

| Field | Description |
|-------|-------------|
| `Method` | HTTP method (GET, POST, PUT, PATCH, DELETE). |
| `Path` | Go 1.22+ ServeMux path template. |
| `ActionCode` | Canonical action code (e.g. `applications.read`). |
| `Platform` | True for admin-only `/api/v1/admin/` routes. |

## Action code format

Action IDs follow the pattern `grant-management.<actionCode>`. For
example, `applications.read` becomes `grant-management.applications.read`.

Every action is `GrantableToApplications: true` and uses `RiskLevel: high`.

A deprecated legacy `manage` action (`grant-management.manage`) is
retained so the old catalog can remain active during migration or rollback.
No new registered routes use this action code, and retaining it does not
authorize the new routes.

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

GET routes use `mode: exact` with the following optional parameter;
the admin user listing additionally accepts `q`, `limit`, and `cursor`:

| Parameter | Format | MaxBytes | Required |
|-----------|--------|----------|----------|
| `consistency` | `text` | 32 | false |

The `consistency` parameter accepts any case/whitespace variant of the
backend value (e.g. `strong`, ` Strong `, `STRONG`). Non-GET routes
(mutations) use `mode: none` — query strings are rejected.

### Admin user listing

`GET /api/v1/admin/users` is a bodyless platform route with action code
`admin.users.read` (Action ID `grant-management.admin.users.read`) against
resource `grant-management/platform/grant`. It uses the platform Bearer gate
above or the existing master-key access path; it is not organization-scoped.

- `q` is an optional literal substring filter over the existing Grant User
  fields `id`, `email`, and `displayName`, not a regex or wildcard expression.
  SQLite LIKE case matching applies; `%`, `_`, and `\` are escaped literally.
- `limit` is a positive decimal integer, defaults to 50, and is capped at 100
  users per page (larger valid integers are clamped).
- `cursor` is optional. Pagination is bounded and ordered by Grant user ID
  ascending; subsequent pages continue strictly after the cursor's user ID.
- A successful response is `200` with `{"items":[...],"nextCursor":"..."}`.
  `items` contains existing Grant User fields: `id`, optional `email`,
  `displayName`, and `active`. `nextCursor` is omitted when no next page exists.
- Results include inactive/disabled Grant users (`active: false`). This is
  the Grant directory, not an identity-provider account listing; no external
  subject identifiers, identity mappings, credentials, or secrets are returned.
- Each page and its lookahead use one strong consistent SQL read. Optional
  `consistency` accepts only `strong` (including the variants described above);
  it cannot weaken this read. Separate pages do not share a frozen snapshot.
- The exact query policy allows only `q`, `limit`, `cursor`, and `consistency`,
  each at most once. Decoded UTF-8 byte limits are 1024 for `q` and `cursor`,
  20 for `limit`, and 32 for `consistency`. Malformed encoding, control characters,
  duplicates, unknown keys, invalid limits, and request bodies return `400`.
  Empty `q` or `cursor` values are equivalent to omitting them. An empty result
  is `{"items":[]}`.

## Request and response policies

- **Body routes** (POST/PUT/PATCH with JSON body): `body: json`,
  `maxBytes: 2097152` (2 MiB), `contentTypes: ["application/json"]`.
- **Bodyless routes** (GET, DELETE, bodyless POST/PUT):
  `body: none`, `maxBytes: 0`.
- **Forward headers**: every route forwards `x-organization-id`.
  JSON body routes additionally forward `content-type`.
- **Concurrency control**: action-set replacement and consent PUT forward
  `if-match`; consent PUT also forwards `if-none-match`. The matching GET
  returns `etag`. Missing preconditions return `428`; stale values return
  `412`. Every response contract permits these statuses and forwards `etag`.
  Individual link PUT/DELETE and metadata PATCH do not require a set ETag.
- **Authorization**: all routes use `forward-user`.
- **Response**: `maxBytes: 4194304` (4 MiB), `redirects: deny`,
  `errorStatuses: [400, 401, 403, 404, 409, 422, 500, 503]`.
  Guarded mutations additionally include `412, 428`.

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

`GET /api/v1/admin/providers/{providerId}/http-capability-generations/{generation}`
reads a stored immutable generation, including a staged or historical generation,
using a strong read on a voter. It reuses the platform `admin.catalog.read` action
and the existing master-key or management-organization bearer authorization.
It returns `providerId`, `generation`, `catalogDigest`, `capabilityCount`,
`responseResourceCount`, `capabilities`, and `responseResources`; both arrays are
present even when empty. It includes no activation or PEP lease metadata and never
changes the active pointer. Invalid/zero/out-of-range generations return 400;
the accepted range is `1..9223372036854775807`. An absent provider/generation pair
returns 404. Shared actions appear once in the generated action definitions,
even when multiple routes use them.

## Migration instructions

### Staging before activation

1. Obtain the source-pinned producer stage artifact for `N=P+1`, where P is
   the latest stored generation; use generation 1 only when none exists. Follow
   [artifact verification and HTTP application](#apply-producer-artifacts-through-http).
2. Review the output for correctness.
3. POST the stage payload to
   `POST /api/v1/admin/providers/grant-management/http-capability-generations`.
4. GET `/api/v1/admin/providers/grant-management/http-capability-generations/N`
   and verify the stored generation content and digest match the expected values.
5. Complete the exact-action role migration below and verify identity, active
   membership and RBAC for the intended principals. In application mode also
   complete application admission and trust/consent migration; user mode does
   not require an application record, admission or consent.
6. Activate with
   `POST /api/v1/admin/providers/grant-management/http-capability-active`,
   providing the observed active `expectedGeneration`/`expectedCatalogDigest`
   and verified `targetGeneration`/`targetCatalogDigest`. Both expectations are
   null only when there is no active catalog.

Never activate a catalog without staging and digest pinning first. The
activation endpoint requires the exact `targetCatalogDigest` to prevent
unintended catalog switches.

### Preserving old manage action and action unions

The deprecated `grant-management.manage` action is included in the
action definitions output (`-actions` flag). When replacing provider
actions, include the legacy action to preserve existing role/app/consent rows
while the old catalog is active. This does not translate their permissions:

Use the producer action array and the reviewed merge, DTO projection and ETag
procedure in [Apply producer artifacts through HTTP](#apply-producer-artifacts-through-http).

The action set is a union: new fine-grained actions are added alongside
existing coarse actions. Existing coarse actions (e.g.
`organization.application.manage`) are not removed. Both layers
coexist; the coarse layer is the existing authorization gate, and the
fine-grained catalog layer is the defense-in-depth gate.

### Existing bindings

Role bindings reference roles; `role_actions` associates those roles with
action IDs. The old three-route catalog used `grant-management.manage` for
`GET /groups`, `POST /groups`, and `DELETE /groups/{groupId}` under an organization.
Before activating the new catalog, migrate those existing grants to exactly:

- `grant-management.groups.read`
- `grant-management.groups.create`
- `grant-management.groups.delete`

Use `PUT /api/v1/orgs/{organizationId}/roles/{roleId}/actions/{actionId}`
to add each exact role action while preserving other actions, including `manage`
for rollback. Existing canonical organization bindings can be reused only when
their provider is `grant-management`, resource type is `organization`, exact
resource ID and subject scope still match, and their role has the required actions.
Provider-`grant` bindings are not aliases. User mode
requires verified identity, active membership and the exact RBAC grant, without
application admission or consent. In application mode, also use
`PUT /api/v1/orgs/{organizationId}/apps/{applicationId}/actions/{actionId}`
to add each application action without removing unrelated actions. Trusted apps must temporarily
leave trusted mode to change their action set, then have trust explicitly
restored by the operator. For non-trusted apps, each affected user must update
their consent through `PUT /api/v1/orgs/{organizationId}/apps/{applicationId}/consent`,
preserving their other consented actions and supplying the current GET ETag as
`If-Match` (or `If-None-Match: *` for first consent). Do not fabricate consent or set trust
merely to bypass this step.

Register definitions and stage the catalog while the old generation remains
active. Check the new exact action decisions for previously allowed and denied
principals, then activate using the expected old generation/digest. If any
required identity, membership or role grant is missing, keep the old generation
active. In application mode, missing required app admission or trust/consent
also blocks activation; those application conditions do not apply to user mode.
Retaining only `manage` will deny the new routes. This is an explicit
operator data migration, not a startup migration or an authorization fallback.
Never expand a legacy group grant into all management actions; additional
administration rights require a separate reviewed grant.

For additional explicitly granted management capabilities:

- Platform bindings use the `platform` resource type with resource ID
  `grant` (constant).
- Organization bindings use the `organization` resource type with
  resource ID bound to the `{organizationId}` path segment.
- Service bindings (me, providers) use the `service` resource type
  with resource ID `grant` (constant).

## Usage

Use the verified producer artifacts through the
[HTTP application sequence](#apply-producer-artifacts-through-http). The released
runtime image does not ship the generator. For a later generation, obtain newly
generated artifacts for that generation instead of modifying a previous stage.

## Runtime access model

The catalog defines the universe of management capabilities. At
runtime:

1. Actions must be registered via
   `PUT /api/v1/admin/providers/grant-management/actions`.
2. A capability generation must be staged and activated.
3. Organization-scoped requests in both modes require verified identity and active membership.
   Application mode additionally requires application admission and trust or
   explicit consent. User mode requires no application record, admission or consent.
4. Role bindings must grant the required action on the target resource, except for the exact organizationless onboarding paths below.

No step is implicit. The catalog is a generator; the API is the
control plane.

### Organizationless onboarding

Only these exact management capabilities admit requests without a selected organization: bodyless `POST /api/v1/me/ensure`, `GET /api/v1/me`, `GET /api/v1/orgs`, and `POST /api/v1/orgs`. They require the configured active/ready first-party auto-approve management app, an active app organization and Provider, its matching active enforcer, explicit app admission of the exact Action, and a verified token client. Existing inactive local users remain denied. The ensure capability retains the stable Action ID `user.materialize`; the legacy `/api/v1/me/materialize` path is not admitted.

The decision, normal authentication lookup, and `GET /api/v1/me` use a namespaced external identity digest without creating a local user or updating a profile. Missing `GET /api/v1/me` mappings return 404 with code `LOCAL_USER_NOT_PROVISIONED`. The voter handles only `POST /api/v1/me/ensure`: it creates a missing local user and identity mapping, reuses an existing user without profile writes, and returns only `{"userId":"..."}`. Ensure creates no membership, role Binding, or consent. Self-service organization creation accepts only `id` and `name`, sets the verified current user as owner, and atomically creates organization-scoped coarse and management-owner Bindings. Missing management Action definitions reject creation.

Normal organization and data APIs retain their authorization checks. Exact organization-management capabilities additionally support the configured trusted management app across its own organization boundary, but still require the caller to be an active member of the target organization with its fine-grained RBAC grant. See [the lifecycle contract](api-lifecycle.md) for prerequisites, ownership transfer, and safe updates to an existing trusted app.

## Lifecycle catalog expansion

The current branch adds organization creation/list/profile, Provider metadata
PATCH, group/agreement metadata PATCH, application metadata/active/delete,
role metadata/delete, organization membership listing/removal, and administrator
consent revocation. The former provisioning-verification route is replaced by
`PUT /api/v1/admin/orgs/{organizationId}/apps/{applicationId}/identity-binding`.
Grant no longer serves `/auth` or `/auth/v1/*`; identity validation remains,
while Gateway routes IdP traffic directly.

The binding endpoint links an existing externally managed client after the
configured read-only adapter verifies it. Only Rauthy supports that verification
today; PocketBase continues to create ready applications without client IDs.
No adapter fallback, client creation or credential issuance occurs. Existing
verified bindings and the legacy storage column are preserved.

See [complete lifecycle route semantics](api-lifecycle.md) and the
[method/path contract](porting-go-http-contract.md). Provider POST now returns
409 on duplicates instead of updating existing rows. Relationships retain PUT;
action-set replacement retains conditional PUT. Metadata PATCH preserves
omission and does not provide new ETag protection.

Publication requires generating and reviewing a new management catalog,
registering its action definitions, explicitly assigning the required actions
to management applications and roles, and activating the new catalog with the
matching Exposure digest. Existing activation does not automatically publish
new routes or grant their permissions. This document does not claim deployment
or live acceptance of this branch.
