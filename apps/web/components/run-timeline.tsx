import { RunRow } from "@/components/run-row";
import { dayBucketKey, dayBucketLabel, type Task } from "@/lib/tasks";

/** Runs grouped by calendar day on a thin rail (ADR-0024, Doppler activity-log pattern). Tasks arrive
 * already filtered and most-recent-first; we keep that order and split on the day boundary. */
export function RunTimeline({ tasks, now }: { tasks: Task[]; now: number }) {
  const groups: { key: string; label: string; tasks: Task[] }[] = [];
  for (const task of tasks) {
    const key = dayBucketKey(task.created_at);
    const last = groups.at(-1);
    if (last?.key === key) {
      last.tasks.push(task);
    } else {
      groups.push({ key, label: dayBucketLabel(task.created_at, now), tasks: [task] });
    }
  }

  return (
    <div className="flex flex-col">
      {groups.map((group) => (
        <section key={group.key}>
          <h3 className="sticky top-0 z-10 bg-surface/95 px-4 py-2 text-xs font-medium text-muted-foreground backdrop-blur">
            {group.label}
          </h3>
          {/* Thin rail to the left of the day's runs (depth via border, not shadow — ADR-0015). */}
          <div className="ml-5 border-l border-border">
            <div className="divide-y divide-border">
              {group.tasks.map((task) => (
                <RunRow key={task.id} task={task} now={now} />
              ))}
            </div>
          </div>
        </section>
      ))}
    </div>
  );
}
