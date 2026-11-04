# Explicit custom-action application eligibility

An active organization owner with `organization.permission.manage` can opt one
of that organization's custom actions within any existing active provider into
application eligibility. The examples use `operable`; the same routes accept
another registered provider, such as `independent-authz`:

1. GET `/api/v1/orgs/{org}/providers/operable/actions` and retain its `ETag`.
   For an actual active owner with permission-management authority this returns
   the existing eligible candidates plus only this organization's own ineligible
   custom actions in that provider. Other organizations' custom action metadata is
   excluded even when eligible. Owners without permission-management authority use the
   eligible-only application-management view as other callers. Both
   representations remain JSON arrays and carry an ETag.
2. PATCH `/api/v1/orgs/{org}/providers/operable/actions/{action}` with
   `If-Match: <ETag>` and `{"grantableToApplications":true}`.
3. Separately admit the action using the existing application action routes.
   For replacement, GET `/api/v1/orgs/{org}/apps/{app}/actions`, preserve the
   actions that should remain, and PUT that path with its own `If-Match` and
   explicit `actionIds`. The existing single-action PUT also remains available
   with its existing semantics (it has no snapshot CAS).
4. Where configured, obtain explicit user consent using the existing consent
   routes. Role actions and resource bindings remain separate requirements.

Creation still defaults to false and does not accept an eligibility field.
PATCH omission preserves eligibility; explicit null and non-booleans are 422.
Every explicitly supplied boolean, including an idempotent value, requires CAS:
missing `If-Match` is 428, stale is 412. Metadata-only PATCH keeps its existing
behavior. Use a fresh GET after visible catalog changes; the catalog-view validator
is conservative and can conflict on unrelated visible action changes. Hidden
foreign custom action changes do not invalidate it. The validator uses
existing action snapshot hashing and staged transaction comparison, rather than
an application action-set ETag. Owner, permission, active provider, namespace,
and snapshot checks execute again in the same transaction as the action update.
A concurrent authority change rejects the write (409); a changed catalog returns
412. Failures do not apply a partial metadata or eligibility update.

Only `<providerId>.custom.org-<sha256-of-org-id>.<feature>.<definition-id>.<operation>`
actions in the caller's own namespace can be changed. A role called owner is
insufficient: active owner membership and permission are both required. Built-in,
system, other-provider and foreign-organization actions are excluded. Each provider
reserves this code namespace for the named organization; a matching canonical ID
and provider are required for mutation. An organization owner cannot create
providers or mint built-in/shared codes. Existing Operable service entry points
remain wrappers and retain their paths, IDs, and defaults.

Eligibility creates no application admission, role action, binding, or consent.
Both single-add and replacement admission transactions reject foreign custom
organization namespaces in every provider, even when the global action flag is
already true. Existing shared provider eligibility is unchanged: ordinary codes
including `custom.shared` in other providers remain shared, while Operable keeps
its existing broader `custom.*` isolation. Existing foreign admissions are
not migrated by this change.

To disable eligibility, explicitly remove all application admissions first.
`false` rejects with 409 while application action or consent-action references
remain; it does not cascade. Scope removal retains its existing consent-action
cleanup. Role permissions remain untouched. The master-only catalog APIs retain
their existing separate administrative behavior, including cascading removal.
They require the deployment master key and do not satisfy this bounded owner
workflow.

Application-management authorization, application snapshot CAS for replacement,
trusted first-party/auto-approve mutation restrictions, consent and runtime RBAC
remain in force. Eligibility is not a bypass for a trusted application's 403.
No private approval API, OAuth client, or application-key issuance API is added.
The management caller must still meet the existing HTTP capability admission
and consent requirements. No permission or application action is auto-granted.

Goauthy deployment readiness is a separate prerequisite. Public application
creation records a Grant application and leaves it unprovisioned; it does not
create an IdP client. The operator must supply the configured fixed application
and existing client, with introspection and expected bootstrap-manifest
verification. This change does not relax client verification or first-party
management protections. Test fixtures establish ready non-first-party admission
behavior only; they do not verify the current Operable client/workspace mapping.
That mapping remains unverified until the owner supplies its deployment context.

Public candidate catalogs apply the same organization custom namespace rule as new
application admission, including actions whose eligibility flag is true. This
narrows the former globally eligible candidate view; a foreign custom action is
not a valid candidate for this organization's application. Built-in/shared
provider actions retain their existing eligibility behavior. Global admin
catalog APIs retain their full catalog and separate master authorization.

A separate legacy limitation remains: `SetRoleAction` and
`ReplaceRoleActionsAuthorized` validate the target role and action existence,
but do not check custom-action organization namespaces. Known foreign action IDs
can therefore remain in or be selected into roles, and existing role reads may
expose their metadata. These paths are unchanged and do not implicitly admit an
action to an application. Existing foreign role/admission rows are not migrated;
this change does not claim to close every legacy authorization/read surface.
