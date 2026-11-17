# `internal/adminui/` — Admin Web Console

`internal/adminui` implements the HTML/HTMX administrative web console for `grant-admin`.

- **HTTP Handlers & Dispatch**: [server.go](server.go)
- **Grant API Client**: [client.go](client.go)
- **Resource Views & Templates**: [resources.go](resources.go), [templates](templates), [assets](assets)

See [docs/admin-ui.md](../../docs/admin-ui.md) for UI capabilities and security boundaries.
