# TrailBase Auth Bridge

`trailbase-auth-bridge` is a standalone TrailBase-based auth service for `grant`.

Responsibilities:

- Run TrailBase auth and admin UI as a separate service
- Expose `POST /api/grant/introspect` for internal `grant` token validation
- Protect the introspection endpoint with `x-grant-internal-secret`

Environment:

- `TRAILBASE_GRANT_INTERNAL_SECRET` required
- `TRAILBASE_DATA_DIR` optional, default `./traildepot`
- `TRAILBASE_ADDRESS` optional, default `0.0.0.0:4000`
- `TRAILBASE_ADMIN_ADDRESS` optional
- `TRAILBASE_DEV` optional
- `TRAILBASE_LOG_RESPONSES` optional

Run:

```bash
cargo run --manifest-path crates/trailbase-auth-bridge/Cargo.toml
```

The bridge exposes the normal TrailBase routes plus:

- `GET /healthz`
- `POST /api/grant/introspect`

`grant` should be configured with:

- `GRANT_TRAILBASE_BASE_URL`
- `GRANT_TRAILBASE_INTROSPECT_PATH=/api/grant/introspect`
- `GRANT_TRAILBASE_INTERNAL_SECRET`
- `GRANT_TRAILBASE_CACHE_TTL_SECONDS`
