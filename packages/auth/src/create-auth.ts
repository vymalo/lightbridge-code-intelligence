import { betterAuth } from "better-auth";
import { nextCookies } from "better-auth/next-js";
import { rustBackendPlugin } from "./rust-backend-plugin";

export interface CreateAuthOptions {
  /** Base URL of the standalone Rust auth backend (control plane). */
  backendUrl: string;
  /** better-auth signing secret (BETTER_AUTH_SECRET). */
  secret: string;
  /** Public base URL of the web app (BETTER_AUTH_URL). */
  baseURL: string;
}

/**
 * Build the app's better-auth instance.
 *
 * No database is configured: sessions are stateless and credential verification is delegated
 * to the standalone Rust backend through {@link rustBackendPlugin}. A production deployment
 * either adds a database adapter or fully offloads identity to the Rust backend (ADR-0007).
 *
 * `nextCookies()` must remain the LAST plugin so it can finalize Set-Cookie headers.
 */
export const createAuth = (options: CreateAuthOptions) =>
  betterAuth({
    secret: options.secret,
    baseURL: options.baseURL,
    plugins: [rustBackendPlugin({ backendUrl: options.backendUrl }), nextCookies()],
  });
