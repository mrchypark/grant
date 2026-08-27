# Provider HTTP capabilities

Grant stores authorization-significant HTTP contracts; it does not proxy provider traffic and never stores an upstream origin.

The Phase A API is inactive by default:

- `POST /api/v1/admin/providers/{providerId}/http-capability-generations` stages one immutable next generation.
- `PUT /api/v1/admin/providers/{providerId}/pep-clients/{pepClientId}` binds one globally unique active/ready application identity to one provider.
- `POST /api/v1/admin/providers/{providerId}/http-capability-active` atomically activates or rolls back the provider pointer.
- `GET /api/v2/http-capabilities/catalog/{providerId}` returns only the active pointer generation to the configured service-authenticated PEP application.
- `POST /api/v2/http-capabilities/decide` re-resolves the active capability, verifies the exact method/path/query/resource/digest/credential evidence, evaluates the existing RBAC/application policy tuple and returns a short-lived receipt.

Admin routes require `X-Grant-Master-Key`. Fetch and decision require `X-Grant-App-Auth-Key`; the server additionally resolves that credential to the single application configured as `GRANT_HTTP_CAPABILITY_PEP_APPLICATION_ID` and checks its active PEP binding. Authorized decisions also require the same end-user Bearer credential and organization context used by the provider-forwarding request.

`public_authn` entries deliberately have no action/resource and never call the decision endpoint. They are still exact catalog entries; Exposure owns their limiter, secret redaction and fixed-origin forwarding. `authorized` entries must name an existing provider action and one closed resource extractor. Phase A accepts only `pathSegment`, `queryScalar`, `jsonPointer`, and `constant`; `stringTemplate` is not a stageable contract. A query scalar must be a required member of the exact query policy. A JSON pointer is the explicit body-extraction trust boundary: the authenticated Exposure PEP supplies the scalar, while Grant binds it to the exact forwarded JSON body digest and length, the opaque forwarded-header digest, principal, and request. Grant does not independently parse or verify the `Content-Type` header value. Phase B must prove that same-principal/request extraction before enabling such a capability. Unknown extractor kinds, methods, paths, query shapes, stale catalogs, disabled capabilities and cross-provider PEP identities fail closed.

Phase A does not create a provider, stage a generation, bind a PEP, activate a pointer, change Patch Data rules or alter the existing P0 `plant.get` APIs.
