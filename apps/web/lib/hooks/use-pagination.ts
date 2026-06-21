"use client";

import { useMemo } from "react";

/** A page-worth of `items` plus the derived bookkeeping a pager needs. */
export interface Page<T> {
  /** Items on the current page. */
  rows: T[];
  /** Total number of pages (≥ 1). */
  pageCount: number;
  /** Current page index, clamped into `[0, pageCount - 1]` (so a shrunk list can't strand the offset). */
  current: number;
  /** Index of the first item on the page (into the full list). */
  start: number;
  /** Total item count. */
  total: number;
  /** Ready-made "1–25 of 132" / "No results" range label. */
  rangeLabel: string;
}

/** Client-side pagination over an already-filtered list. Pure derivation (memoized) — the page index
 * itself is owned by the caller (URL state via nuqs), so this stays a controlled view. */
export function usePagination<T>(items: T[], pageSize: number, page: number): Page<T> {
  return useMemo(() => {
    const pageCount = Math.max(1, Math.ceil(items.length / pageSize));
    const current = Math.min(Math.max(0, page), pageCount - 1);
    const start = current * pageSize;
    const rows = items.slice(start, start + pageSize);
    const rangeLabel =
      items.length === 0
        ? "No results"
        : `${start + 1}–${Math.min(start + pageSize, items.length)} of ${items.length}`;
    return { rows, pageCount, current, start, total: items.length, rangeLabel };
  }, [items, pageSize, page]);
}
