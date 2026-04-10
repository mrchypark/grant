# TrailBase Premium Login Design

## Summary

Build a premium branded login screen in front of TrailBase while keeping TrailBase as the authentication engine and token issuer.

The screen should provide:

- a large logo slot with no hardcoded asset yet
- email and password sign-in
- social buttons for Google, Kakao, and Apple
- an Auth0-like polished visual treatment with stronger brand expression

## Goals

- Replace direct use of the default TrailBase login UI as the primary user entrypoint.
- Keep TrailBase on the official auth API path instead of forking its auth internals.
- Ship a premium login experience that works on desktop and mobile.
- Enable Google and Apple with TrailBase-supported provider flows immediately.
- Enable Kakao without forking TrailBase if possible.

## Non-Goals

- Rebuild TrailBase token issuance or session management from scratch.
- Fork TrailBase to add a first-class Kakao provider at this stage.
- Build a full marketing landing page around login.
- Finalize permanent brand copy or logo assets.

## Context

TrailBase officially supports two integration modes:

1. Use the built-in auth UI mounted under `/_/auth/...`.
2. Build a custom UI against TrailBase auth APIs.

The current service already wraps TrailBase inside `trailbase-auth-bridge`, which keeps TrailBase routes available and adds internal introspection for `certd`.

TrailBase source inspection shows:

- built-in providers for Google and Apple
- a generic `OIDC0` provider
- support for disabling the built-in auth UI
- a public provider listing endpoint and provider-specific login endpoints

This makes a headless custom login page the lowest-risk path.

## Decision

Adopt a custom frontend login page as the main entrypoint and keep TrailBase as a headless auth backend.

Use:

- password login via `/api/auth/v1/login`
- Google via `/api/auth/v1/oauth/google/login`
- Apple via `/api/auth/v1/oauth/apple/login`
- Kakao via `/api/auth/v1/oauth/oidc0/login`, with `oidc0` configured against Kakao's OIDC-compatible endpoints if Kakao OIDC is sufficient for the desired scope

Do not fork TrailBase for a dedicated Kakao provider unless OIDC-based integration proves insufficient.

Canonical route policy:

- unauthenticated users land on `/login`
- `/` redirects to `/login` when custom login is enabled
- `/` and `/login` both redirect to `/_/auth/login` when fallback mode is enabled
- authenticated users should bypass `/login` and continue to the product's default post-login destination

## Why This Approach

### Benefits

- preserves TrailBase upgradeability
- keeps all token issuance and session semantics inside TrailBase
- avoids a custom security boundary for Google and Apple
- gives full visual control over the login experience
- allows Kakao to be attempted as configuration first rather than code first

### Rejected Alternatives

#### Keep the built-in TrailBase auth UI

Rejected because the default UI is intentionally basic and not a good fit for a premium branded entrypoint.

#### Add a dedicated Kakao provider to TrailBase now

Rejected because it creates upstream divergence and long-term maintenance debt for a provider that may already be expressible through TrailBase's generic OIDC support.

#### Move all OAuth flows fully into the Axum bridge

Rejected for now because it would require a new secure user provisioning and TrailBase session exchange path. That is a larger auth architecture change than this task requires.

## UX Direction

### Visual Style

- premium branded product login
- bright neutral foundation with tinted surfaces
- large typographic hierarchy
- strong but restrained accent color
- subtle depth, texture, and motion
- no generic SaaS card grid styling

### Layout

Desktop:

- centered shell with generous whitespace
- left visual or brand column
- right login panel
- logo slot at the top of the panel

Mobile:

- stacked single-column layout
- brand copy collapses above the form
- all controls remain first-screen accessible

### Content Structure

- logo slot placeholder
- title
- short supporting copy
- email field
- password field
- primary sign-in button
- divider
- social buttons: Google, Kakao, Apple
- secondary support links such as password reset

## Route Design

Primary user entrypoint:

- `/login`

Canonical unauthenticated entry behavior:

