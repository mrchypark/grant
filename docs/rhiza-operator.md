# Embedded Rhiza Operator

`deploy/overlays/rhiza-operator` connects Grant's embedded Rhiza v0.20.0 runtime to the upstream namespace-scoped recovery Operator. It is an optional fresh-cluster deployment contract, not a live deployment command and not an in-place upgrade for an existing v0.12 store.

One Grant instance in native `Local` mode cannot use this recovery Operator.
Upstream v0.20.0 enforces `voterCount >= 3` in both the
[CRD](https://github.com/mrchypark/rhiza/blob/v0.20.0/deploy/operator/cluster-crd.yaml)
and the controller's adoption and bootstrap validation in
[`automatic.go`](https://github.com/mrchypark/rhiza/blob/v0.20.0/pkg/operator/automatic.go).
The [supervised fencing startup gate](https://github.com/mrchypark/rhiza/blob/v0.20.0/pkg/node/fencing_session.go)
also rejects `Local` mode and membership counts below two; a supervisor is not a
single-node Operator alternative. Three voter Pods on one Kubernetes host are
still three Grant instances. For the supported one-instance archive recovery path,
see [the Local recovery verification](backup-restore.md#single-instance-local-archive-recovery).

The optional controller image remains pinned to v0.19.0. Its compatibility with
the v0.20.0 embedded runtime is **unverified**; this module upgrade does not qualify
that controller/runtime combination or upgrade the deployed controller.

The rendered default namespace is `grant`. For an IED composition, transform the whole overlay to the target namespace and provision the same secrets there; do not combine it with a broad recovery-port policy. The shared `grant-lifecycle-state` ConfigMap remains Rauthy-only lifecycle input and is not read by this controller.

The three vendored CRDs in the overlay are cluster-scoped platform prerequisites even though the controller RBAC and its custom resources are namespace-scoped. A platform administrator must review and install those CRDs before a namespace owner uses the rendered controller resources. This task does not apply either scope to a cluster.

## Prerequisites

Build the Grant image with the Go integration that opens Rhiza from its native environment and starts `Database.OperatorBindAddr` on `RHIZA_BIND_ADDR`. The handler must expose only Rhiza's recovery and membership endpoints before the strong-read wait; do not route it through the public Grant Service or Ingress.

A native learner is deliberately not a passive Grant replica. Before its quorum-certified promotion, it returns HTTP 200 only for `/healthz`; business routes return 503. Promotion requires a certified quorum and strong read, then runs `Migrate` and `Bootstrap` before the normal voter handler is installed. This prevents a learner from serving Grant business traffic from an unpromoted or locally stale state.

Provision these namespace-local secrets outside Git before rendering for use. Object-store keys use the full `RHIZA_OBJSTORE_*` prefix. Their exact names are part of the Operator rewrite contract:

The object-store key list below describes S3. Other providers use their native `RHIZA_OBJSTORE_*` settings and credentials; the S3 access-key fields are not universal requirements.

| Secret | Required keys | Notes |
| --- | --- | --- |
| `grant-rhiza-operator-business` | selected Grant business/authentication `GRANT_*` keys | Copy only required business keys from `grant-secrets`; omit every rejected native-mode alias. |
| `grant-rhiza-operator-object-store` | `RHIZA_OBJSTORE_PROVIDER`, `RHIZA_OBJSTORE_ENDPOINT`, `RHIZA_OBJSTORE_BUCKET`, `RHIZA_OBJSTORE_PREFIX`, `RHIZA_OBJSTORE_REGION`, `RHIZA_OBJSTORE_ACCESS_KEY`, `RHIZA_OBJSTORE_SECRET_KEY` | Use a versioned shared object store and a fresh, unused prefix. It replaces `grant-rhiza-storage`, which contains the rejected `GRANT_CLUSTER_MEMBERS` alias. |
| `grant-rhiza-operator-credentials` | `members`, `peer_tokens`, `admin` | Create as immutable. `members` is the three-member native JSON array, `peer_tokens` is the keyed node-token JSON map, and `admin` is a distinct token. All tokens must be at least 32 bytes and distinct from peer and business secrets. |

`grant-rhiza-config` supplies the initial `RHIZA_CLUSTER_ID=grant-rhiza-v019`. Replace that value, the observation resources' `sourceClusterID` and `adoptClusterID`, the native members, and the object-store prefix together before first use. The value must name a new logical Rhiza cluster; it must not reuse the legacy v0.12 cluster ID or storage prefix.

The initial credential Secret remains immutable. During a reviewed recovery, the controller creates a different immutable generation Secret, then rewrites the `StatefulSet`'s explicit `RHIZA_CLUSTER_MEMBERS`, `RHIZA_PEER_TOKENS`, and `RHIZA_ADMIN_TOKEN` references to it. Do not rename those environment variables or replace them with shell expansion.

The overlay leaves the existing private runtime transport disabled. If an embedded voter needs TCP 8200, add the `private-runtime` transport patch, private Service and `grant-runtime-client: "true"` policy rule to this composition; do not include the same base twice. Configure `GRANT_RUNTIME_BIND_ADDR` on that private port and keep `GRANT_REMOTE_API_SECRET` and `GRANT_READ_FENCE_KEY` distinct from Rhiza admin and peer credentials. A native learner returns 503 from that transport until quorum-certified promotion; Patch Data's passive learner behavior is separate.

## Observation and manual recovery

The included `RhizaRecovery/grant-rhiza-observe` has an empty `recoveryID` and can only observe peers. `RhizaCluster/grant-rhiza` sets `automatic: false`; the Deployment does not set automatic recovery or a Kubernetes fencer.

For a manual recovery, a trusted incident authority creates a separate request only after it has fenced every old voter, client, archive writer, and storage writer outside Kubernetes. The reviewed shape is:

```yaml
apiVersion: rhiza.mrchypark.dev/v1alpha1
kind: RhizaRecovery
metadata:
  name: grant-rhiza-manual-REPLACE
spec:
  statefulSet: grant-voter
  container: grant
  sourceClusterID: REPLACE_FRESH_SOURCE_CLUSTER_ID
  durability: before-ack
  recoveryID: REPLACE_UNIQUE_RECOVERY_ID
  allowDataLoss: false
  fence:
    recoveryID: REPLACE_UNIQUE_RECOVERY_ID
    clusterID: REPLACE_FRESH_SOURCE_CLUSTER_ID
    statefulSetUID: REPLACE_ACTUAL_STATEFULSET_UID
    confirmed: false
    evidence: REPLACE_EXTERNAL_FENCING_EVIDENCE
```

This intentionally remains blocked until a separately authorized fence authority supplies a truthful confirmation and evidence. Pod disappearance, a Kubernetes Ready condition, or a NetworkPolicy is not fencing. Enable automatic recovery only after a trusted external fencer, a runtime/storage executor, and a reviewed recovery exercise exist; do not enable the Kubernetes fencer for this deployment. Recovery Anchor is also disabled: Grant does not implement the required `CheckBinding` and `UpdateApplication` write guards.

The NetworkPolicy preserves the base public TCP 8080 Service/Ingress behavior, grants TCP 8101 only to the operator, and grants UDP 8100 between voters and upstream-created automatic learners (`rhiza.mrchypark.dev/automatic-learner`). That learner label is the exact upstream Pod label; it is not `app=grant,role=voter`. A manually provisioned learner needs an equivalent reviewed policy before it joins. The policy is additive with `private-runtime`; that overlay does not expose TCP 8101.

Render-only verification is available with `sh scripts/verify-rhiza-deployment.sh`. It does not contact a cluster or read secrets.
