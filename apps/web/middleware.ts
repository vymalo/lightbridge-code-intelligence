import {
  appBaseUrl,
  SESSION_COOKIE,
  verifyAccessToken,
  verifyConfigFromEnv,
} from "@lightbridge/auth";
import { type NextRequest, NextResponse } from "next/server";

// Protect the dashboard (and anything under it). Runs on the Edge runtime — uses `jose` only.
export const config = { matcher: ["/dashboard", "/dashboard/:path*"] };

export async function middleware(req: NextRequest) {
  const token = req.cookies.get(SESSION_COOKIE)?.value;
  const claims = token ? await verifyAccessToken(token, verifyConfigFromEnv()) : null;
  if (!claims) {
    // Anchor to the configured public origin — `req.url`'s host is the pod's internal bind
    // address behind the ingress.
    return NextResponse.redirect(new URL("/api/auth/login", appBaseUrl()));
  }
  return NextResponse.next();
}
