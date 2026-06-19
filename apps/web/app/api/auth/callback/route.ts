import { cookieOptions, PKCE_COOKIE, SESSION_COOKIE, STATE_COOKIE } from "@lightbridge/auth";
import { type NextRequest, NextResponse } from "next/server";
import * as client from "openid-client";
import { getOidc } from "@/lib/oidc";

export const runtime = "nodejs";

/** Handle the IdP redirect back: exchange the code for tokens and set the session cookie. */
export async function GET(req: NextRequest) {
  const { config } = await getOidc();

  const codeVerifier = req.cookies.get(PKCE_COOKIE)?.value;
  const expectedState = req.cookies.get(STATE_COOKIE)?.value;
  if (!codeVerifier || !expectedState) {
    return NextResponse.redirect(new URL("/sign-in?error=missing_state", req.url));
  }

  let tokens: client.TokenEndpointResponse;
  try {
    tokens = await client.authorizationCodeGrant(config, new URL(req.url), {
      pkceCodeVerifier: codeVerifier,
      expectedState,
    });
  } catch {
    return NextResponse.redirect(new URL("/sign-in?error=exchange_failed", req.url));
  }

  // The access token is the bearer credential sent to the control plane (resource server) and the
  // value the middleware validates. Cookie lifetime tracks the token's own expiry.
  const maxAge = typeof tokens.expires_in === "number" ? tokens.expires_in : 1800;

  const res = NextResponse.redirect(new URL("/dashboard", req.url));
  res.cookies.set(SESSION_COOKIE, tokens.access_token, cookieOptions(maxAge));
  res.cookies.delete(PKCE_COOKIE);
  res.cookies.delete(STATE_COOKIE);
  return res;
}
