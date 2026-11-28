# `internal/api/` — HTTP Router & Endpoint Contracts

`internal/api` exposes Grant's HTTP API over Go's `net/http` `ServeMux` and enforces authentication, strict JSON decoding, and `learner` node gating.

## Key Files

- **Server, Middleware & JSON Validation**: [http.go](http.go)
  - Enforces `master` (`X-Grant-Master-Key`), `application` (`X-Grant-App-Auth-Key` + Bearer/Gateway + `X-Organization-ID`), `user`, and `http-decision` authentication boundaries.
  - Rejects duplicate security headers (`400`) and restricts `learner` nodes to `/healthz`, `/readyz`, `/api/v2/http-capabilities/{catalog/*,decide}`, `/api/v1/authz/{check,batch-check,filter}`, `/api/v1/application-authz/{check,batch-check,filter}`, and `/api/v2/providers/{providerId}/authz/{batch-check,filter}`.
- **Route Registration & Inventory**: [routes.go](routes.go), [testdata/routes-voter.txt](testdata/routes-voter.txt), [management_inventory_test.go](management_inventory_test.go)
  - `routes-voter.txt` tracks the `98` registered HTTP routes.
- **Authorization Endpoints**: [authz.go](authz.go), [provider_user_authz.go](provider_user_authz.go)
  - `/api/v1/authz/filter`, `/api/v1/application-authz/filter`, and `/api/v2/providers/{providerId}/authz/filter` support both candidate-list filtering (`resources`) and bounded keyset reverse lookup (`resource: { provider, type }`, `limit`, `cursor`).
