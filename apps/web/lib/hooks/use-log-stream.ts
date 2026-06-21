"use client";

import { useEffect, useState } from "react";

export type LogStreamState = "streaming" | "done" | "error";

/**
 * Stream a run's `text/plain` log response from `/api/runs/{id}/logs`, appending chunks as they
 * arrive. Returns the buffered `text` and the stream `state`. The fetch is aborted on unmount (or
 * when `taskId` changes), and a late chunk after abort never updates state. Read-only — filtering /
 * search / copy all operate on the returned `text` in the view.
 */
export function useLogStream(taskId: string): { text: string; state: LogStreamState } {
  const [text, setText] = useState("");
  const [state, setState] = useState<LogStreamState>("streaming");

  useEffect(() => {
    const controller = new AbortController();
    let cancelled = false;
    setText("");
    setState("streaming");

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
        // marks an error for genuine failures, not for navigating away.
        if (!cancelled) setState("error");
      }
    })();

    return () => {
      cancelled = true;
      controller.abort();
    };
  }, [taskId]);

  return { text, state };
}
