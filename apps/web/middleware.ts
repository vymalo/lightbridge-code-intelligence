import { SESSION_COOKIE, verifyAccessToken, verifyConfigFromEnv } from "@lightbridge/auth";
import { type NextRequest, NextResponse } from "next/server";

// Protect the dashboard (and anything under it). Runs on the Edge runtime — uses `jose` only.
export const config = { matcher: ["/dashboard", "/dashboard/:path*"] };

export async function middleware(req: NextRequest) {
  const token = req.cookies.get(SESSION_COOKIE)?.value;
  const claims = token ? await verifyAccessToken(token, verifyConfigFromEnv()) : null;
  if (!claims) {
    return NextResponse.redirect(new URL("/api/auth/login", req.url));
  }
  return NextResponse.next();
}