- `/login` is the only canonical custom login route.
- `/` must server-side redirect to `/login` when the custom login experience is enabled.
- `/` must server-side redirect to `/_/auth/login` when fallback mode is enabled.
- `/login` must also server-side redirect to `/_/auth/login` when fallback mode is enabled, so direct bookmarks and stale links do not bypass rollback.
- The `/login` routing contract is not complete until the route terminates in the minimal custom shell itself; redirect plumbing without a reachable shell is considered incomplete, and temporary placeholder endpoints are not part of the intended contract.

Backend endpoints used by the page:

- `POST /api/auth/v1/login`
- `GET /api/auth/v1/oauth/google/login`
- `GET /api/auth/v1/oauth/apple/login`
- `GET /api/auth/v1/oauth/oidc0/login`
- optional `GET /api/auth/v1/oauth/providers` for dynamic provider availability

Built-in TrailBase auth pages remain available only as fallback during rollout. They may be disabled later once the custom page is proven.

## Kakao Strategy

### Phase 1

Treat Kakao as the generic OIDC provider.

Configuration expectations:

- `provider_id = OIDC0`
- provider map key remains `oidc0`
- `display_name = "Kakao"`
- `auth_url`, `token_url`, and `user_api_url` point to Kakao-compatible OIDC endpoints

Operational rule:

- `oidc0` is treated as a Kakao-dedicated slot in this system and must not be repointed to another issuer without an explicit migration plan

The UI should label the button as `Kakao`, even though the backend route remains `oidc0`.

Kakao must not be treated as production-ready on configuration alone. Before enabling the button in production, the implementation must prove:

- issuer discovery and metadata are accepted by the configured TrailBase OIDC flow
- issuer value is stable and explicitly expected
- JWKS-based signature validation succeeds for the configured issuer
- `aud` and any provider-specific authorized-party semantics match the configured client
- `sub`, `email`, and verification-related claims map cleanly to TrailBase expectations
- the trusted identity source is explicitly defined as ID token claims, userinfo claims, or a documented combination
- requested scopes are sufficient for the desired user identity contract
- the callback and token exchange succeed end-to-end against a real Kakao app configuration

If discovery, claims, or scope behavior do not match the expected contract, the Kakao button must be hidden and the rollout must stop at Google and Apple.

Kakao must remain behind a dedicated enablement flag that is off by default. The button is hidden until production-like OIDC verification is complete.

### Social Identity Linking Policy

All social providers, including Google, Apple, and Kakao, must satisfy the same account-linking policy.

Required policy:

- authoritative external account key is provider name plus provider subject
- email-based linking is allowed only if TrailBase treats the external identity as verified and the email claim is present and verified according to the provider contract
- if verified email is absent, hidden, private, or ambiguous, the provider login must not auto-link to an existing account
- duplicate-account creation risk must be treated as a launch blocker until the linking policy is verified with real provider responses
- collisions must fail closed rather than implicitly merging identities
- if current backend behavior cannot enforce this policy, rollout requires a backend change before social login is exposed
- if any existing identities were created under different linking semantics, rollout requires a migration or explicit compatibility decision before enabling new provider buttons

### Phase 2 Fallback

If Kakao cannot be integrated cleanly through OIDC, only then consider moving Kakao into a separate bridge-managed OAuth flow. That would still be preferred over forking TrailBase.

## Technical Design

### Frontend Serving

Use TrailBase's static `public_dir` support to serve the custom login frontend from the same service origin.

Why:

- avoids cross-origin auth complexity
- keeps redirects and cookies same-origin
- fits TrailBase's existing static asset serving model

Asset namespace rules:

- custom frontend assets must live under a dedicated prefix such as `/login-assets/`
- asset filenames must be versioned or content-hashed for cache-busting
- reserved prefixes `/api/`, `/_/auth/`, and provider callback routes must never be served from static assets

Runtime config rules:

- rollout booleans such as custom-login enablement, fallback mode, and Kakao gating may use defaults only when the env var is absent
- malformed rollout booleans are fatal configuration errors because silent fallback could expose the wrong login surface during rollout or rollback
- `/login` HTML should use rollback-friendly caching rather than long-lived caching
- no service worker should control `/login`

### Route Ownership And Precedence

Static asset serving must not shadow or replace TrailBase auth and API routes.

Hard constraints:

