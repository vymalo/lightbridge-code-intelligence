"use client";

import { Check, Copy } from "lucide-react";
import { Button } from "@/components/ui/button";
import { useCopyToClipboard } from "@/lib/hooks/use-copy";

/** A read-only shell command with a copy button — e.g. the `kubectl logs` one-liner so a user can
 * stream a run's logs from their own terminal. */
export function CommandSnippet({ command, label }: { command: string; label?: string }) {
  const { copied, copy } = useCopyToClipboard();

  return (
    <div className="flex flex-col gap-1.5">
      {label && <span className="text-xs text-muted-foreground">{label}</span>}
      <div className="flex items-center gap-2 rounded-md border border-border bg-base-200 px-2.5 py-1.5">
        <code className="min-w-0 flex-1 overflow-x-auto whitespace-nowrap font-mono text-xs">
          {command}
        </code>
        <Button
          variant="ghost"
          size="xs"
          onClick={() => copy(command)}
          aria-label="Copy command"
          className="btn-square"
        >
          {copied ? <Check className="size-3.5" /> : <Copy className="size-3.5" />}
        </Button>
      </div>
    </div>
  );
}
