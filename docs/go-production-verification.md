# Go deployment verification — 2026-09-08

**Status: local verification passed; no production rollout performed.** The work
is on `porting-go`, based on `d2e5edf`. The operational fixes described below are
verified in the working tree; their final source commit is pending.

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

## Executed verification

The following passed after the operational fixes:

- `go test -race -count=1 ./...` and `go vet ./...`.
- `scripts/verify-go-cli-contract.sh`, `scripts/verify-docs.sh`, and
  `scripts/verify-rhiza-deployment.sh`.
- `git diff --check` and focused independent liveness/readiness tests.

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

These results establish local behavior. They do not verify the target cluster's
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
2. Commit the verified fixes and publish an **amd64** image from that exact
   revision; pin its immutable registry digest.
3. Apply a disposable instance of the production overlay with separate storage
   identity, then verify scheduling, probes, three-voter membership and quorum.
4. Verify real authentication, negative authorization, TLS and ingress behavior.
5. Exercise recovery with the target object store and credentials, confirm
   rollback/cutover boundaries, and only then consider traffic changes.

The earlier local Docker build and startup smoke used Linux/arm64 and predates
these operational fixes. It is not evidence that the final amd64 production
image has been built or deployed.
