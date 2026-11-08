# Native gRPC transport for the passive learner PDP

Exposure is the policy enforcement point (PEP); Grant's passive learner is the policy decision point (PDP). This optional native gRPC connection carries capability catalogs and decisions for HTTP and unary gRPC business calls, as well as provider USER batch checks. It carries no business traffic. The existing [HTTP capability](http-capabilities.md) and [unary gRPC capability](grpc-capabilities.md) contracts still define the evidence, policy, leases and receipts. The existing HTTP PDP endpoints remain available for migration and other clients. The learner still uses the [Rhiza replica and voter read fence](runtime-modes.md) to authorize against a current snapshot; the gRPC listener does not replace that path.

This document describes the Grant source in the integration worktree and the expected Exposure client behavior. It does not establish that Exposure implements this transport, that the pair has passed end-to-end tests, or that a release or deployment has changed.

## Pinned service and payloads

The original four RPCs are defined in [api/pdp/v1/pdp.proto](../api/pdp/v1/pdp.proto) at Grant commit `5f9aa4bd4d1c47ebd3de6555f0c01fd9f4c61a27`. The Go import is `github.com/mrchypark/grant/api/pdp/v1`. To use the additive USER batch RPC, opt in with a client and server pinned to a source revision or bindings artifact that includes it; the original pin does not expose this method. No release or Exposure support is assumed.

The service is `grant.pdp.v1.PolicyDecisionPoint`. All methods are unary:

| Method | Request | Response |
| --- | --- | --- |
| `GetHttpCatalog` | `provider_id` | `catalog_json`: active HTTP capability catalog |
| `DecideHttp` | `evidence_json`: HTTP capability decision input | `decision_json`: HTTP capability decision |
| `GetGrpcCatalog` | `provider_id` | `catalog_json`: active unary gRPC capability catalog |
| `DecideGrpc` | `evidence_json`: unary gRPC capability decision input | `decision_json`: unary gRPC capability decision |
| `BatchCheckProviderUser` | `provider_id`, `checks_json`: provider USER batch body | `decision_json`: ordered USER batch results and PEP policy revision |

The protobuf envelope carries JSON bytes using the existing typed Grant DTOs. Preserve their field names, integer precision, closed request schemas, digest preimages and receipt checks. Grant decodes evidence into those types and encodes the typed result; it does not tunnel HTTP requests, HTTP statuses or raw HTTP responses. The listener exposes no legacy simple-read, admin, reflection or streaming RPCs.

## Metadata and decisions

- Every RPC requires exactly one `x-grant-app-auth-key` metadata value, matching the configured Grant app key exactly. Grant also checks the configured PEP's provider binding and active policy through the existing capability service.
- Every decision or batch RPC requires exactly one `authorization` value with the original nonempty Bearer credential. Grant trims outer whitespace and preserves the scheme, space and token for any capability credential digest. Credentials belong in metadata, outside the JSON.
- `DecideGrpc` and `BatchCheckProviderUser` require exactly one nonempty `x-organization-id`. For `DecideHttp`, omit it only when the existing HTTP capability and authorization mode permit omission. If supplied, it must be a single nonempty value. Grant trims outer whitespace. Catalog fetches need only the app key.
- An evaluated denial can return gRPC `OK` with `allowed: false`. For capability decisions, forwarding requires an explicit allow and a valid, matching, unexpired receipt. A gRPC failure, malformed response, mismatched evidence or missing required receipt must deny the affected operation. Do not retry authorization over HTTP.

| gRPC status | Meaning |
| --- | --- |
| `InvalidArgument` | Duplicate required metadata values, malformed Bearer or organization metadata, a decoded protobuf request rejected by Grant (for example, unknown fields), or malformed or invalid JSON evidence. Grant maps domain errors 400 and 422 here. |
| `Unauthenticated` | Missing or invalid app key, missing Bearer metadata on a decision, or a domain authentication failure (401). |
| `PermissionDenied`, `NotFound` | Domain authorization or binding error (403), or a missing catalog or unregistered batch action (404). An unregistered action fails the entire batch. |
| `FailedPrecondition` | Domain precondition failure (409, 412 or 428). |
| `ResourceExhausted` | A message or JSON size limit was exceeded (including domain 413). |
| `Unavailable` | The PDP cannot establish authority, such as an unavailable read fence (domain 503). |
| `DeadlineExceeded`, `Canceled`, `Internal` | Deadline, cancellation, or another server failure. |

If malformed wire bytes fail decoding before the Grant handler runs, grpc-go determines the status. The PEP still fails closed.

The catalog response, decision evidence and USER batch request are each limited to 2 MiB of JSON; decision and batch responses are limited to 64 KiB of JSON. The server's gRPC send and receive message limit is 2 MiB + 1,024 bytes (2,098,176 bytes), including protobuf overhead. This is a message limit, separate from the JSON limits. Each RPC handler caps processing at 10 seconds; a shorter caller deadline still applies. The transport deadline does not extend any catalog lease or decision receipt.

