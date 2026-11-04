# Experimental single-node Local DR without PVC

This opt-in profile uses one embedded Local writer with a stable NodeID and
ClusterID, no peers and no Operator. Local data and local logical backup exports
live on disk-backed emptyDir. An independent remote S3 archive is required with
explicitly selected durability; async is the requested priority, not zero data loss.
Filesystem archives, another directory on the same
emptyDir, and an object store backed only by the same node are not independent DR.
This is controlled manual recovery with downtime. Production use remains blocked
until the chosen image, remote archive and operational fencing procedure qualify.

## Complete deployment configuration

Render deploy/overlays/standalone-nopvc separately in an approved namespace. Do not
apply it over an existing PVC StatefulSet or combine it with standalone/HA overlays.
The old PVC overlay is unchanged. Both init and main retain v0.4.1 (Rhiza v0.21.2),
IfNotPresent, the same envFrom order, mounts, UID/GID 10001, read-only root filesystem,
dropped capabilities and disabled service-account token automount. Source tests
against Rhiza v0.22.1 do not qualify that published image. Override both images only
in an independently reviewed customization with a verified immutable image.

Provision grant-secrets with distinct GRANT_MASTER_KEY and GRANT_APP_AUTH_KEY,
GRANT_AUTH_PROVIDER and the complete selected identity-provider configuration.
For Rauthy this includes GRANT_RAUTHY_BASE_URL, GRANT_RAUTHY_CLIENT_ID and
GRANT_RAUTHY_CLIENT_SECRET; provision any additional API credentials required by
selected application operations. Never use fixture credentials for deployment.

