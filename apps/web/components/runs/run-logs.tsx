"use client";

import { Check, Copy, Download } from "lucide-react";
import { useEffect, useMemo, useRef, useState } from "react";
import { Button } from "@/components/ui/button";
import { SearchInput } from "@/components/ui/search-input";
import { useCopyToClipboard } from "@/lib/hooks/use-copy";
import { useLogStream } from "@/lib/hooks/use-log-stream";

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
 * Live agent-Job log stream for a run (Epic #75; filter bar in ADR-0024, daisyUI in ADR-0027).
 * Streaming lives in `useLogStream`; the level filter, search, copy, and download all operate on the
 * buffered text client-side. Read-only.
 */
export function RunLogs({ taskId }: { taskId: string }) {
  const { text, state } = useLogStream(taskId);
  const { copied, copy } = useCopyToClipboard();
  const [level, setLevel] = useState<Level>("all");
  const [query, setQuery] = useState("");
  const preRef = useRef<HTMLPreElement>(null);

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

  const body = text ? shown.join("\n") : "";
  const placeholder = state === "streaming" ? "Connecting to the run's logs…" : "No log output.";

  const download = () => {
    if (!body) return;
    // Download what's shown (filtered), to match Copy. Defer revoke so the click resolves first.
    const blob = new Blob([body], { type: "text/plain" });
    const url = URL.createObjectURL(blob);
    const a = document.createElement("a");
    a.href = url;
    a.download = `run-${taskId}.log`;
    a.click();
    setTimeout(() => URL.revokeObjectURL(url), 100);
  };

  return (
    <div className="flex flex-col gap-2">
      <div className="flex flex-wrap items-center gap-2">
        <select
          value={level}
          onChange={(e) => setLevel(e.target.value as Level)}
          aria-label="Filter logs by level"
          className="select select-sm w-auto"
        >
          {LEVELS.map((l) => (
            <option key={l.value} value={l.value}>
              {l.label}
            </option>
          ))}
        </select>
        <SearchInput
          value={query}
          onChange={(e) => setQuery(e.target.value)}
          placeholder="Search logs"
          aria-label="Search logs"
          className="w-44"
        />
        <div className="ml-auto flex items-center gap-1">
          <Button variant="ghost" size="xs" onClick={() => copy(body)} disabled={!body}>
            {copied ? <Check className="size-3.5" /> : <Copy className="size-3.5" />}
            {copied ? "Copied" : "Copy"}
          </Button>
          <Button variant="ghost" size="xs" onClick={download} disabled={!body}>
            <Download className="size-3.5" />
            Download
          </Button>
        </div>
      </div>

      <pre
        ref={preRef}
        className="max-h-96 overflow-auto rounded-md bg-base-200 p-3 font-mono text-xs leading-relaxed text-base-content/60"
      >
        {body || (filtering ? "No lines match the current filters." : placeholder)}
      </pre>

      <span className="text-xs text-base-content/60">
        {state === "streaming" && "● streaming live"}
        {state === "done" && "stream ended"}
        {state === "error" && "could not stream logs"}
        {text && filtering && ` · showing ${shown.length} of ${allLines.length} lines`}
      </span>
    </div>
  );
}
