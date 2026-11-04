# Standalone Local Kubernetes

Current source selects Rhiza v0.21.2 Local storage. The standalone overlay runs
one Grant process and targets the v0.4.1 release candidate.
It contains no Operator or HA membership. Do not combine it with the
HA base or Operator overlay. Grant's `voter` role selects its writable API;
Local mode has one stable identity and no peer replication.

## Image prerequisite

Both containers target `ghcr.io/mrchypark/grant:v0.4.1` with `IfNotPresent`.
This forthcoming patch release selects Rhiza v0.21.2; v0.4.0 contains v0.21.1.
Verify publication and the image source revision before using the new tag.
For isolated testing, build the candidate and explicitly override both images in
a separate local customization before importing it into the selected cluster:

```sh
docker build --platform linux/amd64 -t grant:standalone-v0212-local .
# For an explicitly selected kind cluster:
kind load docker-image grant:standalone-v0212-local --name YOUR_CLUSTER
```

For another runtime, use its supported local-image import mechanism on every
eligible node. Rebuild and reimport after source changes; `IfNotPresent` can reuse
a stale tag. The existing Dockerfile includes `cmd/grant` automatically.
The published `ghcr.io/mrchypark/grant:v0.3.0` image embeds Rhiza v0.21.0 and
has no `restore-local-storage` command; it cannot run this overlay.
Release publication follows review approval and successful candidate verification.

## Shared configuration and storage

The ConfigMap supplies `GRANT_RUNTIME_MODE=embedded`,
`GRANT_LOCAL_NODE_ROLE=voter`, `GRANT_DATA_DIR=/var/lib/grant/rhiza`, stable
NodeID/ClusterID, empty peer/listener settings, `GRANT_CLUSTER_MEMBERS=[]`, and:

```json
{"Local":true,"NodeID":"grant-standalone","ClusterID":"grant-standalone-v1","Members":[],"PeerAddr":""}
```

There is no `RHIZA_LOCAL` alias. Both containers receive exactly the same
`envFrom` entries in the same order and mount the same retained data PVC at
`/var/lib/grant`. ConfigMap values are last, overriding same-name secret values.
Object-store environment fields merge with the JSON configuration; both opens
use explicit `RHIZA_OBJSTORE_DURABILITY=before-ack` and require object-store access.

In a dedicated namespace, independently provision restricted secrets:

- `grant-secrets`: distinct `GRANT_MASTER_KEY` and `GRANT_APP_AUTH_KEY`, the
  selected external authentication provider and its real required settings.
- `grant-rhiza-storage`: a supported independent external object-store provider,
  bucket, dedicated prefix, endpoint and credentials or workload identity.
  Never use a second directory on the data PVC as the independent archive.

Select the persistent disk StorageClass and capacity through a separate
customization; the default is a retained 10Gi ReadWriteOnce claim. Configure
service account/workload identity explicitly if required; token automount is off.
Keep the same ClusterID, NodeID, bucket, prefix and DataDir across init and main.
The data volume remains a PVC; only `/tmp` uses memory-backed `emptyDir`.

## Init and startup

The single init container runs `/usr/local/bin/grant restore-local-storage`.
This application-owned command uses the public `rhiza.Open` followed by `Close`
with the same effective configuration and DataDir as main. It starts no Grant
SQL, schema initialization, auth bootstrap or services. An Open or Close error
exits nonzero and prevents main startup. Rhiza supplies no official init CLI or
image; Grant owns both entrypoint and image.

The main `/usr/local/bin/grant` opens the same store, validates its binding and
reconciles history. Equal histories do not blindly download again. Keep archive
configuration active in main, including the selected durability. This retained-PVC
overlay still selects before-ack. The v0.4.1 init command requires before-ack;
the v0.7.0 source candidate additionally accepts explicit async for profiles that
select it, with possible loss of acknowledged but unarchived writes after data loss.
Missing or unsupported durability still fails before storage opens. Init success is
not readiness: main must finish storage and service initialization, and `/readyz`
must pass. Startup/liveness use `/healthz`. SIGTERM reaches Grant directly; the
60-second grace period includes its 30-second HTTP drain and store close, without
a measured guarantee for every storage latency.

