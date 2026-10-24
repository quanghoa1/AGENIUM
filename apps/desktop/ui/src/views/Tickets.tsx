import { Check, CircleCheck, Circle, Link2, MessageCircleQuestion, Play } from "lucide-react";
import { useEffect, useState } from "react";
import { Empty, Mono, PageHeader, PriorityBadge, Section, TicketStatusBadge, cn } from "../components/ui";
import { api } from "../lib/api";
import { useApp } from "../lib/store";
import type { Column, InboxItem, PlanView } from "../lib/types";

type Tab = "board" | "plan" | "inbox";

export function TicketsView() {
  const app = useApp();
  const tab: Tab = (["board", "plan", "inbox"].includes(app.route.id ?? "") ? app.route.id : "board") as Tab;
  const p = app.projects.find((x) => x.id === app.project);
  const [busy, setBusy] = useState(false);
  if (!app.project || !p) return <Empty>{app.t("common.none")}</Empty>;
  return (
    <div>
      <PageHeader title={`${p.key_prefix} — ${p.name}`} sub={p.path}>
        <div className="flex rounded-lg border border-line-strong bg-elevated p-0.5">
          {(["board", "plan", "inbox"] as Tab[]).map((x) => (
            <button key={x} onClick={() => app.go("tickets", x)} className={cn("rounded-md px-3 py-1 text-xs", tab === x ? "bg-surface font-medium shadow-sm" : "text-subtle")}>
              {app.t(x === "board" ? "tickets.board" : x === "plan" ? "tickets.plan" : "tickets.inbox")}
              {x === "inbox" && p.inbox > 0 && <span className="ml-1.5 font-mono text-amber-500">{p.inbox}</span>}
            </button>
          ))}
        </div>
        <button
          className="btn"
          disabled={busy}
          title="One scheduler round: Lead inbox, work runs, reviews"
          onClick={() => {
            setBusy(true);
            api
              .runRound(app.project!)
              .then((r) => app.notify(`${r.length} run(s)`))
              .catch((e) => app.notify(String(e)))
              .finally(() => (setBusy(false), app.refresh()));
          }}
        >
          <Play className="h-3.5 w-3.5" /> Run a round
        </button>
      </PageHeader>
      {tab === "board" && <Board project={app.project} />}
      {tab === "plan" && <Plan project={app.project} />}
      {tab === "inbox" && <Inbox project={app.project} />}
    </div>
  );
}

function Board({ project }: { project: string }) {
  const app = useApp();
  const [cols, setCols] = useState<Column[] | null>(null);
  useEffect(() => {
    api.board(project).then(setCols);
  }, [project, app.tick]);
  if (!cols) return <div className="text-xs text-subtle">{app.t("common.loading")}</div>;
  const shown = cols.filter((c) => c.cards.length || !["Cancelled", "Reopened"].includes(c.status));
  return (
    <div className="flex gap-3 overflow-x-auto pb-2">
      {shown.map((c) => (
        <div key={c.status} className="flex min-w-[220px] flex-1 flex-col rounded-xl border border-line bg-surface/60">
          <div className="flex items-center justify-between border-b border-line px-3 py-2">
            <TicketStatusBadge status={c.status} />
            <Mono className="text-subtle">{c.cards.length}</Mono>
          </div>
          <div className="space-y-2 p-2">
            {c.cards.map((k) => (
              <button key={k.key} onClick={() => app.go("ticket", k.key)} className="w-full rounded-lg border border-line bg-surface p-2.5 text-left shadow-sm transition-colors hover:border-accent/50">
                <div className="flex items-center justify-between">
                  <Mono className="font-semibold text-cyan-600 dark:text-cyan-400">{k.key}</Mono>
                  <PriorityBadge p={k.priority} />
                </div>
                <div className="mt-1.5 line-clamp-2 text-xs font-medium">{k.title}</div>
                <div className="mt-2 flex flex-wrap items-center gap-1.5 text-[10px] text-subtle">
                  <span className="chip border-line-strong bg-elevated">{k.plan_item}</span>
                  {k.assignee && <span className="truncate">@{k.assignee}</span>}
                </div>
                {(k.blocked_by.length > 0 || k.open_questions > 0 || k.approved || k.merged || k.escalated || k.reopen_count > 0) && (
                  <div className="mt-2 flex flex-wrap gap-1">
                    {k.blocked_by.length > 0 && (
                      <span className="chip border-amber-500/30 bg-amber-500/10 text-amber-600 dark:text-amber-300">
                        <Link2 className="h-3 w-3" /> {k.blocked_by.join(",")}
                      </span>
                    )}
                    {k.open_questions > 0 && (
                      <span className="chip border-blue-500/30 bg-blue-500/10 text-blue-600 dark:text-blue-300">
                        <MessageCircleQuestion className="h-3 w-3" /> {k.open_questions}
                      </span>
                    )}
                    {k.approved && <span className="chip border-purple-500/30 bg-purple-500/10 text-purple-600 dark:text-purple-300">APPROVED</span>}
                    {k.merged && <span className="chip border-emerald-500/30 bg-emerald-500/10 text-emerald-600 dark:text-emerald-300">MERGED</span>}
                    {k.reopen_count > 0 && <span className="chip border-rose-500/30 bg-rose-500/10 text-rose-600 dark:text-rose-300">REOPENED ×{k.reopen_count}</span>}
                    {k.escalated && <span className="chip border-rose-500/40 bg-rose-500/15 text-rose-600 dark:text-rose-300">ESCALATED</span>}
                  </div>
                )}
              </button>
            ))}
            {c.cards.length === 0 && <div className="py-6 text-center text-[11px] text-subtle">–</div>}
          </div>
        </div>
      ))}
    </div>
  );
}

