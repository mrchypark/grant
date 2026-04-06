# Chaos Mesh on `ied`

This directory contains `Chaos Mesh` installation guidance and experiment examples
for the `certd-chaos` namespace on the `gke_ied-cluster` cluster.

It is intentionally conservative:

- Examples only cover experiments that can be represented honestly with static YAML.
- `leader` versus `follower` voter targeting is not hidden behind a misleading selector.
- `learner -> voter` `NetworkChaos` has been validated on `ied` for the current
  learner semantics: local reads remain available, `?consistency=strong` fails, and
  learner write endpoints stay rejected.

## Scope

These manifests target the current deployment labels:

- `app=certd`, `role=voter` on `certd-voter` `StatefulSet`
- `app=certd`, `role=learner`, `instance=learner-1|2|3` on learner `Deployment`s
- `app=trailbase` on the `trailbase` `Deployment`

## Install Chaos Mesh on GKE

Chaos Mesh should be installed once by a cluster admin into a separate namespace such
as `chaos-mesh`. The project examples below stay in `certd-chaos`.

Official installation reference:

- [Install Chaos Mesh using Helm](https://chaos-mesh.org/docs/production-installation-using-helm/)

`ied` is a `containerd` cluster, so use the `containerd` runtime settings:

```bash
helm repo add chaos-mesh https://charts.chaos-mesh.org
helm repo update

kubectl --context gke_ied-cluster create namespace chaos-mesh

helm upgrade --install chaos-mesh chaos-mesh/chaos-mesh \
  --namespace chaos-mesh \
  --kube-context gke_ied-cluster \
  --set chaosDaemon.runtime=containerd \
  --set chaosDaemon.socketPath=/run/containerd/containerd.sock
```

Verify installation:

```bash
kubectl --context gke_ied-cluster get pods -n chaos-mesh \
  -l app.kubernetes.io/instance=chaos-mesh
```

## Optional experiment runner RBAC

The controller and CRDs are cluster-level Chaos Mesh components. This repo does not
try to install them. The RBAC in
[rbac/experiment-runner-rbac.yaml](rbac/experiment-runner-rbac.yaml)
is only for a namespace-scoped service account that creates and manages experiment
objects in `certd-chaos`.

Apply it if you want a dedicated service account for scripted experiments:

```bash
kubectl --context gke_ied-cluster apply -f \
  deploy/chaos-mesh/rbac/experiment-runner-rbac.yaml
```

## Supported static examples

These manifests are directly usable after Chaos Mesh is installed:

- learner restart:
  [pod/learner-1-restart.yaml](pod/learner-1-restart.yaml)
- TrailBase outage:
  [pod/trailbase-down.yaml](pod/trailbase-down.yaml)
- learner-1 to all voters partition:
  [network/learner-1-to-voters-partition.yaml](network/learner-1-to-voters-partition.yaml)
- a safe serial workflow that avoids dynamic leader/follower targeting:
  [workflow/gke-safe-sequence.yaml](workflow/gke-safe-sequence.yaml)

## Voter delete: do not guess leader or follower

Static label selectors cannot identify the current hiqlite leader. For voter deletion,
use a pod-list selector and set the pod name after checking cluster state.

Templates:

- follower candidate:
  [pod/follower-delete-template.yaml](pod/follower-delete-template.yaml)
- leader target:
  [pod/leader-delete-template.yaml](pod/leader-delete-template.yaml)

Leader/follower selection procedure:

- [guides/leader-delete.md](guides/leader-delete.md)

## Recommended usage

1. Start with `learner-1-restart.yaml` and `trailbase-down.yaml`.
2. For automated regression, do not judge partition via a `Service` port-forward.
   Probe inside the learner pod instead. `kubectl port-forward svc/certd-learner-1`
   can hang under partition even while `localhost:8080/healthz` inside the pod still
   reports `readReady=true` and `writeReady=false`.
3. For voter deletion, identify the actual pod first and then apply the matching
   template.
4. Keep `leader-delete` outside a static workflow until you have a reliable leader
   discovery step that patches the target pod name at runtime.

## NetworkChaos validation notes

Chaos Mesh supports `selector`, `target`, and `direction` for network partition, so it
can model `learner -> voter` egress loss accurately in principle.

On `ied`, the validated behavior is:

- learner pod `/healthz` degrades to `readReady=true`, `writeReady=false`
- learner default local reads keep returning `200`
- learner reads with `?consistency=strong` return `500` with
  `strong consistency requested but quorum-backed identity sync is unavailable`

The reliable probe path is learner pod local loopback, not service port-forward.

## References

- [Install Chaos Mesh using Helm](https://chaos-mesh.org/docs/production-installation-using-helm/)
- [Define the Scope of Chaos Experiments](https://chaos-mesh.org/docs/2.6.7/define-chaos-experiment-scope/)
- [Simulate Pod Faults](https://chaos-mesh.org/docs/2.7.2/simulate-pod-chaos-on-kubernetes/)
- [Simulate Network Faults](https://chaos-mesh.org/docs/next/simulate-network-chaos-on-kubernetes/)
- [Create Chaos Mesh Workflow](https://chaos-mesh.org/docs/create-chaos-mesh-workflow/)
- [Securing tenant namespaces using restrict authorization feature in Chaos Mesh](https://chaos-mesh.org/blog/securing-tenant-namespaces-using-restrict-authorization-feature/)
