import type { BetterAuthClientPlugin } from "better-auth/client";
import type { rustBackendPlugin } from "./rust-backend-plugin";

/**
 * Client counterpart to {@link rustBackendPlugin}. Lets the better-auth client infer the
 * `/rust-backend/sign-in` endpoint so it can be called type-safely from the browser.
 */
export const rustBackendClient = () => {
  return {
    id: "rust-backend",
    $InferServerPlugin: {} as ReturnType<typeof rustBackendPlugin>,
    pathMethods: {
      "/rust-backend/sign-in": "POST",
    },
  } satisfies BetterAuthClientPlugin;
};
