# TrailBase Auth Bridge

`trailbase-auth-bridge` is a standalone TrailBase-based auth service for `certd`.

Responsibilities:

- Run TrailBase auth and admin UI as a separate service
- Expose `POST /api/certd/introspect` for internal `certd` token validation
- Protect the introspection endpoint with `x-certd-internal-secret`

Environment:

- `TRAILBASE_CERTD_INTERNAL_SECRET` required
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
- `POST /api/certd/introspect`

`certd` should be configured with:

- `CERTD_TRAILBASE_BASE_URL`
- `CERTD_TRAILBASE_INTROSPECT_PATH=/api/certd/introspect`
- `CERTD_TRAILBASE_INTERNAL_SECRET`
- `CERTD_TRAILBASE_CACHE_TTL_SECONDS`
