# `internal/adminauth/` — Admin UI OAuth2/PKCE & Session Auth

`internal/adminauth` handles OIDC/OAuth 2.0 authentication for `grant-admin`:

- **Implementation**: [auth.go](auth.go)
  - Implements PKCE (`S256`) authorization code login (`/login`, `/callback`, `/logout`), encrypted/signed session cookies, `Origin` validation, and CSRF token verification (`validFormCSRF`, `validOrigin`).
  - Bounds in-memory pending login states (`maxLogins = 1000`) by evicting the oldest-expiring entry when full.
