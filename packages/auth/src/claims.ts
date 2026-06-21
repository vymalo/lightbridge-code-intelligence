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
  /** Expiry, unix seconds. */
  exp: number;
  /** Any other claim — so the configurable **permissions** claim (ADR-0023) can be read by path. */
  [claim: string]: unknown;
}
