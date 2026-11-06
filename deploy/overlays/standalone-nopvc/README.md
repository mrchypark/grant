# Experimental controlled Local DR without PVC

See [the NoPVC runbook](../../../docs/single-node-nopvc.md). This opt-in overlay
inherits standalone init/main and security, and overrides both images to the
v0.9.1 release contract (Rhiza v0.23.0). Verify image publication and source
revision before deployment. The final override applies to both init and main;
the inherited standalone image pin is not used by this overlay.
The older v0.4.1 restore command rejects async; it cannot run this async profile.
This is a single-node `Local: true`, S3-only example with no Operator or PVC,
not a generic example for every object-store provider or a qualified production rollout.
Never apply over an existing retained-PVC StatefulSet; use a separately approved
namespace and migration plan. Do not combine both overlays in one namespace.

Remote S3 archive configuration is mandatory. The ConfigMap overrides secret
provider/directory values with s3 and an empty filesystem directory. It removes
the inherited durability key so grant-rhiza-storage can explicitly select
RHIZA_OBJSTORE_DURABILITY=async (the requested priority) or before-ack. Both modes
are supported by the Rhiza v0.23.0 integration and Grant restore command;
before-ack is optional. Supply the other archive fields
using grant-rhiza-storage; no secret
or bucket is provisioned here. A filesystem archive is not supported by this
profile. Configuration and physical archive independence require deployment review.

Async acknowledges writes before remote archival completes. If local data is lost,
acknowledged writes not yet archived may be lost; this profile provides no fixed
loss-window bound or zero-data-loss guarantee. Changing durability does not fence
another writer or resolve uncertain archive ownership.

Indefinite not-ready/unreachable NoExecute tolerations suppress the usual timed
node-condition eviction. Recovery deliberately requires manual intervention and
proven writer exclusion. These tolerations are not fencing or protection against
node deletion, force deletion, other eviction controllers or duplicate deployments.

Run scripts/verify-standalone-nopvc-deployment.sh from the repository root.
[CI run 42138972875](https://github.com/mrchypark/grant/actions/runs/42138972875)
completed with overall SUCCESS for runtime source
`8c3dd5d9ff252b033452ea107ec91c0e872ae929`, including render/configuration
checks and Local clean-close recovery source tests. See the runbook for the
qualification boundaries.
Rendering and configuration tests do not establish Kubernetes recovery or remote
archive durability. This profile is experimental controlled manual DR, not
unattended HA; live Pod replacement, operational fencing and production-cloud
recovery remain unqualified.
