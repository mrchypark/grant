# GoAuthy identity-binding verification (2026-10-24)

## Consumer contract

Grant verifies a GoAuthy binding only for the configured GoAuthy application
ID, and only when the requested client ID matches both the introspection client
ID and expected client ID. It reads
`GET /oidc/client-configuration` using that client's existing HTTP Basic
credentials. The response must use
`goauthy.client-configuration.v1`, identify the configured issuer and bootstrap
client, be enabled and confidential, and contain the exact local redirect list.
Grant also requires `authorization_code` and `refresh_token`, its configured
scopes in the allowed scope list, its resource in the allowed `audience` list,
and exactly `[resource]` in the `default_aud` list.

Grant accepts extra configured scopes, flows, and audiences because the
bootstrap client may serve other existing flows. Those extras do not alter
Grant's existing token audience, scope, application, or role checks. Metadata is
strict JSON with the exact v1 fields and arrays; its ETag must be the quoted
`sha256:<lowercase hex>` digest of the exact response bytes. Missing, stale,
unsupported, or malformed responses fail closed. The endpoint exposes no client
secret or user data, and no key is written to Grant configuration. Grant also
requires a single `application/json` Content-Type and `Cache-Control: no-store`.

## Verification status

The focused Grant package tests passed after the final request-header fix:
`go test ./internal/grant`. `go vet ./internal/grant`, `go build ./...`, and
`git diff --check` also passed in the isolated clean checkout.

## Scoped patch paths

- `internal/grant/identity_binding.go`
- `internal/grant/identity_binding_test.go`
- `internal/grant/identity_binding_goauthy.go`
- `internal/grant/identity_binding_goauthy_test.go`
- `internal/grant/service_entity_lifecycle.go`
- `internal/grant/service_entity_lifecycle_test.go`
- `internal/api/routes_lifecycle.go`
- `internal/api/lifecycle_test.go`
- `docs/api-lifecycle.md`
- `docs/verification/grant-goauthy-binding-2026-10-24.md`

No credentials or token material are recorded here.

## Production rollout and bind outcome

The deployed consumer image was built from the actual pre-change Grant image
`asia-northeast3-docker.pkg.dev/patch2-the-new-era/conalog/grant@sha256:8d14770ec01c795cb98cf49db20e64151ae34b534ed152c7e23280eb983594a6`
at repository base `dfa5e94d8bc2a212ddb8bc2b4f036eb06eedb2ec`, with only the
four scoped identity-binding source files above. The derivative retained the
base image configuration and all six existing layers, adding only the Grant
binary layer. No ConfigMap, Secret, PVC, schema, or auth configuration changed.

Before the encoding fix, Grant was deployed as
`asia-northeast3-docker.pkg.dev/patch2-the-new-era/conalog/grant@sha256:11724e4d63f81988dfce6186f608dcc20f14edb471d4eae1df893e44ef413ccb`.
The bind returned 503, `invalid Goauthy client configuration validator`. The
confirmed cause was HTTP content negotiation: Go's transport requested gzip
and transparently decompressed the body, while GoAuthy correctly returned a
weak ETag for the encoded representation. Grant hashes the received exact
body and requires a strong ETag. The minimal fix sets
`Accept-Encoding: identity` on this metadata GET only; strict ETag and
`no-store` validation remain unchanged. A regression assertion requires that
request header.

The fixed image is
`asia-northeast3-docker.pkg.dev/patch2-the-new-era/conalog/grant@sha256:de0345892e824a567fdc702bcb5309c0103f2e333ecb89d1bf95f0fb5e405083`.
It was rolled out by changing only the Grant StatefulSet container image,
guarded on the prior `11724e4d...` image and 1/1 readiness. Rollout completed
at generation/observed generation 6/6; `grant-0` was Running and Ready on the
new image ID with zero restarts. The immediate rollback target is the prior
`11724e4d...` image above; the original base `8d14770e...` is also retained
above.

After the transport fix, the existing-app bind returned 409:
`Goauthy bootstrap configuration does not match local application
requirements`. At the initial diagnosis, the response contained only that
error, and the local field mismatch had not yet been confirmed. No app
recreation, direct database access, force-ready, or manifest mutation was
performed. The later authorized owner GET and exact redirect mismatch are
recorded below; the validator remains strict.

## Prebinding redirect PATCH and final rollout

