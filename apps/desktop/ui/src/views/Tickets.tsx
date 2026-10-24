import { Check, Circle, CircleCheck, Link2, MessageCircleQuestion, Play } from "lucide-react";
import { useEffect, useState } from "react";
import { Markdown } from "../components/Markdown";
import { ListItem, Modal, ProgressBar, Segmented } from "../components/motion";
import { Empty, Mono, PageHeader, PriorityBadge, Section, TicketStatusBadge, cn } from "../components/ui";
import { AnimatePresence } from "motion/react";
import { useRef } from "react";
import { api } from "../lib/api";
import { useApp } from "../lib/store";
import type { AgentEntry, Card, Column, InboxItem, PlanView, TicketStatus } from "../lib/types";

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
        <Segmented
          id="tickets-tab"
          value={tab}
          onChange={(x) => app.go("tickets", x)}
          items={(["board", "plan", "inbox"] as Tab[]).map((x) => ({
            value: x,
            label: (
              <>
                {app.t(x === "board" ? "tickets.board" : x === "plan" ? "tickets.plan" : "tickets.inbox")}
                {x === "inbox" && p.inbox > 0 && <span className="ml-1 font-mono text-amber-500">{p.inbox}</span>}
              </>
            ),
          }))}
        />
        <button
          className="btn"
          disabled={busy}
          title={app.t("tickets.roundHint")}
          onClick={() => {
            setBusy(true);
            api
              .runRound(app.project!)
              .then((r) => app.notify(`${r.length} ${app.t("tickets.roundDone")}`))
              .catch((e) => app.notify(String(e)))
              .finally(() => (setBusy(false), app.refresh()));
          }}
        >
          <Play className="h-3.5 w-3.5" /> {app.t("tickets.round")}
        </button>
      </PageHeader>
      {tab === "board" && <Board project={app.project} />}
      {tab === "plan" && <Plan project={app.project} />}
      {tab === "inbox" && <Inbox project={app.project} />}
    </div>
  );
}

/** What dropping a card on a column means for the user (the state machine's User column). */
type Drop = { kind: "assign" | "close" | "cancel"; card: Card } | null;

