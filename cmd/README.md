# `cmd/` — Grant Binaries & CLI Tools

This directory contains the executable entrypoints built by `grant`:

| Binary | Purpose | Entrypoint |
|---|---|---|
| `grant` | Core HTTP authorization server (`voter` or `learner`) and runtime/migration commands | [grant/main.go](grant/main.go) |
| `grant-admin` | Administrative web UI server backed by OIDC/PKCE (`internal/adminauth`) | [grant-admin/main.go](grant-admin/main.go) |
| `grant-management-catalog` | Generates the management HTTP capability catalog (`77` routes) | [grant-management-catalog/main.go](grant-management-catalog/main.go) |

See [docs/SPEC.md](../docs/SPEC.md) for runtime modes, environment variables, and container packaging details.
