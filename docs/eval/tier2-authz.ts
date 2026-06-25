// Dashboard API: fetch a single task by id for the authenticated caller.

import { db } from "../db";

interface Ctx {
  params: { id: string };
  /** The authenticated user's id, resolved from the session. */
  userId: string;
}

export async function getTask(ctx: Ctx): Promise<Response> {
  const task = await db.tasks.findById(ctx.params.id);
  if (!task) {
    return new Response("not found", { status: 404 });
  }
  return Response.json(task);
}
