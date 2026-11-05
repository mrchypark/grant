# Provider-bound user authorization

This is an **unreleased candidate** API. Candidate CI and actual GCS-backed
passive-learner acceptance are pending; this document makes no live PASS claim.
Published Grant v0.8.0 cannot perform application-free generic batch checks.
Its existing `/api/v1/application-authz/*` APIs retain their application
admission and consent contract unchanged.

The candidate exposes exactly two private read operations on voters and passive
learners:

| Method | Path | Result |
|--------|------|--------|
| POST | `/api/v2/providers/{providerId}/authz/batch-check` | One decision per supplied check. |
| POST | `/api/v2/providers/{providerId}/authz/filter` | Allowed IDs from supplied candidates. |

There is no single-check route, resource inventory, or mutation endpoint. Network
placement is private even though these routes share the Grant API listener.
Deployment, one-time restore and topology instructions are separate from this
consumer contract.

## Caller and authority

Send each request to the endpoint configured for its provider, with:

```http
Authorization: Bearer <original-end-user-access-token>
X-Organization-ID: <selected-organization-id>
X-Grant-App-Auth-Key: <private-service-key>
Content-Type: application/json
```

The service key authenticates the calling service; it does not identify the
user or grant resource access. Do not send a master key or substitute a service
token for the original user's Bearer credential. Reject duplicate Authorization,
AppAuthKey and organization headers rather than combining their values.

Grant verifies Goauthy introspection and matching-subject UserInfo online and
derives the local user from its existing qualified identity mapping. It requires
an active local user and active organization membership. The requested provider
must be active and have an active, unexpired, explicit `user` policy for the
server's configured `GRANT_HTTP_CAPABILITY_PEP_CLIENT_ID`. That deployment's
`GRANT_APP_AUTH_KEY` authenticates the private request. No application row,
application admission, application consent, or HTTP capability catalog is needed.
An application-mode PEP is rejected, without fallback.

The path selects one provider. Every resource and registered action must belong
to it; mixed-provider requests are rejected. A deployment bound to one provider's
PEP cannot represent another provider. Consumers needing different providers
must use their corresponding configured endpoints, not a shared URL that silently
changes provider or PEP identity.

## Batch check

Replace the placeholders below with existing registered action and resource IDs;
these examples do not register actions or grant authority.

```http
POST /api/v2/providers/<provider-id>/authz/batch-check
```

```json
{
  "checks": [
    {
      "action": "<registered-action-id>",
      "resource": {
        "provider": "<provider-id>",
        "type": "<resource-type>",
        "id": "<resource-id>"
      }
    }
  ]
}
```

A successful HTTP 200 response has this shape; the decision shown is illustrative:

```json
{
  "results": [{"allowed": true, "reason": "role_binding"}],
  "pepPolicyRevision": 1
}
```

`results` has exactly one entry per check, in input order, including repeated
checks. Each entry contains boolean `allowed` and string `reason`; HTTP 200 does
not mean every check was allowed. A valid check without matching authority can
return `allowed: false`. Invalid authentication, policy, action registration or
request structure instead fails the request; no partial batch is returned.

`{"checks":[]}` is valid. It still authenticates the user, validates current
provider/PEP policy and membership, and obtains a fence before returning
`{"results":[],"pepPolicyRevision":<positive-revision>}`.

## Filter supplied candidates

```http
POST /api/v2/providers/<provider-id>/authz/filter
```

```json
{
  "action": "<registered-action-id>",
  "resources": [
    {"provider": "<provider-id>", "type": "<resource-type>", "id": "<resource-id>"}
  ]
}
```

```json
{"resource_ids":["<resource-id>"],"pepPolicyRevision":1}
```

All candidates must use one resource type and the path provider. The response
contains only allowed candidate IDs, preserving input order and repeated allowed
IDs. It does not discover resources, return an authorized total, or provide a
continuation token. The caller supplies its candidates and must preserve their
provider/type context when using the returned bare IDs.

`resources: []` is valid with a nonempty registered action. Even this empty
filter validates the action's provider, user, membership and current fenced PEP
policy before returning an empty `resource_ids` array and positive revision.

