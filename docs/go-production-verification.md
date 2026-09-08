# Go deployment verification — 2026-09-08

**Status: native AMD64 container and disposable Kubernetes CI verification
passed; no production rollout performed.** The work is on `porting-go`, based
on `d2e5edf`. The final verified revision is `011b0cd`; its application
implementation is unchanged from the operational fixes in `8e3a436`.

## Findings corrected and reviewed

- `/healthz` now returns HTTP 200 with `{"status":"ok"}` without contacting the
  database. It no longer risks restart loops during quorum loss.
- `/readyz` retains database diagnostics, uses one bounded evaluation, and
  requires the configured read consistency. Strong mode fails closed without a
  linearizable read. Local and prefer-strong modes retain their fallback rules.
  The checks take at most approximately two seconds; Kubernetes readiness allows
  three seconds.
- The public Grant Service exposes only HTTP. Native UDP peer communication
  remains on the headless Service; obsolete Hiqlite ports are removed.

Independent reviewers confirmed the health API, consistency/deadline behavior,
public/private Service boundary, and the rendered-overlay verification checks.
CI regressions were also corrected: a stray script-list entry, an unsupported
fixture provider name (now `filesystem`), and stale readiness assertions against
`/healthz`. Regression checks retain separate process-health and database-ready
requirements, including rejection when quorum is unavailable.

## Executed verification

The following passed after the operational fixes:

- `go test -race -count=1 ./...` and `go vet ./...`.
- `scripts/verify-go-cli-contract.sh`, `scripts/verify-docs.sh`, and
  `scripts/verify-rhiza-deployment.sh`.
- `git diff --check` and focused independent liveness/readiness tests.

[CI run 34210691940](https://github.com/Conalog/grant/actions/runs/34210691940)
at `011b0cd` completed successfully: both `go-verification` and `local-bootstrap`
passed; publication jobs were skipped. Native AMD64 container verification
covered startup, strong readiness, authenticated write/read and unauthenticated
rejection under UID 10001, a read-only root filesystem, dropped capabilities,
512 MiB memory and one CPU. The smoke rejects emulated Docker architectures.
Its local Docker image ID was
`sha256:80d0e0b2224637dfbdd67835bae2a001530b19b1e902535a4182858b4dfe6b4f`;
this is verification provenance, not a published registry digest.

The disposable Kubernetes run successfully rolled out three Grant and three
Rauthy replicas and passed runtime, readiness, authentication-proxy and Rauthy
pod-replacement checks.
These are CI-cluster results, not a deployment to IED or production.

The local `scripts/verify-go-three-voter.sh` run **`v2d405`** exited successfully.
It used native Go processes and disposable MinIO and verified:

1. Shared-S3 three-voter writes and strong reads, two-voter quorum survival, and
   same-directory voter recovery.
2. Abrupt loss of one voter followed by recovery into a fresh local directory.
3. HTTP 200 liveness and HTTP 503 strong readiness after quorum loss.
4. Abrupt loss of all voters, followed by recovery into three fresh directories
   from the unchanged shared S3 prefix, including the latest committed marker.
5. Populated snapshot restoration into a separate fresh three-voter cluster,
   closed readiness before all voters opt in, rejection of conflicting target
   admission, and restored-cluster quorum recovery.

These results establish local and CI behavior. They do not verify the target cluster's
CNI, Kubernetes probes, production S3 credentials/policies, resource limits,
authentication ingress, or traffic cutover.

## Read-only environment assessment

IED was reachable with four Ready **amd64** nodes and an available default
`standard-rwo` storage class. Resource-creation permission checks succeeded.
The previously recorded isolated target namespace was absent. A bounded
read-only scan found no Grant workload on tg900. Neither observation authorizes
changing an existing deployment. No Secret values were inspected.

## Remaining deployment gates

An exact target has not been selected, and no live rollout has occurred. Before
claiming deployment verification complete:

1. Confirm the cluster and isolated namespace, preserving existing workloads.
2. Publish an image from the exact verified revision and pin its immutable
   registry digest.
3. Apply a disposable instance of the production overlay with separate storage
   identity, then verify scheduling, probes, three-voter membership and quorum.
4. Verify real authentication, negative authorization, TLS and ingress behavior.
5. Exercise recovery with the target object store and credentials, confirm
   rollback/cutover boundaries, and only then consider traffic changes.