Subsequent authorized owner GET evidence confirmed the local application is
confidential and active, but has `redirectUri=null`,
`provisioningStatus=unprovisioned`, and no client binding. The producer callback
is `https://operable.dev.conalog.com/oauth/callback`; the missing local
callback was the exact proven mismatch. The existing user-authorized app PATCH
now accepts only `name` and `redirectUri`. Redirect updates are restricted to
non-platform-managed, unprovisioned, unbound applications with no active
security-mutation lease. The value uses the same canonical URL validation as
application creation; omitted values are preserved, and explicit null, empty,
or invalid values are rejected. The old redirect manifest and app state are
rechecked in the write transaction. This compare-and-swap complements the
identity binder's manifest snapshot, so a concurrent PATCH makes a stale bind
fail closed. Application type, client ID/secret, scopes, description, and
other metadata are not mutable through this route.

In the isolated clean context based at
`dfa5e94d8bc2a212ddb8bc2b4f036eb06eedb2ec`, the full Grant package tests
passed; focused redirect PATCH tests in Grant and API passed; `go vet
./internal/grant`, `go build ./...`, and `git diff --check` passed. The full API
package run had one unrelated browser E2E failure:
`OAuth discovery failed: invalid userinfo endpoint`. Pasteur's read-only review
reported no P0/P1/P2 findings.

The final amd64 image
`asia-northeast3-docker.pkg.dev/patch2-the-new-era/conalog/grant@sha256:d6b77f8c26f8bcdd090cf2d229ee8328838584e6593c8036e9c03fb66206bcc5`
was layered on the exact prior live image
`asia-northeast3-docker.pkg.dev/patch2-the-new-era/conalog/grant@sha256:de0345892e824a567fdc702bcb5309c0103f2e333ecb89d1bf95f0fb5e405083`,
replacing only `/usr/local/bin/grant`. The Grant StatefulSet was guarded on
that prior digest and only its image field was updated. Rollout completed at
generation/observed generation 7/7 with 1/1 replicas ready; the pod image ID
matches the new digest. In-pod `/readyz` returned `status=ok`,
`readReady=true`, and `writeReady=true`. The immediate rollback target is the
`de034...` image above. No config, Secret, PVC, schema, or auth setting changed.

## Parent live application and authorization verification

The parent reports that the existing owner-authorized PATCH returned 200 for
`redirectUri=https://operable.dev.conalog.com/oauth/callback`. An immediate
owner GET returned 200 with that exact callback while the application remained
`unprovisioned` and unbound, confirming the PATCH did not make the app ready.
The existing master identity-binding PUT for client `operable` then returned
200; the owner GET returned 200 with the same callback and `ready` binding.
PATCHing the ready app's callback returned 409, and reading it from another
organization returned 403. No new client, secret, or role was created.

The parent also reports a valid user+application-key
`application-authz/check` for `operable.page.read`, provider `operable`, type
`page`, and synthetic resource ID `contract-verification-no-page` returned
HTTP 200 with `allowed=false`, `reason=no_matching_binding`. This proves the
authorization decision for that supplied tuple only; Grant does not establish
that the synthetic page exists, so this is not page existence or CRUD E2E
evidence.

## Image provenance, rollback, and cleanup

The final deployed tag was
`asia-northeast3-docker.pkg.dev/patch2-the-new-era/conalog/grant:dfa5e94-redirect-uri-20261024-r2`,
resolved to the amd64 digest recorded above. It was built from clean source
base `dfa5e94d8bc2a212ddb8bc2b4f036eb06eedb2ec` plus only the four approved
identity-binding files and the bounded redirect-PATCH implementation, tests,
route, and docs listed above. Its OCI labels identify base digest
`de0345892e824a567fdc702bcb5309c0103f2e333ecb89d1bf95f0fb5e405083` and
patchset `goauthy-identity-binding+prebinding-redirect-uri`. The runtime
derivative replaced only `/usr/local/bin/grant`; the immediate rollback target
is the full prior live digest `de0345892e824a567fdc702bcb5309c0103f2e333ecb89d1bf95f0fb5e405083`.

Owned cleanup was verified: isolated worktree `/tmp/grant-redirect-build.gvfNlf/source`
and all files under `/tmp/grant-redirect-build.gvfNlf` were removed;
`/tmp/grant-binding-verify.oeeVoP` was also absent. The superseded, never
deployed candidate tag `dfa5e94-redirect-uri-20261024` (digest
`236073344cf973afb794cda15a2ef924374fe106a19aaaeba3e273f01795ebcc`) was
deleted from Artifact Registry. The temporary local Docker copies of the
final and rollback images were removed. No process from these build paths
remained. Shared BuildKit cache was not pruned because the builder is shared.
The deployed image and registry rollback target remain available. Parent-owned
token and port-forward cleanup are outside this worker's scope.
