"use client";

import { Check, Copy } from "lucide-react";
import { useState } from "react";

/** A read-only shell command with a copy button — e.g. the `kubectl logs` one-liner so a user can
 * stream a run's logs from their own terminal. */
export function CommandSnippet({ command, label }: { command: string; label?: string }) {
  const [copied, setCopied] = useState(false);

  const copy = async () => {
    try {
      // navigator.clipboard is undefined in insecure contexts / older browsers.
      if (!navigator?.clipboard) return;
      await navigator.clipboard.writeText(command);
      setCopied(true);
      setTimeout(() => setCopied(false), 1500);
    } catch {
      // Clipboard blocked (e.g. insecure context) — the command is still selectable in the <code>.
    }
  };

  return (
    <div className="flex flex-col gap-1.5">
      {label && <span className="text-xs text-muted-foreground">{label}</span>}
      <div className="flex items-center gap-2 rounded-md border border-border bg-[var(--surface)] px-2.5 py-1.5">
        <code className="min-w-0 flex-1 overflow-x-auto whitespace-nowrap font-mono text-xs">
          {command}
        </code>
        <button
          type="button"
          onClick={copy}
          aria-label="Copy command"
          className="shrink-0 rounded p-1 text-muted-foreground transition-colors hover:bg-muted hover:text-foreground"
        >
          {copied ? <Check className="size-3.5" /> : <Copy className="size-3.5" />}
        </button>
      </div>
    </div>
  );
}
