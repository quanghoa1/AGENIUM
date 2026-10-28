import { CircleCheck, CircleDashed, Link2, ListChecks, Pause, Play } from "lucide-react";
import { useEffect, useState } from "react";
import { ProgressBar } from "../../components/motion";
import { Empty, Mono, TicketStatusBadge, cn } from "../../components/ui";
import { api } from "../../lib/api";
import { useApp } from "../../lib/store";
import type { Column, LiveData, PlanView } from "../../lib/types";

type TaskState = "done" | "playing" | "paused" | "queued" | "todo";

/** The state icon of a task: a "playing" button while someone works on it, a tick when done. */
function TaskIcon({ s }: { s: TaskState }) {
  if (s === "done") return <CircleCheck className="h-5 w-5 shrink-0 text-emerald-500" />;
  if (s === "playing")
    return (
      <span className="relative inline-flex h-5 w-5 shrink-0">
        <span className="absolute inset-0 animate-ping rounded-full bg-blue-500/40" />
        <span className="relative inline-flex h-5 w-5 items-center justify-center rounded-full bg-blue-600 text-white">
          <Play className="ml-px h-2.5 w-2.5 fill-current" />
        </span>
      </span>
    );
  if (s === "paused")
    return (
      <span className="inline-flex h-5 w-5 shrink-0 items-center justify-center rounded-full bg-amber-500 text-white">
        <Pause className="h-2.5 w-2.5 fill-current" />
      </span>
    );
  if (s === "queued") return <span className="h-5 w-5 shrink-0 rounded-full border-2 border-blue-500/50" />;
  return <CircleDashed className="h-5 w-5 shrink-0 text-subtle" />;
}

/** The room's plan as a task list: each plan item with its tickets, who holds them and where they stand. */
export function PlanList({ project, data }: { project?: string | null; data: LiveData }) {
  const app = useApp();
  const [plan, setPlan] = useState<PlanView | null>(null);
  const [cols, setCols] = useState<Column[]>([]);
  useEffect(() => {
    if (!project) return;
    api.plan(project).then(setPlan);
    api.board(project).then((c) => setCols(c ?? []));
  }, [project, app.tick]);
  if (!project) return <div className="p-4 text-[11px] text-subtle">{app.t("live.noProject")}</div>;
  const cards = new Map(cols.flatMap((c) => c.cards.map((card) => [card.key, { ...card, status: c.status }] as const)));
  const running = new Set(data.agents.filter((a) => a.run?.ticket_key).map((a) => a.run!.ticket_key!));
  const milestones = plan?.plan?.body.milestones ?? [];
  const items = plan?.items ?? [];
  const stateOf = (i: (typeof items)[number]): TaskState => {
    if (i.done) return "done";
    const sts = i.tickets.filter(([, st]) => st !== "Cancelled");
    if (sts.some(([k, st]) => st === "InProgress" || running.has(k))) return "playing";
    if (sts.some(([, st]) => st === "Pending")) return "paused";
    return sts.length ? "queued" : "todo";
  };
  const done = items.filter((i) => i.done).length;
  return (
    <div className="min-h-0 flex-1 overflow-y-auto p-4">
      {!plan?.plan ? (
        <Empty>{app.t("live.noPlan")}</Empty>
      ) : (
        <>
          <div className="mb-3 flex items-center gap-3">
            <ListChecks className="h-4 w-4 text-accent" />
            <span className="text-sm font-semibold">Plan v{plan.plan.version}</span>
            <div className="h-1.5 flex-1 overflow-hidden rounded-full bg-line">
              <ProgressBar value={items.length ? done / items.length : 0} className="bg-emerald-500" />
            </div>
            <Mono className="text-subtle">
              {done}/{items.length} {app.t("live.tasksDone")}
            </Mono>
          </div>
          <div className="space-y-4">
            {milestones.map((m) => (
              <div key={m.id}>
                <div className="label mb-1.5">
                  {m.id} · {m.title}
                </div>
                <div className="space-y-1.5">
                  {items
                    .filter((i) => i.milestone === m.id)
                    .map((i) => {
                      const s = stateOf(i);
                      return (
                        <div key={i.id} className={cn("flex items-start gap-3 rounded-lg border p-2.5", s === "playing" ? "border-blue-500/50 bg-blue-500/5" : s === "paused" ? "border-amber-500/40 bg-amber-500/5" : "border-line")}>
                          <TaskIcon s={s} />
                          <div className="min-w-0 flex-1">
                            <div className="flex items-center gap-1.5 text-xs">
                              <Mono className="text-subtle">{i.id}</Mono>
                              <span className={cn("truncate font-medium", s === "done" && "text-subtle line-through")}>{i.title}</span>
                            </div>
                            {i.tickets.map(([key, st]) => {
                              const c = cards.get(key);
                              return (
                                <div key={key} className="mt-1 flex flex-wrap items-center gap-1.5 text-[11px]">
                                  <Mono className="text-cyan-600 dark:text-cyan-400">{key}</Mono>
                                  <TicketStatusBadge status={st} />
                                  {c?.assignee && <span className="text-muted">@{c.assignee}</span>}
                                  {c?.blocked_by.length ? (
                                    <span className="inline-flex items-center gap-0.5 text-amber-600 dark:text-amber-400">
                                      <Link2 className="h-3 w-3" /> {c.blocked_by.join(", ")}
                                    </span>
                                  ) : null}
                                </div>
                              );
                            })}
                          </div>
                        </div>
                      );
                    })}
                </div>
              </div>
            ))}
          </div>
        </>
      )}
    </div>
  );
}
