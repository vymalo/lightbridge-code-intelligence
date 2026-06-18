import { createAuth } from "@lightbridge/auth";

// Require an explicit secret when actually serving in production; never fall back to the
// public dev value there, or session/encryption material would be forgeable. The check is
// skipped during `next build` (NEXT_PHASE), where the secret is injected at deploy time.
const secret = process.env.BETTER_AUTH_SECRET;
const isBuildPhase = process.env.NEXT_PHASE === "phase-production-build";
if (!secret && process.env.NODE_ENV === "production" && !isBuildPhase) {
  throw new Error("BETTER_AUTH_SECRET must be set in production");
}

// The app wires environment into the shared, env-free auth factory.
export const auth = createAuth({
  backendUrl: process.env.AUTH_BACKEND_URL ?? "http://localhost:8080",
  secret: secret ?? "dev-only-insecure-secret-change-me",
  baseURL: process.env.BETTER_AUTH_URL ?? "http://localhost:3000",
});