## Closed DTOs and bounds

- Bodies are closed and field names are case-sensitive on both voters and
  learners. Batch accepts only `checks`; each check accepts `action` and
  `resource`. Filter accepts only `action` and `resources`. Each resource uses
  `provider`, `type`, and `id`.
- Unknown or duplicate JSON fields, missing required fields, null arrays and
  null required strings are rejected. Strings must be nonblank. The legacy
  `resource.view` field is unsupported, including an explicit null value.
- No subject/user ID, application ID, authorization mode or request policy
  revision override is accepted. Grant derives the subject and reads policy.
- A request contains at most **1,000 checks or candidates** and a **2 MiB
  (2,097,152-byte)** body.
  Empty arrays are different from missing or null arrays.
- A path provider is nonempty, at most 256 bytes, and contains only ASCII letters,
  digits, `.`, `_`, `:`, or `-`; encoded slashes and whitespace are not accepted.

Errors use Grant's JSON `error` envelope. Authentication failures return 401;
provider/policy/member restrictions can return 403; a missing action can return
404. Provider mismatches, mixed filter types, duplicate security headers and
item-count overflow return 400; closed-DTO violations return 422; oversized bodies
return 413. Invalid JSON syntax returns 400 and unsupported content type returns
415. A failed fence, expired read budget or unstable snapshot returns 503 rather
than an allow. Consumers must treat a failed request as having no usable result.

## Fenced reads and revision provenance

External authentication occurs once before the three-second local read budget.
A passive learner obtains a quorum-backed applied slot from the voter's private
`POST /read-fence`, synchronizes, then reads policy, identity, membership, action
ownership and RBAC at one accepted applied slot at or beyond that fence. A slot
change restarts the local operation, with at most three total attempts within
the same budget.
The final pinned read also covers empty batches and filters. Voters obtain their
own linearizable fence. There is no stale-read or application fallback.

`pepPolicyRevision` is the positive revision of the server-selected PEP policy
used in that accepted read. It is not an applied-slot number or a caller-supplied
CAS precondition. The response does not expose the applied slot, an expiry lease,
or a reusable effect permit. Recheck later protected operations; no result locks
authority or makes a subsequent backend write atomic with authorization.

## Resource namespaces and existing bindings

Authority identifies the exact `(provider, resource type, resource ID)` together
with the registered action and authenticated subject/organization. Names alone
do not make resources equivalent. The current Compos consumer maps collection
checks to resource type `object_type` with a collection ID, and row checks to
resource type `object` with a row ID. Those are consumer mappings, not built-in
Grant registrations or automatic grants. The consumer owns ID uniqueness within
each provider/type namespace; no extra collection context is inferred for a row.
Collection permission does not implicitly grant row permission. Literal `*` is
not a wildcard or an inheritance rule.

Use the existing [role/action/binding registration procedure](registration-api-management.md#등록과-사용-순서).
An authorized organization permission manager creates a binding through
`POST /api/v1/orgs/{organizationId}/bindings`; the subject organization must match
the selected organization. Logical binding uniqueness includes
`(subjectType, subjectId, subjectOrganizationId, providerId, resourceType,
resourceId, roleId)`. A binding's `id` is also unique. POST creates a binding; it
is not CAS or upsert. Adding an action to a role does not change its binding's
resource scope. This API adds no inheritance, registry or delegation mechanism.

## Distinction from HTTP capability record scopes

These generic checks evaluate supplied tuples and return observations, without
HTTP request/body digests or a staged catalog. They do not replace Exposure's
catalog-bound `/api/v2/http-capabilities/decide` forwarding contract.

The filter's 1,000-candidate bound does not increase Exposure record-scope limits.
An HTTP list capability's record scope has its own bounded candidate enumeration,
at most 199 returned IDs and a bounded equality-ID outer ACL filter combined with
the client filter. It is not an unlimited list policy. See
[HTTP capability record scopes](http-capabilities.md#record-level-list-scope)
for the separate contract and bounds.
