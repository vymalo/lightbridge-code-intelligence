/**
 * Client-side aggregation for the Overview insights surface (ADR-0024). Pure functions over the
 * already-fetched task list — no control-plane aggregation endpoint yet (flagged follow-up). UTC day
 * bucketing keeps the time series deterministic across SSR/CSR (matches `dayBucketKey` in tasks.ts).
 */

import {
  durationSeconds,
  repoLabel,
  type StatusVariant,
  statusVisual,
  type Task,
} from "@/lib/tasks";

const DAY_MS = 86_400_000;
const MONTH_DAY_FORMATTER = new Intl.DateTimeFormat("en", {
  month: "short",
  day: "numeric",
  timeZone: "UTC",
});

export interface Kpis {
  total: number;
  /** Succeeded / (succeeded + failed), or `null` when nothing has completed. */
  passRate: number | null;
  /** Median duration in seconds over completed runs, or `null`. */
  p50Seconds: number | null;
  active: number;
}

export function computeKpis(tasks: Task[], now: number): Kpis {
  let success = 0;
  let completed = 0;
  let active = 0;
  const durations: number[] = [];
  for (const t of tasks) {
    const variant = statusVisual(t.status).variant;
    if (variant === "active") active++;
    if (variant === "success" || variant === "error") {
      completed++;
      if (variant === "success") success++;
      const d = durationSeconds(t, now);
      if (d !== null) durations.push(d);
    }
  }
  return {
    total: tasks.length,
    passRate: completed ? success / completed : null,
    p50Seconds: durations.length ? median(durations) : null,
    active,
  };
}

function median(values: number[]): number {
  if (values.length === 0) return 0;
  const sorted = [...values].sort((a, b) => a - b);
  const mid = Math.floor(sorted.length / 2);
  const hi = sorted[mid] ?? 0;
  if (sorted.length % 2) return hi;
  const lo = sorted[mid - 1] ?? hi;
  return Math.round((lo + hi) / 2);
}

export interface DayBucket {
  key: string;
  label: string;
  count: number;
}

/** Runs created per UTC day over the last `days` days (inclusive of today), zero-filled. */
export function runsPerDay(tasks: Task[], now: number, days = 14): DayBucket[] {
  const today = Math.floor(now / DAY_MS);
  const buckets = new Map<string, number>();
  const order: string[] = [];
  for (let i = days - 1; i >= 0; i--) {
    const key = new Date((today - i) * DAY_MS).toISOString().slice(0, 10);
    buckets.set(key, 0);
    order.push(key);
  }
  for (const t of tasks) {
    const key = t.created_at.slice(0, 10);
    if (buckets.has(key)) buckets.set(key, (buckets.get(key) ?? 0) + 1);
  }
  return order.map((key) => ({
    key,
    label: MONTH_DAY_FORMATTER.format(new Date(`${key}T00:00:00Z`)),
    count: buckets.get(key) ?? 0,
  }));
}

export interface Slice {
  label: string;
  count: number;
}

/** Top repositories by run count (descending), capped at `limit`. */
export function breakdownByRepo(tasks: Task[], limit = 6): Slice[] {
  const counts = new Map<string, number>();
  for (const t of tasks) {
    const label = repoLabel(t);
    counts.set(label, (counts.get(label) ?? 0) + 1);
  }
  return [...counts.entries()]
    .map(([label, count]) => ({ label, count }))
    .sort((a, b) => b.count - a.count)
    .slice(0, limit);
}

const OUTCOME_ORDER: { variant: StatusVariant; label: string }[] = [
  { variant: "success", label: "Succeeded" },
  { variant: "error", label: "Failed" },
  { variant: "active", label: "Running" },
  { variant: "pending", label: "Pending" },
  { variant: "muted", label: "Cancelled" },
];

/** Run counts per status variant, in a fixed order, omitting empty buckets. */
export function breakdownByOutcome(tasks: Task[]): Slice[] {
  const counts = new Map<StatusVariant, number>();
  for (const t of tasks) {
    const variant = statusVisual(t.status).variant;
    counts.set(variant, (counts.get(variant) ?? 0) + 1);
  }
  return OUTCOME_ORDER.map(({ variant, label }) => ({
    label,
    count: counts.get(variant) ?? 0,
  })).filter((s) => s.count > 0);
}

/** Compact duration label from seconds (mirrors `duration` in tasks.ts, but from a number). */
export function formatSeconds(seconds: number): string {
  if (seconds < 60) return `${seconds}s`;
  const minutes = Math.floor(seconds / 60);
  const rem = seconds % 60;
  if (minutes < 60) return rem ? `${minutes}m ${rem}s` : `${minutes}m`;
  const hours = Math.floor(minutes / 60);
  return `${hours}h ${minutes % 60}m`;
}
