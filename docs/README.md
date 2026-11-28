# `docs/` — Architecture, API Contracts, and Operations Docs

Key documentation in `docs/`:

- **System Specification**: [SPEC.md](SPEC.md)
- **Authorization & Identity Model**: [authorization-model.md](authorization-model.md)
- **Database Schema & Migrations**: [database-schema.md](database-schema.md)
- **HTTP Compatibility & Provider Authz**: [porting-go-http-contract.md](porting-go-http-contract.md), [provider-user-authorization.md](provider-user-authorization.md), [http-capabilities.md](http-capabilities.md)
- **Operations & Admin UI**: [runtime-modes.md](runtime-modes.md), [backup-restore.md](backup-restore.md), [admin-ui.md](admin-ui.md), [single-node.md](single-node.md), [rhiza-operator.md](rhiza-operator.md)
- **Design Specs**: [designs/lookup-resources.md](designs/lookup-resources.md)

Run `sh scripts/verify-docs.sh` to validate all relative markdown links and schema inventory references.
