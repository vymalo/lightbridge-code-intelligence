// Resolve the active session for an incoming request from its session cookie.

import { verifyJwt } from "../jwt";

interface Session {
  userId: string;
  /** Unix seconds at which the session expires. */
  expiresAt: number;
}

/**
 * Verify the session cookie and return the active session, or null if the cookie
 * is not a valid, current session. `nowUnix` is the current time in Unix seconds.
 */
export function getActiveSession(cookie: string, nowUnix: number): Session | null {
  const payload = verifyJwt(cookie);
  if (!payload || !payload.sub) {
    return null;
  }
  return { userId: payload.sub, expiresAt: payload.exp };
}