- `/api/**` always remains owned by TrailBase
- `/_/auth/**` always remains owned by TrailBase while fallback mode exists
- OAuth callback routes remain TrailBase-owned, not frontend-owned
- the custom frontend owns `/login` and its static assets only

Expected callback ownership:

- Google callback: `/api/auth/v1/oauth/google/callback`
- Apple callback: `/api/auth/v1/oauth/apple/callback`
- Kakao callback through generic OIDC: `/api/auth/v1/oauth/oidc0/callback`

The frontend initiates auth. TrailBase remains the sole callback handler and token issuer.

### Bridge Changes

`trailbase-auth-bridge` should be updated to pass a real `public_dir` into TrailBase server initialization, pointing at built frontend assets.

The bridge should continue to merge TrailBase routes and the introspection route without changing auth semantics.

### Login Submission

Email/password flow:

- submit credentials by same-origin `fetch` to TrailBase login API using the documented JSON request contract
- preserve TrailBase's standard token and cookie/session handling without inventing a parallel token format
- show inline error messaging on failure
- do not turn the frontend into a custom token issuer or session layer
- verify the exact JSON request and response behavior against TrailBase during implementation

OAuth flow:

- social buttons navigate to provider login endpoints
- use provider-specific routes for Google and Apple
- use `oidc0` for Kakao

### CSRF, State, Nonce, And PKCE Contract

- OAuth `state`, `nonce`, callback verification, and PKCE remain TrailBase-owned concerns.
- The custom frontend only initiates OAuth by navigating to TrailBase provider login endpoints.
- The frontend must not generate or validate OAuth `state` or `nonce` independently.
- If native-app or cross-origin PKCE flows are introduced later, they must use TrailBase's documented code-flow contract rather than a custom token exchange.
- For password login, the first implementation uses a same-origin JSON `fetch` flow that follows TrailBase's documented `/api/auth/v1/login` contract.
- Login CSRF defense is mandatory for the browser login surface. The implementation must explicitly protect against cross-site login initiation and must verify the deployed `Origin`, cookie, and `SameSite` behavior before rollout.
- verify session fixation protections on successful login
- ship a restrictive CSP for `/login`
- Replay, invalid-state, and callback-failure cases must be covered in verification.

### Return Path Policy

- post-login navigation may accept an allowlisted `return_to` or `next` value
- only relative in-app paths are allowed
- invalid, missing, or cross-origin targets fall back to the default product destination
- the same redirect policy must behave consistently through `/login` and `/_/auth/login` during fallback mode
- normalization and validation must happen server-side, not only in frontend code
- the allowlist must be strict enough to prevent open redirects and must reject protocol-relative or encoded external targets

### Session Handling Rules

- Do not store auth or refresh tokens in `localStorage` or `sessionStorage`.
- Do not proxy raw user credentials through a custom bridge endpoint if TrailBase can receive them directly.
- Do not mint or transform TrailBase auth tokens in custom frontend logic.
- If the frontend needs persisted session state, prefer secure same-origin cookie behavior already supported by TrailBase.
- Logout must continue to use TrailBase logout semantics so refresh token invalidation remains authoritative.
- Any cached frontend auth state must be dropped immediately on logout, even if a short-lived auth token may still exist until expiry.

### Provider Availability

The UI may use a static expected set at first, but should be designed so the visible buttons can later be driven by `/api/auth/v1/oauth/providers`.

This lets operations disable a provider without editing frontend layout logic.

## Security Considerations

- Stay on same-origin routes where possible.
- Do not proxy passwords through custom non-TrailBase APIs unless required.
- Keep TrailBase as the single issuer of auth and refresh tokens.
- Require HTTPS in all deployed environments.
- Avoid splitting account-linking logic across multiple auth backends.
- Only promote Kakao beyond OIDC if the fallback path has a clear user-linking and token-issuance design.

## Compatibility And Upgrade Gates

This design depends on a narrow TrailBase contract:

- `public_dir` serving remains available
- the login endpoint remains `/api/auth/v1/login`
- OAuth initiation endpoints remain `/api/auth/v1/oauth/<provider>/login`
- OAuth callback ownership remains inside TrailBase
- `OIDC0` remains configurable with custom `display_name`, `auth_url`, `token_url`, and `user_api_url`

