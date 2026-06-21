"use client";

import { useCallback, useEffect, useState } from "react";

/**
 * A `useState` that persists to `localStorage` under `key`. SSR-safe: the server and the first client
 * paint render `fallback` (so there's no hydration mismatch), then an effect hydrates the stored value
 * if present and valid. `validate` is a type guard that rejects stale/unknown stored values (e.g. a
 * view mode that no longer exists). Reads/writes are wrapped in try/catch — `localStorage` throws in
 * private mode / when disabled, in which case the value simply stays in memory.
 */
export function useLocalStorageState<T extends string>(
  key: string,
  fallback: T,
  validate?: (value: string) => value is T,
): [T, (value: T) => void] {
  const [value, setValue] = useState<T>(fallback);

  useEffect(() => {
    try {
      const stored = window.localStorage.getItem(key);
      if (stored !== null && (!validate || validate(stored))) setValue(stored as T);
    } catch {
      // localStorage unavailable — keep the in-memory fallback.
    }
  }, [key, validate]);

  const set = useCallback(
    (next: T) => {
      setValue(next);
      try {
        window.localStorage.setItem(key, next);
      } catch {
        // Write failed (quota / disabled) — the in-memory value still updated.
      }
    },
    [key],
  );

  return [value, set];
}