function Board({ project }: { project: string }) {
  const app = useApp();
  const [cols, setCols] = useState<Column[] | null>(null);
  const [drag, setDrag] = useState<{ card: Card; from: TicketStatus } | null>(null);
  const [over, setOver] = useState<TicketStatus | null>(null);
  const [drop, setDrop] = useState<Drop>(null);
  const [agents, setAgents] = useState<AgentEntry[]>([]);
  const [value, setValue] = useState("");
  // The dialog keeps its last content while it leaves.
  const last = useRef<Drop>(null);
  if (drop) last.current = drop;
  const dlg = drop ?? last.current;
  useEffect(() => {
    api.board(project).then(setCols);
  }, [project, app.tick]);
  useEffect(() => {
    api.agents().then((a) => setAgents(a.filter((x) => x.project === project && !x.is_planner && (x.solo || !x.is_lead))));
  }, [project]);
  if (!cols) return <div className="text-xs text-subtle">{app.t("common.loading")}</div>;

  const allowed = (from: TicketStatus, to: TicketStatus): "assign" | "close" | "cancel" | "reopen" | null => {
    if (to === "Assigned" && ["New", "Assigned", "Reopened"].includes(from)) return "assign";
    if (to === "Closed" && from === "Resolved") return "close";
    if (to === "Cancelled" && !["Closed", "Cancelled"].includes(from)) return "cancel";
    if (to === "Reopened" && from === "Resolved") return "reopen";
    return null;
  };
  const onDrop = (to: TicketStatus) => {
    setOver(null);
    if (!drag || drag.from === to) return;
    const a = allowed(drag.from, to);
    if (a === "reopen") {
      app.notify(app.t("tickets.reopenHint"));
      app.go("ticket", drag.card.key);
    } else if (a) {
      setValue("");
      setDrop({ kind: a, card: drag.card });
    } else app.notify(app.t("tickets.dropNo"));
    setDrag(null);
  };
  const confirm = () => {
    if (!drop) return;
    const k = drop.card.key;
    const p =
      drop.kind === "assign"
        ? api.ticketAction(project, k, "assign", { agent: value })
        : api.ticketAction(project, k, drop.kind, { reason: value });
    p.then(() => (app.notify(app.t("common.done")), app.refresh()))
      .catch((e) => app.notify(String(e)))
      .finally(() => setDrop(null));
  };
  const shown = cols.filter((c) => c.cards.length || !["Cancelled", "Reopened"].includes(c.status) || drag);
  return (
    <>
      <div className="mb-2 text-[11px] text-subtle">{app.t("tickets.dropHint")}</div>
      <div className="flex gap-3 overflow-x-auto pb-2">
        {shown.map((c) => (
          <div
            key={c.status}
            onDragOver={(e) => (e.preventDefault(), setOver(c.status))}
            onDragLeave={() => setOver((o) => (o === c.status ? null : o))}
            onDrop={() => onDrop(c.status)}
            className={cn(
              "flex flex-col rounded-xl border bg-surface/60 transition-all",
              c.cards.length || drag ? "min-w-[220px] flex-1" : "w-[132px] shrink-0",
              over === c.status && drag ? (allowed(drag.from, c.status) ? "border-accent bg-blue-600/5" : "border-rose-500/50") : "border-line",
            )}
          >
            <div className="flex items-center justify-between border-b border-line px-3 py-2">
              <TicketStatusBadge status={c.status} />
              <Mono className="text-subtle">{c.cards.length}</Mono>
            </div>
            <div className="min-h-[80px] space-y-2 p-2">
              <AnimatePresence initial={false} mode="popLayout">
              {c.cards.map((k) => (
                <ListItem key={k.key} id={k.key}>
                <div
                  draggable
                  onDragStart={() => setDrag({ card: k, from: c.status })}
                  onDragEnd={() => (setDrag(null), setOver(null))}
                  onClick={() => app.go("ticket", k.key)}
                  className={cn("press cursor-grab rounded-lg border border-line bg-surface p-2.5 shadow-sm hover:border-accent/50 hover:shadow-md active:cursor-grabbing", drag?.card.key === k.key && "opacity-40")}
                >
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
                </div>
                </ListItem>
              ))}
              </AnimatePresence>
            </div>
          </div>
        ))}
      </div>
      <Modal
        open={!!drop}
        title={dlg ? `${dlg.card.key} → ${dlg.kind === "assign" ? app.t("tickets.assignTo") : dlg.kind === "close" ? app.t("ticket.close") : app.t("ticket.cancel")}` : ""}
        onClose={() => setDrop(null)}
      >
        {dlg && (
        <>
          <div className="mb-3 text-xs text-muted">{dlg.card.title}</div>
          {dlg.kind === "assign" ? (
            <select className="input" value={value} onChange={(e) => setValue(e.target.value)} autoFocus>
              <option value="">–</option>
              {agents.map((a) => (
                <option key={a.agent.id} value={a.agent.id}>
                  {a.agent.name} ({a.agent.role})
                </option>
              ))}
            </select>
          ) : (
            <input className="input" placeholder={app.t("common.reason")} value={value} onChange={(e) => setValue(e.target.value)} autoFocus />
          )}
          <div className="mt-4 flex justify-end gap-2">
            <button className="btn" onClick={() => setDrop(null)}>
              {app.t("common.cancel")}
            </button>
            <button className={dlg.kind === "cancel" ? "btn-danger" : "btn-primary"} disabled={dlg.kind === "assign" ? !value : value.trim().length < 5} onClick={confirm}>
              <Check className="h-3.5 w-3.5" /> OK
            </button>
          </div>
        </>
        )}
      </Modal>
    </>
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
      .then(() => (app.notify(app.t("common.done")), app.refresh()))
      .catch((e) => app.notify(String(e)));
  const who = (s?: string | null) => (s?.startsWith("agent:") ? "Lead" : s);
  return (
    <div className="grid gap-5 xl:grid-cols-5">
      <Section
        className="xl:col-span-2"
        title={pv.spec ? `Spec v${pv.spec.version}` : "Spec"}
        right={
          pv.spec &&
          !pv.spec.locked_at && (
            <button className="btn-primary" onClick={() => approve("spec", pv.spec!.version)}>
              <Check className="h-3.5 w-3.5" /> {app.t("common.approve")}
            </button>
          )
        }
      >
        {pv.spec ? (
          <div className="space-y-3 text-xs">
            <div className="text-[11px] text-subtle">{pv.spec.locked_at ? `${app.t("plan.approvedBy")} ${who(pv.spec.approved_by)}` : app.t("plan.waits")}</div>
            {pv.spec.body.goals.map((g, i) => (
              <div key={i} className="font-medium">
                {g}
              </div>
            ))}
            <div className="space-y-1.5">
              {pv.spec.body.requirements.map((r) => (
                <div key={r.id} className="flex gap-2">
                  <Mono className="shrink-0 whitespace-nowrap text-cyan-600 dark:text-cyan-400">{r.id}</Mono>
                  <span className="text-muted">{r.text}</span>
                </div>
              ))}
            </div>
          </div>
        ) : (
          <Empty>{app.t("plan.noSpec")}</Empty>
        )}
      </Section>
      <Section
        className="xl:col-span-3"
        title={pv.plan ? `Plan v${pv.plan.version}` : "Plan"}
        right={
          pv.plan &&
          !pv.plan.approved_by && (
            <button className="btn-primary" onClick={() => approve("plan", pv.plan!.version)}>
              <Check className="h-3.5 w-3.5" /> {app.t("common.approve")}
            </button>
          )
        }
      >
        {pv.plan ? (
          <div className="space-y-4">
            <div className="text-[11px] text-subtle">{pv.plan.approved_by ? `${app.t("plan.approvedBy")} ${who(pv.plan.approved_by)}` : app.t("plan.waits")}</div>
            {pv.plan.reason && (
              <div className="rounded-md border border-line bg-elevated p-2 text-xs text-muted">
                {app.t("plan.revision")}: {pv.plan.reason}
              </div>
            )}
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
                  <div className="mb-2 h-1 overflow-hidden rounded-full bg-background">
                    <ProgressBar value={items.length ? done / items.length : 0} />
                  </div>
                  <div className="space-y-1">
                    {items.map((i) => (
                      <div key={i.id} className="flex items-center gap-2 rounded-md px-2 py-1.5 hover:bg-elevated/60">
                        {i.done ? <CircleCheck className="h-4 w-4 text-emerald-500" /> : <Circle className="h-4 w-4 text-subtle" />}
                        <Mono className="w-12 text-subtle">{i.id}</Mono>
                        <span className="flex-1 text-xs">{i.title}</span>
                        <span className="flex gap-1">
                          {i.tickets.map(([k, s]) => (
                            <button key={k} onClick={() => app.go("ticket", k)} title={k}>
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
          <Empty>{app.t("plan.noPlan")}</Empty>
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
            <Markdown className="flex-1 text-xs" text={i.text} />
            {i.ticket && (
              <button className="btn" onClick={() => app.go("ticket", i.ticket!)}>
                {i.ticket}
              </button>
            )}
          </div>
        ))}
        {items.length === 0 && <Empty>{app.t("inbox.empty")}</Empty>}
      </div>
    </Section>
  );
}
