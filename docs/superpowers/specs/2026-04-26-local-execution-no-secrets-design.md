# Local Execution Certificate Boundary

## Confirmed Requirement

`runEphemeral` and every local-execution flow must keep certificate bundles, certificate passwords, and private keys on the user's machine. The backend may receive certificate material only for an explicit stored-certificate registration flow whose purpose is server escrow.

The forbidden local-execution fields are:

- `password`
- `bundle`
- `signPriKeyBase64`
- `kmPriKeyBase64`
- raw decrypted PKCS#8 private key material

## Current Violation

The current SDK `runEphemeral` delegates to the desktop agent `/approve-ephemeral-run` endpoint. The agent asks the user for certificate approval and password, signs a gateway challenge, then posts the password and full proof bundle to `/cert-gateway/ephemeral-executions`. The Rust backend validates the bundle and runs the RPS collector server-side.

That is server execution with uploaded secrets. It is not local execution.

## Target Design

Local execution is an agent-owned execution mode:

1. The SDK calls the desktop agent for `runEphemeral`.
2. The desktop agent prompts natively for certificate use.
3. The certificate password and private key are used only inside the local agent or a local worker process.
4. RPS login and collection happen locally.
5. The agent submits only non-secret collected results, artifact bytes/metadata, run status, and an optional signed proof to the backend.
6. Stored registration remains separate and explicit: `registerStored` may send the bundle/password to `/certificates` because its purpose is backend escrow.

## Implementation Guardrails

- The public SDK `runEphemeral` path must not call a gateway method whose request type contains `bundle` or `password`.
- The desktop-agent local execution handler must not construct a gateway request containing certificate bundle fields.
- The existing gateway `ephemeral-executions` endpoint is an escrow/server-execution endpoint and must not be used by the default local execution API.
- Full local execution may upload RPS document artifact bytes as `contentBase64`, but must clear local source paths and must not include certificate bundle/private-key/password fields.
- Browser certificate escrow methods must keep explicit names and `escrow: true` opt-in.
- Tests must search the serialized request bodies, not only TypeScript or Go types, because secret exposure is a wire-level failure.
- Documentation and examples must distinguish:
  - local execution: no certificate material leaves the machine
  - stored registration: explicit backend escrow
  - escrowed browser execution: explicit legacy/server execution, not local

## First PR Scope

The first implementation must make the unsafe path impossible to use accidentally:

- Rename SDK/agent local execution call sites so `runEphemeral` targets local execution semantics.
- Remove or isolate any default `runEphemeral` dependency on `GatewayClient.executeEphemeralExecution`.
- Add regression tests that fail if local execution serializes `bundle`, `password`, `signPriKeyBase64`, or `kmPriKeyBase64`.
- Keep explicit escrow methods for existing backend execution compatibility.

If the full local collector cannot be packaged in this PR, `runEphemeral` must fail closed with a clear local-executor-unavailable error rather than silently falling back to uploading secrets.

## Implemented Hardening Notes

- Desktop local execution now routes to a local executor hook and returns `501` when no local worker is configured.
- `/sign` is signature-only and rejects non-`cert-gateway:` challenges.
- The desktop agent validates requested business number against the selected certificate before local execution or stored registration.
- Gateway challenge creation can bind `origin`, `businessNo`, `facilityCode`, `startMonth`, `endMonth`, and `signCertSha256Hex` into the hashed payload.
- Server-side `ephemeral-executions` remains an explicit escrow/server-execution compatibility endpoint, not the default local execution path.
