"use client";

import { useEffect, useRef, useState } from "react";

/**
 * Live agent-Job log stream for a run (Epic #75, Milestone C). Reads the `text/plain` streaming
 * response from `/api/runs/{id}/logs` chunk by chunk and appends to a scrolling pane. Read-only;
 * cleans up the fetch on unmount.
 */
export function RunLogs({ taskId }: { taskId: string }) {
  const [text, setText] = useState("");
  const [state, setState] = useState<"streaming" | "done" | "error">("streaming");
  const preRef = useRef<HTMLPreElement>(null);

  useEffect(() => {
    const controller = new AbortController();
    let cancelled = false;

    (async () => {
      try {
        const res = await fetch(`/api/runs/${encodeURIComponent(taskId)}/logs`, {
          signal: controller.signal,
        });
        if (!res.ok || !res.body) {
          const detail = await res.text().catch(() => "");
          if (!cancelled) {
            setText(detail || `Failed to load logs (HTTP ${res.status}).`);
            setState("error");
          }
          return;
        }
        const reader = res.body.pipeThrough(new TextDecoderStream()).getReader();
        for (;;) {
          const { done, value } = await reader.read();
          if (done) break;
          if (value && !cancelled) setText((prev) => prev + value);
        }
        if (!cancelled) setState("done");
      } catch {
        // A real fetch/stream failure (network/reset). Unmount aborts set `cancelled`, so this only
        // marks an error for genuine failures.
        if (!cancelled) setState("error");
      }
    })();

    return () => {
      cancelled = true;
      controller.abort();
    };
  }, [taskId]);

  // Auto-scroll to the newest lines — but only if the user is already near the bottom, so scrolling
  // up to read earlier output isn't yanked away. `text` is the intended trigger.
  // biome-ignore lint/correctness/useExhaustiveDependencies: text is the scroll trigger
  useEffect(() => {
    const el = preRef.current;
    if (el && el.scrollHeight - el.scrollTop - el.clientHeight < 100) {
      el.scrollTop = el.scrollHeight;
    }
  }, [text]);

  return (
    <div className="flex flex-col gap-2">
      <pre
        ref={preRef}
        className="max-h-96 overflow-auto rounded-md bg-[var(--surface)] p-3 font-mono text-xs leading-relaxed text-muted-foreground"
      >
        {text || (state === "streaming" ? "Connecting to the run's logs…" : "No log output.")}
      </pre>
      <span className="text-xs text-muted-foreground">
        {state === "streaming" && "● streaming live"}
        {state === "done" && "stream ended"}
        {state === "error" && "could not stream logs"}
      </span>
    </div>
  );
}
