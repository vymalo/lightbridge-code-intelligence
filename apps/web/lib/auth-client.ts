"use client";

import { rustBackendClient } from "@lightbridge/auth/client";
import { createAuthClient } from "better-auth/react";

export const authClient = createAuthClient({
  plugins: [rustBackendClient()],
});