## Opt in to provider USER batch checks

On the existing passive learner listener, call `/grant.pdp.v1.PolicyDecisionPoint/BatchCheckProviderUser`. Set `ProviderUserBatchRequest.provider_id` to the configured provider and `checks_json` to the closed [HTTP USER batch-check body](provider-user-authorization.md#batch-check), for example:

```json
{"checks":[{"action":"example.read","resource":{"provider":"example","type":"item","id":"one"}}]}
```

The provider on each tuple must equal `provider_id`, and the action must belong to that provider. The existing provider-bound USER service verifies the original Goauthy credential, active membership and USER PEP policy, and evaluates up to 1,000 exact checks in one fenced snapshot. It needs no capability catalog or application admission. The `decision_json` response is the existing `{"results":[{"allowed":true,"reason":"role_binding"}],"pepPolicyRevision":1}` shape. Results follow request order, including repeated checks; an empty batch still validates identity, policy and the fence. Errors return no partial result.

Correlate each result to the pending check at the same position. Verify that the result count matches the request and the policy revision is positive. The response carries no tuple IDs or request binding, so the client cannot independently verify the order. gRPC `OK` alone grants nothing. The revision is not an applied slot or a reusable receipt, and authorization is not atomic with a later backend effect. There is no filter or streaming RPC. An older listener can return `Unimplemented`; fail closed without fallback.

## Listener and client selection

The listener is off by default. Set `GRANT_PDP_GRPC_LISTEN_ADDR` to a TCP `host:port` on a passive Grant learner. The process rejects this listener on voters, promotion learners and remote-only runtimes. For TLS, set both `GRANT_PDP_GRPC_TLS_CERT_FILE` and `GRANT_PDP_GRPC_TLS_KEY_FILE`; a partial pair, or TLS files without a listen address, is invalid.

Plaintext is allowed only on a literal loopback bind address, such as `127.0.0.1:8443` or `[::1]:8443`. Hostnames such as `localhost`, wildcard binds and other nonloopback addresses require TLS 1.2 or later. In TLS mode, the client must trust the server certificate chain and verify the expected hostname or IP against the certificate. The Grant listener does not require a TLS client certificate; the app key remains the PEP credential.

Keep this listener on private ingress and restrict reachability to the intended PEPs, including when TLS is enabled. Do not publish it on public ingress. The app key is checked in the handler after grpc-go has received and decoded the protobuf envelope; it does not protect the listener from unauthenticated connections or decoding work.

Configure Exposure explicitly for the intended PDP transport and endpoint. For native gRPC, use the configured trust and server identity; never disable certificate verification or downgrade to plaintext on failure. During migration, switch between HTTP and gRPC by explicit configuration. A failed gRPC request must never trigger HTTP fallback.

Exposure issues one logical PDP RPC per operation. Do not retry in application code, configure gRPC retry or hedging policies, or resend after an ambiguous outcome. Use grpc-go's [`WithDisableRetry`](https://github.com/grpc/grpc-go/blob/v1.84.0/dialoptions.go#L678) to disable configured retries. It leaves transparent retries enabled when nothing was written to the wire or the library knows the server application did not receive the RPC for processing. Only that library behavior is permitted, within the original RPC context and deadline, with the same request payload and, for capability decisions, the same request binding. A single logical RPC can therefore have more than one physical transport attempt.

Before rollout, verify the pinned client and a real passive learner together: exercise the RPCs the client uses, compare their typed results with the HTTP equivalents, and verify capability receipt binding. For USER batches, also check mixed allow/deny results, revocation and fence loss. Confirm that a denied decision, bad certificate, lost read fence, or expired deadline yields no business call and no HTTP fallback. These are acceptance checks for the pair, not results established by reviewing the Grant source.

On Darwin arm64, run the opt-in HTTP business capability smoke against the executable from Exposure CI run `42467000290` (attempt 2), artifact `13611614991`, source `5bf054c32ae55aff0397ee2bb341ba5df4926e98`. Verify the archive SHA-256 `79352e6b0057550fa4834e25c9d3d1b136d3a56d7e353e3f5588447c1fe234dd` before extracting. Set `EXPOSURE_BINARY` to the absolute path of the executable:

```sh
GRANT_EXPOSURE_NATIVE_PDP_INTEROP=1 \
GRANT_EXPOSURE_BINARY="$EXPOSURE_BINARY" \
GRANT_EXPOSURE_BINARY_SHA256=0be83b3fb1407a51657b2dc5f0356ed1471f3fdf642f2068628edc6d7d9b014e \
  go test -mod=readonly -run '^TestPrincipalExposureNativePDP$' -count=1 -timeout=2m -v ./internal/api
```

The test skips by default. It uses only loopback fixtures and covers USER and application authorization, role revocation, fence loss, upstream credential isolation, and zero requests to a configured HTTP fallback trap. It does not exercise the unary gRPC business capability RPCs or qualify a deployment.
