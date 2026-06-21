/**
 * Verified identity claims from an OIDC access token (Keycloak). Only the fields the app reads are
 * typed; signature / issuer / audience / expiry are checked by {@link verifyAccessToken}.
 */
export interface SessionClaims {
  /** Stable subject identifier (the user's id in the IdP). */
  sub: string;
  email?: string;
  preferred_username?: string;
  name?: string;
  /** Keycloak realm roles, used for the admin gate (Epic #75). Absent for non-Keycloak IdPs. */
  realm_access?: { roles?: string[] };
  /** Expiry, unix seconds. */
  exp: number;
}