Before disabling the built-in auth UI, add verification that pins this contract:

- bridge-level smoke tests for `/login`, `/api/auth/v1/login`, and provider redirect initiation
- at least one test asserting static asset serving does not shadow `/api/**`
- an upgrade smoke checklist to run whenever the TrailBase dependency is bumped

If any of these assumptions change in a TrailBase upgrade, the custom login rollout pauses until compatibility is restored.

## Error Handling

- invalid credentials: inline error under the form
- temporarily disabled provider: disabled button or friendly inline notice
- provider redirect failure: recoverable error state with retry action
- missing provider config: hide the button or show unavailable state, not a broken redirect

## Accessibility

- explicit labels for email and password
- visible focus states on all inputs and buttons
- keyboard reachable social buttons
- sufficient contrast in all states
- reduced-motion safe page transitions

## Rollout Plan

1. Add a config flag or environment switch that can immediately route users back to the built-in TrailBase UI.
2. Implement and prove the fallback behavior for both `/` and `/login`.
3. Exercise a rollback drill before any production exposure.
4. Add frontend asset serving support to the bridge.
5. Implement the premium `/login` page.
6. Wire email/password to TrailBase login.
7. Complete provider preflight for Google and Apple and keep those buttons hidden until configuration is verified.
8. Wire Google and Apple buttons to provider endpoints.
9. Configure `oidc0` for Kakao but keep the Kakao button hidden until OIDC and identity-linking proof is complete.
10. Keep built-in TrailBase auth UI available during verification.
11. Disable built-in auth UI only after the custom flow is verified and rollback has been exercised.

## Testing Strategy

- manual verification of desktop and mobile layouts
- email/password success and failure flows
- Google redirect initiation
- Apple redirect initiation
- Kakao redirect initiation through `oidc0`
- logout and session invalidation still work as expected
- verify the custom page remains functional when individual providers are disabled
- verify `/login` assets do not shadow `/api/**` or `/_/auth/**`
- verify fallback to built-in TrailBase UI through the rollback switch
- verify TrailBase dependency upgrades against the contract above before rollout changes
- verify invalid OAuth callback, invalid state, and replay-style failure handling remain TrailBase-owned and safe
- verify asset prefixing and cache-busting prevent stale custom login assets from surviving rollback
- verify fresh `/login` HTML is served after rollback switch changes

## Rollback Strategy

Rollback must be operationally cheap.

Requirements:

- a config switch can disable the custom `/login` entrypoint and return users to the built-in TrailBase UI
- the same switch must control both `/` and `/login` server-side behavior
- the built-in UI remains available until the custom page has passed production verification
- provider-specific incidents can hide only the affected provider button without taking down the whole login page
- asset caching for the custom login page must use a versioned build or cache-busting strategy so rollback does not leave stale branded assets active at the entry route

Minimum production verification before disabling fallback:

- password login success
- password login failure handling
- Google redirect initiation
- Apple redirect initiation
- Kakao redirect initiation only if OIDC proof is complete
- logout and re-login behavior

## Provider Preflight Checklist

Google:

- OAuth client created for the deployed site
- exact redirect URI registered
- client id and secret stored in deployment secrets
- login initiation and callback verified in deployed environment

Apple:

- Apple service configuration created
- exact redirect URI registered
- required Apple key, team, and client configuration validated
- Apple domain and service verification completed where required
- client configuration stored in deployment secrets
- login initiation and callback verified in deployed environment

Kakao:

- OIDC discovery metadata validated
- exact redirect URI registered
- required scopes confirmed
- real provider claims captured and checked against linking policy
- production-safe `sub` and verified-email behavior confirmed before enabling the button

## Open Questions

- Which build tool should generate the static frontend assets for `public_dir`?
- Should the provider buttons be rendered dynamically from TrailBase config on first implementation, or be hardcoded and validated at startup?

## Recommendation

Proceed with a custom premium login page served from the same origin, using TrailBase auth APIs directly and treating Kakao as an OIDC-backed provider first.