function Plan({ project }: { project: string }) {
  const app = useApp();
  const [pv, setPv] = useState<PlanView | null>(null);
  useEffect(() => {
    api.plan(project).then(setPv);
  }, [project, app.tick]);
  if (!pv) return <div className="text-xs text-subtle">{app.t("common.loading")}</div>;
  const approve = (what: "spec" | "plan", v: number) =>
    api
      .approve(project, what, String(v))
      .then(() => (app.notify("✓"), app.refresh()))
      .catch((e) => app.notify(String(e)));
  return (
    <div className="grid gap-5 xl:grid-cols-5">
      <Section className="xl:col-span-2" title={pv.spec ? `Spec v${pv.spec.version}` : "Spec"} right={pv.spec && !pv.spec.locked_at && <button className="btn-primary" onClick={() => approve("spec", pv.spec!.version)}><Check className="h-3.5 w-3.5" /> {app.t("common.approve")}</button>}>
        {pv.spec ? (
          <div className="space-y-3 text-xs">
            <div className="text-[11px] text-subtle">{pv.spec.locked_at ? `approved by ${pv.spec.approved_by}` : "waits for your approval"}</div>
            {pv.spec.body.goals.map((g, i) => (
              <div key={i} className="font-medium">
                {g}
              </div>
            ))}
            <div className="space-y-1.5">
              {pv.spec.body.requirements.map((r) => (
                <div key={r.id} className="flex gap-2">
                  <Mono className="text-cyan-600 dark:text-cyan-400">{r.id}</Mono>
                  <span className="text-muted">{r.text}</span>
                </div>
              ))}
            </div>
          </div>
        ) : (
          <Empty>No spec yet: tell the Lead what you need.</Empty>
        )}
      </Section>
      <Section className="xl:col-span-3" title={pv.plan ? `Plan v${pv.plan.version}` : "Plan"} right={pv.plan && !pv.plan.approved_by && <button className="btn-primary" onClick={() => approve("plan", pv.plan!.version)}><Check className="h-3.5 w-3.5" /> {app.t("common.approve")}</button>}>
        {pv.plan ? (
          <div className="space-y-4">
            {pv.plan.reason && <div className="rounded-md border border-line bg-elevated p-2 text-xs text-muted">revision: {pv.plan.reason}</div>}
            {pv.plan.body.milestones.map((m) => {
              const items = pv.items.filter((i) => i.milestone === m.id);
              const done = items.filter((i) => i.done).length;
              return (
                <div key={m.id}>
                  <div className="mb-1.5 flex items-center justify-between">
                    <span className="text-xs font-semibold">
                      <Mono className="text-subtle">{m.id}</Mono> {m.title}
                    </span>
                    <Mono className="text-subtle">
                      {done}/{items.length}
                    </Mono>
                  </div>
                  <div className="space-y-1">
                    {items.map((i) => (
                      <div key={i.id} className="flex items-center gap-2 rounded-md px-2 py-1.5 hover:bg-elevated/60">
                        {i.done ? <CircleCheck className="h-4 w-4 text-emerald-500" /> : <Circle className="h-4 w-4 text-subtle" />}
                        <Mono className="w-12 text-subtle">{i.id}</Mono>
                        <span className="flex-1 text-xs">{i.title}</span>
                        <span className="flex gap-1">
                          {i.tickets.map(([k, s]) => (
                            <button key={k} onClick={() => app.go("ticket", k)}>
                              <TicketStatusBadge status={s} />
                            </button>
                          ))}
                        </span>
                      </div>
                    ))}
                  </div>
                </div>
              );
            })}
          </div>
        ) : (
          <Empty>No plan yet.</Empty>
        )}
      </Section>
    </div>
  );
}

function Inbox({ project }: { project: string }) {
  const app = useApp();
  const [items, setItems] = useState<InboxItem[] | null>(null);
  useEffect(() => {
    api.inbox(project).then(setItems);
  }, [project, app.tick]);
  if (!items) return <div className="text-xs text-subtle">{app.t("common.loading")}</div>;
  return (
    <Section title={app.t("tickets.inbox")}>
      <div className="space-y-2">
        {items.map((i, n) => (
          <div key={n} className="flex items-start gap-3 rounded-lg border border-line bg-elevated/50 p-3">
            <span className="chip border-line-strong bg-surface text-muted">{i.kind}</span>
            <div className="flex-1 whitespace-pre-wrap text-xs">{i.text}</div>
            {i.ticket && (
              <button className="btn" onClick={() => app.go("ticket", i.ticket!)}>
                {i.ticket}
              </button>
            )}
          </div>
        ))}
        {items.length === 0 && <Empty>The Lead's inbox is empty.</Empty>}
      </div>
    </Section>
  );
}
