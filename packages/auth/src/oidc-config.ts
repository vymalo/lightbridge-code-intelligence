/**
 * Config for the OIDC Authorization-Code (+PKCE) flow run by the web app's Node route handlers.
 * Kept free of any `openid-client` import so this module stays usable from Edge code; the flow
 * itself lives in `apps/web` (Node runtime).
 */
export interface OidcClientConfig {
  issuer: string;
  clientId: string;
  /**
   * Client secret. Omit for a **public client** (PKCE only) — this is the default for local dev so
   * no secret is committed. Set `OIDC_CLIENT_SECRET` in production for a confidential client.
   */
  clientSecret?: string;
  redirectUri: string;
  postLogoutRedirectUri: string;
  /** Space-delimited scopes; defaults to `openid profile email`. */
  scope: string;
}

function required(name: string): string {
  const value = process.env[name];
  if (!value) throw new Error(`${name} must be set`);
  return value;
}

/** Build {@link OidcClientConfig} from env. Throws if a required variable is missing. */
export function oidcClientConfigFromEnv(): OidcClientConfig {
  return {
    issuer: required("OIDC_ISSUER").replace(/\/+$/, ""),
    clientId: required("OIDC_CLIENT_ID"),
    clientSecret: process.env.OIDC_CLIENT_SECRET || undefined,
    redirectUri: process.env.OIDC_REDIRECT_URI ?? "http://localhost:3000/api/auth/callback",
    postLogoutRedirectUri: process.env.OIDC_POST_LOGOUT_REDIRECT_URI ?? "http://localhost:3000",
    scope: process.env.OIDC_SCOPE ?? "openid profile email",
  };
}
