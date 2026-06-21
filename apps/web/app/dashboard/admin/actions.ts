"use server";

import { revalidatePath } from "next/cache";
import { setRepoStatus } from "@/lib/admin";

/** Approve a pending repository (opens the gate + triggers its base index), then refresh the queue. */
export async function approveRepoAction(formData: FormData): Promise<void> {
  const id = Number(formData.get("id"));
  if (Number.isFinite(id)) await setRepoStatus(id, "approve");
  revalidatePath("/dashboard/admin");
}

/** Deny a pending repository (keeps it out of scope + purges any index data), then refresh. */
export async function denyRepoAction(formData: FormData): Promise<void> {
  const id = Number(formData.get("id"));
  if (Number.isFinite(id)) await setRepoStatus(id, "deny");
  revalidatePath("/dashboard/admin");
}
