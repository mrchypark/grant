# Organization-context producer and PDP contract

The matching consumer protocol is `exposure/docs/organization-context.md`.
Grant supports capability-level `organizationContext: "required" | "optional"`.
Omitted means required and remains omitted on publication; explicit required
and optional each change the contract and generation digest. The typed field is
last, after `responseAuthorization`. Schema version remains 1. Null, empty,
unknown values/types, duplicate keys, case aliases and a policy on public authn
are rejected, even on disabled capabilities. Optional record scope or response
authorization is rejected. Optional batch parents require every child to opt in,
including both upsert branches. Existing immutable version and staging/activation
preconditions continue to apply. Four management self-service capabilities now
use version 2 and explicitly optional context; other tuples do not infer policy.

An absent PDP organization header is accepted only for an explicitly optional
capability. Supplied empty/whitespace or repeated organization headers are
rejected. Decision DTOs are unchanged: organization remains an HTTP header.
Catalog generation, contract, resource, credential, subject/audience and request
binding verification still run. Grant exclusively decides allow/deny.

For arbitrary providers, orgless authorization requires an explicit direct user
entitlement as described in [database-schema.md](database-schema.md). Provision
through the master-key-only operator endpoint (replace the example IDs):

```sh
curl -X PUT "$GRANT_URL/ops/orgless-bindings/inventory-self-read" \
  -H "X-Grant-Master-Key: $GRANT_MASTER_KEY" \
  -H 'Content-Type: application/json' \
  --data '{"subjectId":"user-1","providerId":"inventory","resourceType":"item","resourceId":"item-1","actionId":"inventory.item.read","state":"active"}'
```

The provider, action and active local user must already exist. The endpoint
creates the reserved global role itself. Tenant administrators cannot mint these
grants. Application admission is still required. No organization membership,
consent, application-owner organization or other tenant rights are substituted
for an absent organization. Change an existing grant's state to disabled with
the same exact tuple to revoke, including after an admission dependency is
deactivated or removed. Resource IDs preserve exact supported values, including
brace and URN UUID forms. The four trusted
management self-service rules additionally support an external identity before
its first local materialization; they remain Grant-specific domain policy.

Independent fixed vectors live in
`internal/grant/testdata/organization-context-vectors.json`. Verify them with
`scripts/verify-organization-context-vectors.sh` (jq/OpenSSL, independent of Go),
and `go test ./internal/grant -run TestOrganizationContext -count=1`.
The omission vector is the legacy encoding; changing only policy changes both
pins. Editing policy while reusing an old pin fails validation.

Grant publication/PDP HTTP checks:

```sh
go test ./internal/api -run '^TestOrganizationContextHTTPPolicy$' -count=1 -v
```

The separately owned real Exposure integration harness is
`internal/api/exposure_organization_context_e2e_test.go`. Its runner is:

```sh
EXPOSURE_SOURCE_DIR=/absolute/path/to/exposure \
  go test ./internal/api -run '^TestExposureOrganizationContextE2E$' \
  -count=1 -v -timeout=180s
```

It uses real Grant storage, service, admin HTTP publication and PDP plus the
actual Exposure executable. Identity verification and the protected upstream are
local fixtures. A normal run without `EXPOSURE_SOURCE_DIR` skips this external
integration. Run the explicit command before claiming interoperability.
