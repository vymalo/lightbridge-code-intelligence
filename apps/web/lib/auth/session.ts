import {
  SESSION_COOKIE,
  type SessionClaims,
  verifyAccessToken,
  verifyConfigFromEnv,
} from "@lightbridge/auth";
import { cookies } from "next/headers";

/** Verified claims for the current request, or null. Server-only (reads the httpOnly cookie). */
export async function currentClaims(): Promise<SessionClaims | null> {
  const token = (await cookies()).get(SESSION_COOKIE)?.value;
  if (!token) return null;
  return verifyAccessToken(token, verifyConfigFromEnv());
}

/** Best display name for a signed-in user. */
export function displayName(claims: SessionClaims): string {
  return claims.name ?? claims.preferred_username ?? claims.email ?? claims.sub;
}