This follows the [upstream init guide](https://github.com/mrchypark/rhiza/blob/v0.21.2/docs/local-init-container-recovery.md)
(source commit `8acb6da35a4c6fc83fa14ecec2fe2b64370fa4bd`), using
the public Open/Close contract. The sample's optional disposable data volume does
not change this overlay's retained-PVC design.

For a host process, export the same complete configuration once, with appropriate
writable data/backup paths, then run sequentially without changing DataDir:

```sh
/usr/local/bin/grant restore-local-storage && /usr/local/bin/grant
```

## Recovery and controlled rollout

Before either recovery or replacement, prove the previous writer has stopped,
cannot be recreated and has no outstanding storage work. One replica,
ReadWriteOnce and `OnDelete` do not fence a partitioned writer. Never force-delete
a pod or clear an uncertain maintenance token based on age. Preserve data,
archives and errors if startup fails. Do not migrate HA into Local mode.

v0.21.2 uses the same archive format as v0.21.0, with no format migration between
them. Older incompatible archives still fail closed without automatic migration.
For an older installation, preserve its data/archive and use a separately
validated migration plan; never delete it to bypass rejection.

These commands render and verify locally without applying to a cluster:

```sh
kubectl kustomize deploy/overlays/standalone
scripts/verify-standalone-deployment.sh
scripts/verify-rhiza-deployment.sh
```

A future authorized rollout must explicitly select context/namespace, verify
the selected image's version and confirm writer exclusion before starting
the replacement. `OnDelete` retains existing pods after template changes; retain
the PVC and archive during the controlled replacement. Replacement causes downtime.
Namespace deletion and storage reclaim policy require separate protection.

## Verification limits

The standalone verifier renders manifests and checks the actual init command,
release image/pull policy, equal environment/mounts, Local configuration, selectors,
retained PVC and manual rollout. A temporary Go overlay exercises actual
`ConfigFromEnv` and effective Rhiza configuration without opening storage or
writing tests into the concurrently owned application package. The core worker's
command tests establish the Open/Close and no-service contract separately.

`scripts/verify-standalone-container.sh` exercises the candidate image with owned
data volumes and a separate filesystem archive: init exits successfully, main
writes and reads an API marker, a graceful close permits same-directory reuse,
and a fresh data volume restores the marker from the same archive. The original
data volume survives until verification cleanup. This models clean data loss,
not a crash or a Chaos Mesh experiment. Linux CI runs this under the existing
container security/resource limits and preserves logs.

No cluster apply, volume attachment, fencing or real external
auth/cloud integration is claimed. The filesystem recovery fixture is not an
all-provider pass. No adapters changed. The historical S3 failure reported for
[v0.21.1](https://github.com/mrchypark/rhiza/releases/tag/v0.21.1) remains evidence;
upgrading alone does not prove that failure permanently resolved in Grant.
The [v0.21.2 release](https://github.com/mrchypark/rhiza/releases/tag/v0.21.2)
addresses Local maintenance release reconciliation only when stable remote readback
proves the exact expected free control at a changed object version. Work errors,
cancellation and uncertain operations remain fail-closed; the upload timeout is
unchanged. This cannot clear an already orphaned held remote token and changes no
archive format or public API. Upstream reports CI run 40838063324 passing all three
pinned emulator SDK adapter contracts, each with 19 passes, 0 failures and 2 skips.
Those upstream results do not qualify Grant's cloud integrations or establish
S3/GCS/Azure production parity. The `e2e-local` CI workflow runs the full race
suite and native AMD64 container tests under production security/resource limits.
Verify successful checks for the release's source revision before rollout;
a local cross-build alone does not establish native image runtime qualification.
