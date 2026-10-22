# Chaos Mesh on `ied`

This directory contains `Chaos Mesh` installation guidance and experiment examples
for the `grant-chaos` namespace on the `gke_ied-cluster` cluster.

It is intentionally conservative:

- Examples only cover experiments that can be represented honestly with static YAML.
- `leader` versus `follower` voter targeting is not hidden behind a misleading selector.

## Scope

These manifests target the current deployment labels:

- `app=grant`, `role=voter` on `grant-voter` `StatefulSet`
- `app=rauthy` on the `rauthy` `StatefulSet`

## Install Chaos Mesh on GKE

Chaos Mesh should be installed once by a cluster admin into a separate namespace such
as `chaos-mesh`. The project examples below stay in `grant-chaos`.

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
objects in `grant-chaos`.

Apply it if you want a dedicated service account for scripted experiments:

```bash
kubectl --context gke_ied-cluster apply -f \
  deploy/chaos-mesh/rbac/experiment-runner-rbac.yaml
```

## Supported static examples

These manifests are directly usable after Chaos Mesh is installed:

- Rauthy outage:
  [pod/rauthy-down.yaml](pod/rauthy-down.yaml)

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

1. Start with `rauthy-down.yaml`.
2. For voter deletion, identify the actual pod first and then apply the matching
   template.
3. Keep `leader-delete` outside a static workflow until you have a reliable leader
   discovery step that patches the target pod name at runtime.

## References

- [Install Chaos Mesh using Helm](https://chaos-mesh.org/docs/production-installation-using-helm/)
- [Define the Scope of Chaos Experiments](https://chaos-mesh.org/docs/2.6.7/define-chaos-experiment-scope/)
- [Simulate Pod Faults](https://chaos-mesh.org/docs/2.7.2/simulate-pod-chaos-on-kubernetes/)
- [Simulate Network Faults](https://chaos-mesh.org/docs/next/simulate-network-chaos-on-kubernetes/)
- [Create Chaos Mesh Workflow](https://chaos-mesh.org/docs/create-chaos-mesh-workflow/)
- [Securing tenant namespaces using restrict authorization feature in Chaos Mesh](https://chaos-mesh.org/blog/securing-tenant-namespaces-using-restrict-authorization-feature/)
