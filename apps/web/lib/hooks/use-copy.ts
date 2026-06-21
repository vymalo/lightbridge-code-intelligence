"use client";

import { useCallback, useEffect, useRef, useState } from "react";

/**
 * Copy-to-clipboard with a transient "copied" flash. Returns `copied` (true for `resetMs` after a
 * successful copy) and a `copy(text)` callback. The timer is cleared on unmount and between copies,
 * so the flag never flips on an unmounted component. `copy` no-ops (returns false) when the clipboard
 * API is unavailable (insecure context / older browser) or the text is empty.
 */
export function useCopyToClipboard(resetMs = 1500) {
  const [copied, setCopied] = useState(false);
  const timer = useRef<ReturnType<typeof setTimeout> | undefined>(undefined);

  useEffect(() => () => clearTimeout(timer.current), []);

  const copy = useCallback(
    async (text: string): Promise<boolean> => {
      if (!navigator?.clipboard || !text) return false;
      try {
        await navigator.clipboard.writeText(text);
        setCopied(true);
        clearTimeout(timer.current);
        timer.current = setTimeout(() => setCopied(false), resetMs);
        return true;
      } catch {
        // Clipboard can reject (document not focused, permissions) — leave the flag down.
        return false;
      }
    },
    [resetMs],
  );

  return { copied, copy };
}
