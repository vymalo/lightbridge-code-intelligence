import Link from "next/link";
import { Card, CardBody, CardHeader, CardTitle } from "@/components/ui/card";
import {
  breakdownByOutcome,
  breakdownByRepo,
  computeKpis,
  formatSeconds,
  runsPerDay,
  type Slice,
} from "@/lib/insights";
import type { Task } from "@/lib/tasks";

/** Operator-glance insights over the fetched run list (ADR-0024, Dub pattern): KPI cards that drill
 * into the filtered runs list, a hand-rolled runs-over-time series (single accent, no chart lib), and
 * breakdown cards with daisyUI `progress` bars. Server-rendered; aggregation is pure (`lib/insights`). */
export function Insights({ tasks, now }: { tasks: Task[]; now: number }) {
  const kpis = computeKpis(tasks, now);
  const series = runsPerDay(tasks, now, 14);
  const byRepo = breakdownByRepo(tasks);
  const byOutcome = breakdownByOutcome(tasks);

  return (
    <div className="flex flex-col gap-4">
      <div className="grid grid-cols-2 gap-3 lg:grid-cols-4">
        <Kpi label="Total runs" value={String(kpis.total)} href="/dashboard/runs" />
        <Kpi
          label="Pass rate"
          value={kpis.passRate === null ? "—" : `${Math.round(kpis.passRate * 100)}%`}
          href="/dashboard/runs?status=success"
        />
        <Kpi
          label="p50 duration"
          value={kpis.p50Seconds === null ? "—" : formatSeconds(kpis.p50Seconds)}
          href="/dashboard/runs"
        />
        <Kpi label="Active" value={String(kpis.active)} href="/dashboard/runs?status=active" />
      </div>

      <Card>
        <CardHeader className="flex items-baseline justify-between">
          <CardTitle>Runs over time</CardTitle>
          <span className="text-xs text-base-content/60">last 14 days</span>
        </CardHeader>
        <CardBody>
          <Sparkline data={series} />
        </CardBody>
      </Card>

      <div className="grid gap-3 sm:grid-cols-2">
        <BreakdownCard title="By repository" slices={byRepo} mono />
        <BreakdownCard title="By outcome" slices={byOutcome} />
      </div>
    </div>
  );
}

function Kpi({ label, value, href }: { label: string; value: string; href: string }) {
  return (
    <Link
      href={href}
      className="card card-border bg-base-200 p-3 transition-colors hover:bg-base-300/50"
    >
      <div className="text-2xl font-semibold tabular-nums">{value}</div>
      <div className="mt-0.5 text-xs text-base-content/60">{label}</div>
    </Link>
  );
}

/** Inline SVG runs-over-time series. Fixed viewBox + non-scaling stroke so it stays crisp at any
 * width; single-accent fill/stroke, no chart library (ADR-0027 keeps ADR-0015's restraint). */
function Sparkline({ data }: { data: { key: string; label: string; count: number }[] }) {
  const total = data.reduce((sum, d) => sum + d.count, 0);
  const max = Math.max(1, ...data.map((d) => d.count));
  const points = data.map((d, i) => {
    const x = data.length > 1 ? (i / (data.length - 1)) * 100 : 50;
    const y = 32 - (d.count / max) * 28; // leave ~4u headroom so peaks aren't clipped
    return { x, y };
  });
  const first = points.at(0);
  const last = points.at(-1);
  if (!first || !last) return null; // no buckets — nothing to draw
  const line = points.map((p) => `${p.x.toFixed(2)},${p.y.toFixed(2)}`).join(" ");
  const area = [
    `M ${first.x.toFixed(2)},32`,
    ...points.map((p) => `L ${p.x.toFixed(2)},${p.y.toFixed(2)}`),
    `L ${last.x.toFixed(2)},32 Z`,
  ].join(" ");

  return (
    <div className="flex flex-col gap-1">
      {/* svg is decorative; the caption below carries the meaning for assistive tech. */}
      <svg
        viewBox="0 0 100 32"
        preserveAspectRatio="none"
        className="h-24 w-full text-primary"
        aria-hidden="true"
      >
        <title>Runs over time</title>
        <path d={area} fill="currentColor" fillOpacity={0.12} />
        <polyline
          points={line}
          fill="none"
          stroke="currentColor"
          strokeWidth={1.5}
          strokeLinejoin="round"
          strokeLinecap="round"
          vectorEffect="non-scaling-stroke"
        />
      </svg>
      <div className="flex items-center justify-between text-xs text-base-content/60">
        <span>{data[0]?.label}</span>
        <span className="tabular-nums">
          {total} run{total === 1 ? "" : "s"} · peak {max}/day
        </span>
        <span>{data[data.length - 1]?.label}</span>
      </div>
    </div>
  );
}

function BreakdownCard({
  title,
  slices,
  mono,
}: {
  title: string;
  slices: Slice[];
  mono?: boolean;
}) {
  const max = Math.max(1, ...slices.map((s) => s.count));
  return (
    <Card>
      <CardHeader>
        <CardTitle>{title}</CardTitle>
      </CardHeader>
      <CardBody>
        {slices.length === 0 ? (
          <p className="text-sm text-base-content/60">No data.</p>
        ) : (
          <ul className="flex flex-col gap-2">
            {slices.map((s) => (
              <li key={s.label} className="flex items-center gap-3">
                <span className={`w-40 shrink-0 truncate text-xs ${mono ? "font-mono" : ""}`}>
                  {s.label}
                </span>
                {/* Native <progress> via daisyUI — semantic + accessible (implicit meter role). */}
                <progress
                  className="progress progress-primary h-2 flex-1"
                  value={s.count}
                  max={max}
                >
                  {s.count}
                </progress>
                <span className="w-8 shrink-0 text-right text-xs tabular-nums text-base-content/60">
                  {s.count}
                </span>
              </li>
            ))}
          </ul>
        )}
      </CardBody>
    </Card>
  );
}
