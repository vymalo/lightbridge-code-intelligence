import type { BetterAuthPlugin } from "better-auth";
import { createAuthEndpoint } from "better-auth/api";

export interface RustBackendOptions {
  /** Base URL of the standalone, portable Rust auth backend (control plane). */
  backendUrl: string;
}

interface VerifyResponse {
  ok: boolean;
  user: { id: string; email: string; name?: string | null } | null;
  reason?: string | null;
}

/**
 * better-auth server plugin that delegates credential verification to the standalone Rust
 * backend (`POST {backendUrl}/auth/verify`).
 *
 * This is authentication (authN) only. The gateway authorization path
 * (Envoy/Authorino + lightbridge-authz) is a separate component and is NOT this plugin.
 *
 * SKELETON: on success it returns the verified user. Promoting that into a full better-auth
 * session (create session + set cookie via `ctx.context`) is the documented next step — see
 * ADR-0007. The request/response contract is pinned by the Rust-side wiremock test
 * (services/control-plane/tests/auth_contract.rs).
 */
export const rustBackendPlugin = (options: RustBackendOptions) => {
  return {
    id: "rust-backend",
    endpoints: {
      rustBackendSignIn: createAuthEndpoint(
        "/rust-backend/sign-in",
        { method: "POST" },
        async (ctx) => {
          const body = (ctx.body ?? {}) as { email?: string; password?: string };
          // Strip trailing slashes so we never build `…//auth/verify`.
          const baseUrl = options.backendUrl.replace(/\/+$/, "");

          const res = await fetch(`${baseUrl}/auth/verify`, {
            method: "POST",
            headers: { "content-type": "application/json" },
            body: JSON.stringify({ email: body.email, password: body.password }),
          }).catch(() => null);

          if (!res) {
            return ctx.json({ ok: false, reason: "auth backend unavailable" }, { status: 503 });
          }

          const data = (await res.json().catch(() => null)) as VerifyResponse | null;
          // TODO(ADR-0007): on data.ok, create a better-auth session and set the cookie.
          // Propagate the backend's status so callers can distinguish failures from successes.
          return ctx.json(data ?? { ok: false, reason: "invalid backend response" }, {
            status: res.ok ? 200 : res.status,
          });
        },
      ),
    },
  } satisfies BetterAuthPlugin;
};
