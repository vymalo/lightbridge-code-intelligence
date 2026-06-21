"use client";

import { Check, Copy, Download, Search } from "lucide-react";
import { useEffect, useMemo, useRef, useState } from "react";
import { cn } from "@/lib/cn";

type Level = "all" | "info" | "warn" | "error";

const LEVELS: { value: Level; label: string }[] = [
  { value: "all", label: "All levels" },
  { value: "info", label: "Info" },
  { value: "warn", label: "Warnings" },
  { value: "error", label: "Errors" },
];

// Heuristic per-line level match — agent/Job logs aren't a fixed format, so we match the level word
// (and a couple of common synonyms) case-insensitively rather than parse a schema.
function lineMatchesLevel(line: string, level: Level): boolean {
  if (level === "all") return true;
  const l = line.toLowerCase();
  if (level === "warn") return l.includes("warn");
  if (level === "error") return l.includes("error") || l.includes("fatal");
  return l.includes("info");
}

/**
 * Live agent-Job log stream for a run (Epic #75; filter bar added in ADR-0024, Lovable pattern).
 * Reads the `text/plain` streaming response from `/api/runs/{id}/logs` chunk by chunk; the level
 * filter, search, copy, and download all operate on the buffered text client-side. Read-only.
 */
export function RunLogs({ taskId }: { taskId: string }) {
  const [text, setText] = useState("");
  const [state, setState] = useState<"streaming" | "done" | "error">("streaming");
  const [level, setLevel] = useState<Level>("all");
  const [query, setQuery] = useState("");
  const [copied, setCopied] = useState(false);
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

  const allLines = useMemo(() => (text ? text.split("\n") : []), [text]);
  const filtering = level !== "all" || query.trim() !== "";
  const shown = useMemo(() => {
    if (!filtering) return allLines;
    const q = query.trim().toLowerCase();
    return allLines.filter(
      (line) => lineMatchesLevel(line, level) && (!q || line.toLowerCase().includes(q)),
    );
  }, [allLines, level, query, filtering]);

  // Auto-scroll to the newest lines while live — but only when not filtering (filtering implies the
  // user is hunting, not tailing) and only if already near the bottom, so reading earlier output
  // isn't yanked away. `shown` is the intended trigger.
  // biome-ignore lint/correctness/useExhaustiveDependencies: shown is the scroll trigger
  useEffect(() => {
    if (filtering) return;
    const el = preRef.current;
    if (el && el.scrollHeight - el.scrollTop - el.clientHeight < 100) {
      el.scrollTop = el.scrollHeight;
    }
  }, [shown, filtering]);

  const copy = async () => {
    if (!navigator?.clipboard) return;
    await navigator.clipboard.writeText(shown.join("\n"));
    setCopied(true);
    setTimeout(() => setCopied(false), 1500);
  };

  const download = () => {
    const blob = new Blob([text], { type: "text/plain" });
    const url = URL.createObjectURL(blob);
    const a = document.createElement("a");
    a.href = url;
    a.download = `run-${taskId}.log`;
    a.click();
    URL.revokeObjectURL(url);
  };

  const body = text ? shown.join("\n") : "";
  const placeholder = state === "streaming" ? "Connecting to the run's logs…" : "No log output.";

  return (
    <div className="flex flex-col gap-2">
      <div className="flex flex-wrap items-center gap-2">
        <select
          value={level}
          onChange={(e) => setLevel(e.target.value as Level)}
          aria-label="Filter logs by level"
          className="h-7 rounded-md border border-border bg-background px-2 text-xs text-foreground outline-none focus:border-border-strong focus:ring-1 focus:ring-ring"
        >
          {LEVELS.map((l) => (
            <option key={l.value} value={l.value}>
              {l.label}
            </option>
          ))}
        </select>
        <div className="relative">
          <Search className="pointer-events-none absolute left-2 top-1/2 size-3.5 -translate-y-1/2 text-muted-foreground" />
          <input
            type="search"
            value={query}
            onChange={(e) => setQuery(e.target.value)}
            placeholder="Search logs"
            className="h-7 w-44 rounded-md border border-border bg-background pl-7 pr-2 text-xs outline-none placeholder:text-muted-foreground focus:border-border-strong focus:ring-1 focus:ring-ring"
          />
        </div>
        <div className="ml-auto flex items-center gap-1">
          <LogAction onClick={copy} label={copied ? "Copied" : "Copy"} disabled={!text}>
            {copied ? <Check className="size-3.5" /> : <Copy className="size-3.5" />}
            {copied ? "Copied" : "Copy"}
          </LogAction>
          <LogAction onClick={download} label="Download" disabled={!text}>
            <Download className="size-3.5" />
            Download
          </LogAction>
        </div>
      </div>

      <pre
        ref={preRef}
        className="max-h-96 overflow-auto rounded-md bg-[var(--surface)] p-3 font-mono text-xs leading-relaxed text-muted-foreground"
      >
        {body || (filtering ? "No lines match the current filters." : placeholder)}
      </pre>

      <span className="text-xs text-muted-foreground">
        {state === "streaming" && "● streaming live"}
        {state === "done" && "stream ended"}
        {state === "error" && "could not stream logs"}
        {text && filtering && ` · showing ${shown.length} of ${allLines.length} lines`}
      </span>
    </div>
  );
}

function LogAction({
  onClick,
  label,
  disabled,
  children,
}: {
  onClick: () => void;
  label: string;
  disabled?: boolean;
  children: React.ReactNode;
}) {
  return (
    <button
      type="button"
      onClick={onClick}
      disabled={disabled}
      aria-label={label}
      className={cn(
        "inline-flex items-center gap-1 rounded-md border border-border px-2 py-1 text-xs text-muted-foreground transition-colors hover:bg-muted hover:text-foreground",
        "disabled:cursor-not-allowed disabled:opacity-40",
      )}
    >
      {children}
    </button>
  );
}