Provision grant-rhiza-storage with RHIZA_OBJSTORE_BUCKET, a dedicated nonempty
RHIZA_OBJSTORE_PREFIX, RHIZA_OBJSTORE_ENDPOINT, RHIZA_OBJSTORE_REGION,
RHIZA_OBJSTORE_ACCESS_KEY and RHIZA_OBJSTORE_SECRET_KEY (and RHIZA_OBJSTORE_SESSION_TOKEN when
required). Use restricted bucket credentials, TLS and appropriate retention policy.
RHIZA_OBJSTORE_INSECURE must be false. Use an endpoint outside the Pod/node failure
domain; credentials and endpoint reachability need separate validation. The profile
forces RHIZA_OBJSTORE_PROVIDER=s3, clears RHIZA_OBJSTORE_DIR and disables insecure mode
through the last ConfigMap envFrom entry. It removes the inherited durability key.
Set RHIZA_OBJSTORE_DURABILITY=async in grant-rhiza-storage for asynchronous archival;
before-ack is a supported optional alternative, not a profile requirement. The verifier
requires one of these two explicit values and checks that it reaches the effective
Rhiza configuration unchanged. See the [v0.21.2 constants](https://github.com/mrchypark/rhiza/blob/v0.21.2/internal/types/config.go).
Do not add env overrides or mount a local
archive. Init refuses missing object-store configuration, but its success alone is
not proof that the expected nonempty archive was recovered.

Keep NodeID, ClusterID, bucket, prefix and DataDir fixed across init/main and recovery.
Changing ClusterID can select an empty namespace and succeed. Restore validation
must check expected business authority, ownership identity/revision and allow/deny
behavior. Local exports in /var/lib/grant/backups disappear with the Pod; copy any
required exports off-Pod. The native archive is separate from logical exports.

With async durability, acknowledgment does not wait for remote archival. Loss of
emptyDir can therefore lose acknowledged writes that have not reached the archive,
including ownership and authorization changes. This profile establishes no fixed
time or write-count bound for that loss window and makes no RPO-zero claim.
Replication intervals are not a guaranteed recovery-point bound. Before-ack is
available when explicitly selected, but does not establish external writer fencing
or qualify recovery. Init success does not prove that the latest acknowledged
authority was recovered; compare it against independently retained evidence.

## Writer exclusion and node failure

The StatefulSet uses replicas=1 and OnDelete, with indefinite NoExecute tolerations
for node.kubernetes.io/not-ready and node.kubernetes.io/unreachable. No
`tolerationSeconds` is set: the ordinary node-condition eviction timeout cannot
automatically replace a partitioned writer. This deliberately sacrifices automatic
recovery and can leave the service unavailable until an operator intervenes.

This guard is not physical fencing. Node deletion, force deletion, other controllers,
manual duplicates, disk-pressure eviction, preemption and infrastructure actions can bypass it. Before deleting a Pod,
starting a replacement or using the archive elsewhere, establish and record external
proof that the old process stopped, cannot resume/restart and has no outstanding
storage operations. For an unreachable node, isolate or power off the old node with
verified infrastructure controls before replacement. A Kubernetes deletion response,
a Lease, replicas=1 and the archive maintenance token do not establish that proof.
Never clear uncertain remote maintenance tokens based on age. If exclusion cannot
be proven, keep replacement blocked and preserve the archive and diagnostic errors.
No automatic failover or fencing implementation is claimed.

A container restart in the same Pod retains emptyDir and usually reopens its local
data; init containers do not normally rerun on an application-container restart.
Pod deletion/replacement creates a fresh emptyDir, reruns init, and requires recovery
from the remote archive. Node loss can also destroy local data. An init Open/Close
success does not make main ready: main must initialize and /readyz must pass.

For a planned replacement, stop incoming writes, permit clean shutdown (the inherited
60-second grace includes drain and storage close but is not a measured universal
bound), verify termination/exclusion, then perform the approved replacement. OnDelete
prevents template updates from triggering rolling replacement; it does not prevent
the StatefulSet controller recreating a deleted Pod. A clean-close recovery result
must not be reported as proof of abrupt termination or partition recovery.

## Verification and approval boundary

Run from the repository root:

```sh
scripts/verify-standalone-deployment.sh
scripts/verify-standalone-nopvc-deployment.sh
```

The new verifier checks the rendered NoPVC contract, indefinite node-condition
tolerations and real ConfigFromEnv/effective Rhiza configuration using synthetic
remote S3 settings, without opening storage or contacting a cloud endpoint. It does
not validate deployment secrets, remote durability, fencing or Kubernetes execution.

Before approval, run the exact image on native AMD64 in an isolated Kubernetes
cluster with disposable remote-S3 test storage. Write authority data, cleanly stop
and exclude the old writer, replace the Pod, confirm the new Pod UID/fresh local
volume and verify recovered ownership/authorization plus new writes. Separately
check that node-condition eviction does not start an unfenced replacement. Record
image digest, version, cluster, archive endpoint/failure domain, cleanup and results.
Native Kubernetes runtime coverage is not established by these validators. Render-only or ARM64
source-test success is not native image/Kubernetes qualification. S3 testing alone
also does not establish GCS/Azure adapter coverage or production-cloud parity.
No deployment, publication, commit or release is authorized by this document.

To validate reviewed deployment environment values without opening storage, provide
`STORAGE_ENV_FILE=/absolute/path/combined-env.json` to the NoPVC verifier. The file
must contain a flat JSON object of string environment values from both secrets
(auth and S3); rendered ConfigMap values are applied last, as in the Pod. The
verifier copies this sensitive file only into its temporary directory and removes
that copy on exit; it does not print values. Keep the original file access-restricted.
Missing or unsupported durability, missing bucket/prefix/endpoint/region/credentials, insecure HTTP, endpoint paths,
filesystem endpoints and local/loopback endpoints fail this preflight. Without the
file, deployment-secret validation is explicitly skipped. This additional policy
belongs to the verifier: ConfigFromEnv alone does not enforce every remote-S3
requirement. Run preflight before deployment and block deployment on failure.
A remote-looking hostname can still resolve to the same node or local storage;
DNS, failure-domain independence, credentials and reachability require external
review and runtime verification. This is not a runtime admission or fencing guard.

## Validation results — 2026-11-02

A source candidate using Rhiza v0.22.1 passed host-process recovery on Darwin ARM64
with CGO_ENABLED=0 through the actual provider adapters/SDKs: S3 with Floci 2.3.0,
GCS with Floci-GCP 0.11.0 and Azure Blob with Floci-AZ 0.15.0. Each isolated run
used the real restore-local-storage CLI and main HTTP server, acknowledged a
provider write, completed an orderly restart, removed the entire DataDir, restored
from the independent emulator archive, and verified the recovered provider list
and SQL data. Role/action bindings, transferred ownership with the exact saved
ETag, and allowed/denied authorization decisions were preserved across recovery.

These were loopback emulator tests with fixture credentials and host directories,
not an exact deployment match. They do not qualify remote TLS, independent cloud
failure domains, the published v0.4.1 image, native AMD64 Kubernetes Pod replacement,
eviction behavior, abrupt crashes or production-cloud parity. This deployment
profile remains S3-only despite the broader provider-adapter recovery coverage.

A separate S3 run started a second writer with the same identity/archive and a
different DataDir while the first was live. The second became ready and returned
HTTP 201; the first subsequently reported Local HEAD advanced on close. This
demonstrates no external fence, not an upstream defect under the documented
single-writer precondition; it does not establish loss of acknowledged data.
Bounded S3 checks failed closed for a wrong NodeID and an unavailable endpoint.
Changing ClusterID succeeded in a separate fresh namespace, reinforcing that init
success alone does not prove recovery of the intended authority. These negative
and competing-writer checks do not establish equivalent behavior for GCS or Azure.
