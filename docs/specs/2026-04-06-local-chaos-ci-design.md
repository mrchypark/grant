# Local Chaos CI Design

## Decision

Do not switch the default local chaos regression path to `vind` yet. Add a `kind`-based local bootstrap and CI runner now, and treat `vind` as a follow-up option after the reduced chaos subset is stable in CI.

## Why Not `vind` Yet

- `vind` is promising for local and CI clusters, but it introduces a new control-plane/runtime layer on top of vCluster that we do not currently exercise anywhere else in this repo.
- Our existing local scripts already assume standard Kubernetes behavior plus local image loading. `kind` satisfies that with less change and less new failure surface.
- The current regression target does not need `vind`-specific advantages such as sleep/wake or built-in load balancers.
- The hardest local chaos case, learner-to-voter network partition, is already excluded from the CI-safe subset. `vind` would not remove that gap by itself.

## Vind feasibility notes

- `vind` can be provisioned in GitHub Actions via `loft-sh/setup-vind@v1` (drop-in style with `setup-kind` and built on vCluster Docker driver).
- CI would still use the same in-repo scripts, but would need a small cluster adapter to avoid direct `kind` assumptions (`kind`-specific context naming and image loading).
- `kind load docker-image` and related `kind` checks are not needed with `vind`, because `setup-vind` relies on the same Docker daemon image cache.
- Recommendation: keep the default as `kind` for now and add `vind` only after a small adapter script is added to:
  - skip `kind`-only bootstrap steps
  - keep the manifest apply/rolling-wait flow unchanged
  - make context switching explicit (`KUBE_CONTEXT`/`kubectl config use-context`)

## What Ships Now

- `scripts/kind-bootstrap.sh`
  Creates a local `kind` cluster, loads local images, creates secrets, and applies the existing manifests without ingress.
- `scripts/kind-chaos.sh`
  Reuses the existing chaos runner with CI-safe flags.
- `scripts/ci-chaos-local.sh`
  Builds images, then dispatches bootstrap/e2e/chaos to the selected backend (`kind` by default, `vind` optional).
- `.github/workflows/chaos-local.yml`
  Runs the local chaos regression in GitHub Actions.
- `scripts/vind-bootstrap.sh`
  Reuses the same manifest and readiness flow as local bootstrap without `kind`-specific cluster creation/image-load steps.
- `scripts/vind-chaos.sh`
  Context-aware chaos wrapper for `vind` clusters.
- `.github/workflows/chaos-vind.yml`
  Manual GitHub Actions job for `CI_CLUSTER_BACKEND=vind` via `loft-sh/setup-vind@v1`.

## CI Scope

Included:
- follower voter delete
- leader voter delete
- learner restart
- TrailBase down/cache TTL/recovery

Excluded:
- learner-to-voter network partition
- MinIO disruption
- full backup/restore DR flow

## Verification

- Local `kind` run must complete `bootstrap -> e2e -> reduced chaos`.
- GitHub Actions workflow must use the same `scripts/ci-chaos-local.sh` entrypoint.
- `bash -n` and `cargo fmt --check` stay clean after the change.
