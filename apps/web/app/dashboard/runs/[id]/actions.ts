"use server";

import { revalidatePath } from "next/cache";
import { currentClaims } from "@/lib/auth/session";
import { hasPermission } from "@/lib/server/admin";
import { cancelTask } from "@/lib/server/api";

// Task ids are UUIDs; reject anything else before calling the control plane.
const UUID_RE = /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/i;

/**
 * Manually cancel a run. Server Actions are public POST endpoints, so this authorizes the caller on
 * `task:cancel` (ADR-0023) and validates the id here too — defense in depth on top of the control
 * plane's own gate — and throws on failure so the UI surfaces it rather than a silent "success".
 */
export async function cancelRunAction(formData: FormData): Promise<void> {
  if (!hasPermission(await currentClaims(), "task:cancel")) {
    throw new Error("Unauthorized: task:cancel permission required");
  }
  const id = String(formData.get("id") ?? "");
  if (!UUID_RE.test(id)) {
    throw new Error("Invalid task id");
  }
  const result = await cancelTask(id);
  if (!result.ok) {
    throw new Error("Failed to cancel the run");
  }
  revalidatePath(`/dashboard/runs/${id}`);
}
