# `internal/` — Go Application Packages

`internal/` holds all private Go packages for `grant`:

| Package | Responsibility |
|---|---|
| [grant](grant/README.md) | Domain models, configuration (`ConfigFromEnv`), external IdP verification, authorization evaluation, Rhiza consensus/replica store, and SQL schema/indexes |
| [api](api/README.md) | Public/private HTTP routing, JSON validation (`decode`, `validateJSON`), authentication middleware, and `/api/v1/*`, `/api/v2/*`, `/ops/*` handlers |
| [adminauth](adminauth/README.md) | OAuth 2.0 + PKCE login flow, CSRF/Origin validation, and encrypted session cookie handling for `grant-admin` |
| [adminui](adminui/README.md) | Server-rendered HTML/HTMX administrative console and API client for `grant-admin` |
