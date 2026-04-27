# Local Execution Review Prevention Checklist

## Scope

This checklist is for REPass certificate gateway and local execution changes. It captures recurring PR review patterns from PR #16 so future changes are written with the expected review evidence already present.

## Authoring Rules

1. Never let local execution send certificate passwords, private keys, encrypted key files, or certificate bundles to the gateway. Only explicit stored-certificate registration may escrow certificate material.
2. Bind every signed challenge to the exact semantic operation: origin, organization, business number, certificate fingerprint, facility code, sync mode, month range, nonce, and expiry.
3. Execute using server-authorized challenge response fields, not caller echo fields. If the server resolves a facility, sync mode, or month range, every downstream caller must use that resolved value.
4. Validate before side effects. Authorization must happen before sync planning, path construction, artifact writes, or persistence.
5. Treat all request snapshots as untrusted. Server-side local-result ingestion must require document bytes and must never fall back to caller-provided server filesystem paths.
6. Treat all file and object paths as hostile until bounded. Validate object keys centrally, reject traversal segments, and for local worker reads require canonical paths under the collector workspace.
7. Put resource ceilings at every trust boundary: request stdin, child-process stdout/stderr, document size per file, aggregate document size, and gateway response size.
8. Keep database invariants atomic. Service-layer prechecks are not enough for uniqueness or ownership; the database schema or single SQL statement must enforce the invariant.
9. Preserve URL encoding semantics. Normalize scheme, host, and path without dropping encoded path information, and escape user-controlled path segments.
10. Cross-compile paired binaries together. A desktop-agent target must package a local worker built for the same OS and architecture.
11. Add a regression test for each reviewed failure mode. The test name should describe the externally visible behavior and should fail on the reviewed bug.
12. Update public docs whenever API request or response contracts change, including resolved challenge fields.

## Pre-PR Self-Review Questions

- Can any local path, object key, organization id, facility code, or URL segment alter storage or routing semantics?
- Can any caller-controlled field be used after a server has already resolved a safer authoritative value?
- Can any challenge be approved for one operation but executed for another?
- Does every secret-bearing flow have a named escrow boundary, and do non-escrow flows return only signatures/results?
- Are all large inputs bounded before decoding, reading, buffering, or serializing?
- Is every uniqueness or single-use guarantee enforced atomically?
- Do Mac and Windows release targets package all required runtime artifacts for the same target triple?
- Does the PR include tests that prove the negative cases reviewers are likely to ask about?
