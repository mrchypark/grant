# `deploy/` — Kubernetes & Container Deployment Manifests

`deploy/` contains base Kubernetes manifests and environment overlays for `grant` and its companion services.

- **Base Manifests**: [k8s](k8s) (StatefulSets, ConfigMaps, Services for `grant` and `rauthy`)
- **Overlays**: [overlays](overlays) (`production`, `ephemeral`, `rhiza-operator`, `standalone`, `standalone-nopvc`, `private-runtime`, `ied`)
- **Chaos Testing**: [chaos-mesh](chaos-mesh/README.md)
- **IdP Launchers**: [rauthy](rauthy), [rauthy-launcher](rauthy-launcher)

Run `sh scripts/verify-rhiza-deployment.sh` after editing deployment manifests.
