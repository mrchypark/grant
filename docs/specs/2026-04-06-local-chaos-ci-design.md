# Local Chaos CI Design

## Decision

Superseded on 2026-04-07: after a full local `vind` bootstrap + e2e + reduced-chaos pass, the shared local/CI-safe regression path now standardizes on `vind` and removes the `kind` fallback from the entrypoint.

## Why `vind` Now

- `vind` has now completed the reduced local chaos subset in this repo: bootstrap, e2e, follower delete, leader delete, learner restart, TrailBase outage, TTL expiry, and recovery.
- `vind` removes `kind load docker-image` from the shared path and keeps the same Docker-backed image visibility model locally and in CI.
- The main runtime-specific risk is `br_netfilter` on Colima, which is now handled as an explicit `vind` preflight concern rather than a reason to maintain a second backend.

## Vind feasibility notes

- `vind` is now the only supported backend for the shared local/CI-safe chaos entrypoint.
- The entrypoint owns `vind` lifecycle directly: preflight, create/connect, bootstrap, e2e, reduced chaos, and teardown.
- CI installs the `vcluster` CLI and runs the same entrypoint as local instead of maintaining a separate `kind`-specific path.

## What Ships Now

- `scripts/ci-chaos-local.sh`
  Builds images, ensures the `vind` cluster/context, then runs bootstrap/e2e/reduced chaos as the single supported local/CI-safe path.
- `scripts/vind-cluster.sh`
  Owns `vind` lifecycle management, including Colima preflight and create/connect/delete flow.
- `.github/workflows/chaos-local.yml`
  Installs the `vcluster` CLI, preloads required kernel modules, and runs the same `vind` entrypoint used locally.
- `scripts/vind-bootstrap.sh`
  Reuses the same manifest and readiness flow behind the `vind`-managed kube context.
- `scripts/vind-chaos.sh`
  Context-aware chaos wrapper for `vind` clusters.

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

- Local `vind` run must complete `bootstrap -> e2e -> reduced chaos`.
- GitHub Actions workflow must use the same `scripts/ci-chaos-local.sh` entrypoint.
- `bash -n` and `cargo fmt --check` stay clean after the change.
