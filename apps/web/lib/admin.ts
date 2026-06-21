import { SESSION_COOKIE, type SessionClaims } from "@lightbridge/auth";
import { cookies } from "next/headers";
import type { ApiResult } from "@/lib/api";
import type { Repository } from "@/lib/repos";

/**
 * Server-only client for the control plane's **admin** API (the approval gate, Epic #75). Like
 * lib/api it forwards the session's OIDC token; the control plane enforces the admin realm role
 * (returns 403 for non-admins). Used by the admin approval screen.
 */

function controlPlaneUrl(): string {
  return (
    process.env.CONTROL_PLANE_URL ??
    process.env.AUTH_BACKEND_URL ??
    "http://localhost:8080"
  ).replace(/\/+$/, "");
}

/** The admin realm role this deployment requires. Mirrors the control plane's `ADMIN_ROLE`. */
export function adminRole(): string {
  return process.env.ADMIN_ROLE?.trim() || "lci-admin";
}

/** Does the signed-in user hold the admin realm role? Gates the admin nav + screen (the control
 * plane is the real enforcement; this just avoids showing a screen that would only 403). */
export function isAdmin(claims: SessionClaims | null): boolean {
  return claims?.realm_access?.roles?.includes(adminRole()) ?? false;
}

function classify(status: number): "unauthenticated" | "unavailable" | "error" {
  if (status === 401 || status === 403) return "unauthenticated";
  if (status === 503) return "unavailable";
  return "error";
}

async function token(): Promise<string | null> {
  return (await cookies()).get(SESSION_COOKIE)?.value ?? null;
}

/** `GET /admin/repositories?status=pending` — the approval queue. */
export async function listPendingRepos(): Promise<ApiResult<Repository[]>> {
  try {
    const t = await token();
    if (!t) return { ok: false, reason: "unauthenticated" };
    const res = await fetch(`${controlPlaneUrl()}/admin/repositories?status=pending`, {
      headers: { authorization: `Bearer ${t}`, accept: "application/json" },
      cache: "no-store",
    });
    if (!res.ok) return { ok: false, reason: classify(res.status), status: res.status };
    return { ok: true, data: (await res.json()) as Repository[] };
  } catch {
    return { ok: false, reason: "unavailable" };
  }
}

/** `POST /admin/repositories/{id}/{approve|deny}` — returns whether it succeeded. */
export async function setRepoStatus(id: number, action: "approve" | "deny"): Promise<boolean> {
  const t = await token();
  if (!t) return false;
  try {
    const res = await fetch(`${controlPlaneUrl()}/admin/repositories/${id}/${action}`, {
      method: "POST",
      headers: { authorization: `Bearer ${t}`, accept: "application/json" },
      cache: "no-store",
    });
    return res.ok;
  } catch {
    return false;
  }
}
