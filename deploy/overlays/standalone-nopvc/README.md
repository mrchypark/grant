# Experimental controlled Local DR without PVC

See [the NoPVC runbook](../../../docs/single-node-nopvc.md). This opt-in overlay
inherits standalone init/main and security, and overrides both images to the
reserved v0.7.0 release from this PR (Rhiza v0.21.2). This tag is not yet published:
do not apply until this exact reviewed source is released as v0.7.0 and its image
revision/publication is verified. v0.6.2 is reserved for the separate helper PR.
The older v0.4.1 restore command rejects async; it cannot run this async profile.
This profile does not qualify the v0.22.1 source candidate or a production rollout.
Never apply over an existing retained-PVC StatefulSet; use a separately approved
namespace and migration plan. Do not combine both overlays in one namespace.

Remote S3 archive configuration is mandatory. The ConfigMap overrides secret
provider/directory values with s3 and an empty filesystem directory. It removes
the inherited durability key so grant-rhiza-storage can explicitly select
RHIZA_OBJSTORE_DURABILITY=async (the requested priority) or before-ack. Both modes
are supported by Rhiza v0.21.2 and this PR's updated Grant restore command;
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
Rendering and configuration tests do not establish Kubernetes recovery or remote
archive durability. This profile is experimental controlled manual DR, not
unattended HA; production remains blocked pending runtime and fencing qualification.
