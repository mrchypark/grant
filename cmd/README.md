# `cmd/` — Grant Binaries & CLI Tools

This directory contains the executable entrypoints built by `grant`:

| Binary | Purpose | Entrypoint |
|---|---|---|
| `grant` | Core HTTP authorization server (`voter` or `learner`) and runtime/migration commands | [grant/main.go](grant/main.go) |
| `grant-admin` | Administrative web UI server backed by OIDC/PKCE (`internal/adminauth`) | [grant-admin/main.go](grant-admin/main.go) |
| `grant-management-catalog` | Generates the management HTTP capability catalog (`68` / `77` routes) | [grant-management-catalog/main.go](grant-management-catalog/main.go) |
| `grant-patch-data-manifest-digest` | Computes canonical SHA-256 digests for Patch Data authority manifests | [grant-patch-data-manifest-digest/main.go](grant-patch-data-manifest-digest/main.go) |
| `patch-data-catalog` | Generates the Patch Data service capability catalog | [patch-data-catalog/main.go](patch-data-catalog/main.go) |

See [docs/SPEC.md](../docs/SPEC.md) for runtime modes, environment variables, and container packaging details.
