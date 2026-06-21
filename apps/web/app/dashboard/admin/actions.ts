"use server";

import { revalidatePath } from "next/cache";
import { hasPermission, setRepoStatus } from "@/lib/admin";
import { currentClaims } from "@/lib/session";

/**
 * Shared body for the approve/deny actions. Server Actions are public POST endpoints, so this
 * authorizes the caller on the specific permission (`repo:approve` / `repo:deny`, ADR-0023) and
 * validates input here too — defense in depth on top of the control plane's own gate — and throws on
 * failure so the UI surfaces it instead of a silent "success". (Not exported: a "use server" module
 * may only export actions.)
 */
async function mutate(formData: FormData, action: "approve" | "deny"): Promise<void> {
  if (!hasPermission(await currentClaims(), `repo:${action}`)) {
    throw new Error(`Unauthorized: repo:${action} permission required`);
  }
  const id = Number(formData.get("id"));
  if (!Number.isInteger(id) || id <= 0) {
    throw new Error("Invalid repository id");
  }
  if (!(await setRepoStatus(id, action))) {
    throw new Error(`Failed to ${action} repository`);
  }
  revalidatePath("/dashboard/admin");
}

/** Approve a pending repository (opens the gate + triggers its base index), then refresh the queue. */
export async function approveRepoAction(formData: FormData): Promise<void> {
  await mutate(formData, "approve");
}

/** Deny a pending repository (keeps it out of scope + purges any index data), then refresh. */
export async function denyRepoAction(formData: FormData): Promise<void> {
  await mutate(formData, "deny");
}
