import type { ReactNode } from "react";
import type { ApiResult } from "@/lib/api";

/**
 * Honest states (ADR-0016). The first-run *empty* is the one place a centered placard is right
 * (the screen has nothing else to do — house rule); errors are inline status lines, not placards.
 */

/** Centered first-run prompt: no runs yet → tell the user how to get one. */
export function EmptyState({
  title,
  children,
  action,
}: {
  title: string;
  children: ReactNode;
  action?: ReactNode;
}) {
  return (
    <div className="flex min-h-[40dvh] flex-col items-center justify-center gap-3 text-center">
      <h2 className="text-base font-medium">{title}</h2>
      <p className="max-w-md text-sm text-base-content/60">{children}</p>
      {action && <div className="mt-1">{action}</div>}
    </div>
  );
}

/** Inline status line for per-section errors/unavailability (not a placard). */
export function StatusLine({
  tone = "muted",
  children,
}: {
  tone?: "muted" | "error";
  children: ReactNode;
}) {
  return (
    <p className={`px-4 py-3 text-sm ${tone === "error" ? "text-error" : "text-base-content/60"}`}>
      {children}
    </p>
  );
}

/** Render the standard failure line for a failed `ApiResult`. */
export function ApiErrorLine({ result }: { result: Extract<ApiResult<unknown>, { ok: false }> }) {
  if (result.reason === "unauthenticated") {
    return (
      <StatusLine tone="error">
        Your session can't access the control plane.{" "}
        <a className="underline underline-offset-2" href="/api/auth/login">
          Sign in again
        </a>
        .
      </StatusLine>
    );
  }
  if (result.reason === "unavailable") {
    return <StatusLine>The control plane is unreachable right now. Try again shortly.</StatusLine>;
  }
  return (
    <StatusLine tone="error">
      Couldn't load data{result.status ? ` (HTTP ${result.status})` : ""}.
    </StatusLine>
  );
}
