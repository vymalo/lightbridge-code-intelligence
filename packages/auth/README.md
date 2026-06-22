# @lightbridge/auth

Shared **OIDC / JWT helpers** for the TypeScript side — the bits the web console needs to log a user
in against Keycloak and to verify the resulting access tokens
([ADR-0014](../../docs/adr/0014-keycloak-oidc-resource-server.md)). No credentials are stored here; it
only reads config from env and validates tokens.

## Surface

One entrypoint (`src/index.ts`), grouped by concern:

| Module | Exports | Purpose |
|---|---|---|
| `oidc-config` | `oidcClientConfigFromEnv`, `appBaseUrl`, `OidcClientConfig` | Build the OIDC client config (issuer, client id, redirect) from env |
| `verify-jwt` | `verifyAccessToken`, `verifyConfigFromEnv`, `VerifyConfig` | Verify an OIDC access token (signature, issuer, audience) via `jose` |
| `claims` | `SessionClaims` | The typed claim shape carried in the session (incl. the `permissions` list, [ADR-0023](../../docs/adr/0023-db-backed-rbac.md)) |
| `session-cookie` | `SESSION_COOKIE`, `STATE_COOKIE`, `PKCE_COOKIE`, `cookieOptions` | Cookie names + hardened options for the Auth Code + PKCE flow |

## Consumed by

`apps/web` (`lib/auth/*`, `middleware.ts`, and the `app/api/auth/*` route handlers). The control plane
verifies the same tokens independently in Rust — this package is the **web** half of that contract.
