import { PassThrough, Readable } from "node:stream";
import { CoreV1Api, KubeConfig, Log } from "@kubernetes/client-node";
import { currentClaims } from "@/lib/auth/session";
import { hasPermission } from "@/lib/server/admin";
import { getTask } from "@/lib/server/api";

/**
 * Streams a run's agent-Job logs to the console (Epic #75, Milestone C).
 *
 * The web pod has its own ServiceAccount with read access to pods/log in the agents namespace (the
 * decision for this milestone), so it reads logs directly from the Kubernetes API rather than through
 * the control plane. The task's `job_name` (from the control plane, which the caller is authorized
 * for via OIDC) selects the Job; we stream its pod's logs as `text/plain`.
 *
 * Node runtime (the kube client + node streams aren't Edge-safe).
 */
export const runtime = "nodejs";
export const dynamic = "force-dynamic";

const AGENTS_NAMESPACE = process.env.AGENT_NAMESPACE?.trim() || "lightbridge-agents";

function text(body: string, status = 200): Response {
  return new Response(body, { status, headers: { "content-type": "text/plain; charset=utf-8" } });
}

export async function GET(
  request: Request,
  { params }: { params: Promise<{ id: string }> },
): Promise<Response> {
  // Requires the `task:logs` permission (ADR-0023); the task lookup below is also OIDC-authorized
  // against the control plane.
  const claims = await currentClaims();
  if (!claims) return text("unauthenticated", 401);
  if (!hasPermission(claims, "task:logs")) return text("missing task:logs permission", 403);

  const { id } = await params;
  const task = await getTask(id);
  if (!task.ok)
    return text(
      `could not load run (${task.reason})`,
      task.reason === "unauthenticated" ? 401 : 502,
    );
  if (!task.data) return text("run not found", 404);
  const jobName = task.data.job_name;
  if (!jobName) return text("No Job for this run yet — logs appear once it's dispatched.");

  // Resolve the Job's pod. Jobs label their pods with `batch.kubernetes.io/job-name`.
  let kc: KubeConfig;
  try {
    kc = new KubeConfig();
    kc.loadFromCluster();
  } catch {
    return text("log streaming is unavailable (no in-cluster Kubernetes config)", 503);
  }
  const core = kc.makeApiClient(CoreV1Api);

  let podName: string | undefined;
  try {
    const pods = await core.listNamespacedPod({
      namespace: AGENTS_NAMESPACE,
      labelSelector: `batch.kubernetes.io/job-name=${jobName}`,
    });
    // Newest pod (a retried Job may have several); fall back to the first.
    podName = pods.items.slice().sort((a, b) => {
      const ta = new Date(a.metadata?.creationTimestamp ?? 0).getTime();
      const tb = new Date(b.metadata?.creationTimestamp ?? 0).getTime();
      return tb - ta;
    })[0]?.metadata?.name;
  } catch (error) {
    return text(`could not list pods for ${jobName}: ${String(error)}`, 502);
  }
  if (!podName) {
    return text("No pod found for this run — its Job may have been cleaned up.");
  }

  // Stream the pod's logs (follow). The kube client writes to a Node stream; bridge it to the web
  // Response and abort the upstream request when the client disconnects.
  const passthrough = new PassThrough();
  try {
    const controller = await new Log(kc).log(AGENTS_NAMESPACE, podName, "", passthrough, {
      follow: true,
      tailLines: 2000,
      timestamps: false,
    });
    request.signal.addEventListener("abort", () => {
      controller.abort();
      passthrough.destroy();
    });
  } catch (error) {
    return text(`could not stream logs for ${podName}: ${String(error)}`, 502);
  }

  return new Response(Readable.toWeb(passthrough) as ReadableStream, {
    headers: {
      "content-type": "text/plain; charset=utf-8",
      "cache-control": "no-store, no-transform",
    },
  });
}
