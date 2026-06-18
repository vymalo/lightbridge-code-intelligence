import { toNextJsHandler } from "better-auth/next-js";
import { auth } from "@/lib/auth";

// Mounts all better-auth endpoints (including the custom /rust-backend/sign-in) under /api/auth.
export const { GET, POST } = toNextJsHandler(auth);
