import { createAuth } from "@lightbridge/auth";

// The app wires environment into the shared, env-free auth factory.
export const auth = createAuth({
  backendUrl: process.env.AUTH_BACKEND_URL ?? "http://localhost:8080",
  secret: process.env.BETTER_AUTH_SECRET ?? "dev-only-insecure-secret-change-me",
  baseURL: process.env.BETTER_AUTH_URL ?? "http://localhost:3000",
});
