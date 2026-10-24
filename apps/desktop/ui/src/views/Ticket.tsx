import { ArrowLeft, Check, GitBranch, Send, X } from "lucide-react";
import { useEffect, useState, type ReactNode } from "react";
import { DiffView } from "../components/DiffView";
import { Markdown } from "../components/Markdown";
import { Empty, Mono, OutcomeBadge, PriorityBadge, Section, TicketStatusBadge, ago, cn, tokens } from "../components/ui";
import { api } from "../lib/api";
import { useApp } from "../lib/store";
import type { AgentEntry, CommentRow, TicketDetail } from "../lib/types";

type Tab = "timeline" | "changes" | "runs" | "audit";

export function TicketView({ id }: { id: string }) {
  const app = useApp();
  const [d, setD] = useState<TicketDetail | null>(null);
  const [tab, setTab] = useState<Tab>("timeline");
  const [diff, setDiff] = useState<{ diff: string; truncated?: boolean; note?: string } | null>(null);
  const [agents, setAgents] = useState<AgentEntry[]>([]);
  const [text, setText] = useState("");
  const [reason, setReason] = useState("");
  const [feedback, setFeedback] = useState<Record<string, string>>({});
  useEffect(() => {
    if (app.project) api.ticket(app.project, id).then(setD).catch((e) => app.notify(String(e)));
  }, [app.project, id, app.tick]);
  useEffect(() => {
    if (tab === "changes" && app.project) api.ticketDiff(app.project, id).then(setDiff).catch((e) => app.notify(String(e)));
  }, [tab, app.project, id, app.tick]);
  useEffect(() => {
    api.agents().then(setAgents);
  }, []);
  if (!d || !app.project) return <div className="text-xs text-subtle">{app.t("common.loading")}</div>;
  const t = d.ticket;
  const project = app.project;
  const act = (p: Promise<unknown>) => p.then(() => (app.notify(app.t("common.done")), app.refresh())).catch((e) => app.notify(String(e)));
  const workers = agents.filter((a) => a.project === project && !a.is_planner && (a.solo || !a.is_lead));
  const tabs: [Tab, string, number | undefined][] = [
    ["timeline", app.t("ticket.timeline"), d.comments.length],
    ["changes", app.t("ticket.changes"), undefined],
    ["runs", app.t("ticket.runs"), d.runs.length],
    ["audit", app.t("ticket.audit"), d.audit.length],
  ];

  return (
    <div className="space-y-4">
      <button className="btn" onClick={() => app.go("tickets")}>
        <ArrowLeft className="h-3.5 w-3.5" /> {app.t("nav.tickets")}
      </button>
      <div className="card p-5">
        <div className="flex flex-wrap items-center gap-2">
          <Mono className="text-sm font-semibold text-cyan-600 dark:text-cyan-400">{t.key}</Mono>
          <TicketStatusBadge status={t.status} />
          <PriorityBadge p={t.priority} />
          <span className="chip border-line-strong bg-elevated text-muted">{t.kind}</span>
          <span className="chip border-line-strong bg-elevated text-muted">{t.plan_item}</span>
          {t.approved && <span className="chip border-purple-500/30 bg-purple-500/10 text-purple-600 dark:text-purple-300">APPROVED</span>}
          {t.merged && <span className="chip border-emerald-500/30 bg-emerald-500/10 text-emerald-600 dark:text-emerald-300">MERGED</span>}
          {t.escalated && <span className="chip border-rose-500/40 bg-rose-500/15 text-rose-600 dark:text-rose-300">ESCALATED</span>}
        </div>
        <h1 className="mt-2 text-lg font-semibold">{t.title}</h1>
        <Markdown className="mt-1 text-xs text-muted" text={t.description} />
        <div className="mt-3 flex flex-wrap gap-x-6 gap-y-1 text-[11px] text-subtle">
          <span>
            {app.t("ticket.assignee")}: <span className="text-fg">{d.assignee_name ?? "–"}</span>
          </span>
          <span>
            {app.t("ticket.report")}: {t.report_template}
          </span>
          {t.depends_on.length > 0 && (
            <span>
              {app.t("ticket.dependsOn")}:{" "}
              {t.depends_on.map((k) => (
                <button key={k} className="mr-1 font-mono text-cyan-600 hover:underline dark:text-cyan-400" onClick={() => app.go("ticket", k)}>
                  {k}
                </button>
              ))}
            </span>
          )}
          {d.workspace && (
            <span className="inline-flex items-center gap-1">
              <GitBranch className="h-3 w-3" /> {d.workspace.branch}
            </span>
          )}
          <span>
            {app.t("ticket.tokens")}: {tokens(t.usage.tokens_in + t.usage.tokens_out)} · ${t.usage.estimated_cost_usd.toFixed(3)}
          </span>
        </div>
      </div>

      <div className="grid gap-4 xl:grid-cols-3">
        <div className="min-w-0 space-y-4 xl:col-span-2">
          <div className="flex rounded-lg border border-line-strong bg-elevated p-0.5 text-xs">
            {tabs.map(([x, label, n]) => (
              <button key={x} onClick={() => setTab(x)} className={cn("flex-1 rounded-md px-3 py-1", tab === x ? "bg-surface font-medium shadow-sm" : "text-subtle")}>
                {label}
                {n !== undefined && <Mono className="ml-1 text-subtle">{n}</Mono>}
              </button>
            ))}
          </div>
          {tab === "timeline" && (
            <div className="space-y-2">
              {d.comments.map((c) => (
                <CommentCard key={c.id} c={c} project={project} ticket={t.key} />
              ))}
              {d.comments.length === 0 && <Empty>{app.t("common.none")}</Empty>}
              <div className="card flex gap-2 p-3">
                <input className="input" placeholder={app.t("ticket.comment")} value={text} onChange={(e) => setText(e.target.value)} />
                <button className="btn-primary" disabled={!text.trim()} onClick={() => act(api.comment(project, t.key, text).then(() => setText("")))}>
                  <Send className="h-3.5 w-3.5" />
                </button>
              </div>
            </div>
          )}
          {tab === "changes" && (diff ? <DiffView diff={diff.diff} /> : <div className="text-xs text-subtle">{app.t("common.loading")}</div>)}
          {tab === "runs" && (
            <div className="card divide-y divide-line">
              {d.runs.map((r) => (
                <button key={r.id} onClick={() => app.go("run", r.id)} className="flex w-full items-center gap-3 px-4 py-2.5 text-left text-xs hover:bg-elevated/60">
                  <OutcomeBadge outcome={r.outcome} />
                  <span className="font-medium">{r.agent_name}</span>
                  <span className="text-muted">{r.model}</span>
                  <Mono className="ml-auto text-subtle">
                    {r.steps} · {tokens(r.usage.tokens_in + r.usage.tokens_out)} · {ago(r.started_at)}
                  </Mono>
                </button>
              ))}
              {d.runs.length === 0 && (
                <div className="p-4">
                  <Empty>{app.t("common.none")}</Empty>
                </div>
              )}
            </div>
          )}
          {tab === "audit" && (
            <div className="card overflow-x-auto">
              <table className="w-full text-xs">
                <tbody className="divide-y divide-line">
                  {d.audit.map((e, i) => (
                    <tr key={i}>
                      <td className="whitespace-nowrap px-3 py-2 font-mono text-[11px] text-subtle">{e.at.replace("T", " ").slice(0, 19)}</td>
                      <td className="px-3 font-medium">{e.actor}</td>
                      <td className="px-3">{e.action}</td>
                      <td className="px-3">{e.accepted ? <span className="text-emerald-600 dark:text-emerald-400">{app.t("ticket.applied")}</span> : <span className="text-rose-600 dark:text-rose-400">{app.t("ticket.refused")}</span>}</td>
                      <td className="max-w-[360px] truncate px-3 font-mono text-[10px] text-subtle" title={JSON.stringify(e.detail)}>
                        {JSON.stringify(e.detail)}
                      </td>
                    </tr>
                  ))}
                </tbody>
              </table>
            </div>
          )}
        </div>

        <div className="space-y-4">
          <Section title={app.t("ticket.acceptance")}>
            <div className="space-y-2 text-xs">
              {t.acceptance.map((a) => (
                <div key={a.id}>
                  <div className="flex gap-2">
                    <Mono className="shrink-0 whitespace-nowrap text-cyan-600 dark:text-cyan-400">{a.id}</Mono>
                    <span>{a.text}</span>
                  </div>
                  {t.status === "Resolved" && <input className="input mt-1" placeholder={app.t("ticket.feedback")} value={feedback[a.id] ?? ""} onChange={(e) => setFeedback({ ...feedback, [a.id]: e.target.value })} />}
                </div>
              ))}
            </div>
          </Section>
          {t.scope_paths.length > 0 && (
            <Section title={app.t("ticket.scope")}>
              <div className="flex flex-wrap gap-1">
                {t.scope_paths.map((s) => (
                  <span key={s} className="chip border-line-strong bg-elevated text-muted">
                    {s}
                  </span>
                ))}
              </div>
            </Section>
          )}
          <Section title={app.t("ticket.override")}>
            <div className="space-y-3 text-xs">
              {t.status === "Resolved" && (
                <div className="flex gap-2">
                  <button className="btn-primary flex-1 justify-center" onClick={() => act(api.ticketAction(project, t.key, "approve"))}>
                    <Check className="h-3.5 w-3.5" /> {app.t("common.approve")}
                  </button>
                  <button
                    className="btn-danger flex-1 justify-center"
                    disabled={!Object.values(feedback).some((v) => v.trim())}
                    onClick={() =>
                      act(
                        api.ticketAction(project, t.key, "reopen", {
                          feedback: Object.entries(feedback)
                            .filter(([, v]) => v.trim())
                            .map(([ac_id, f]) => ({ ac_id, feedback: f })),
                        }),
                      )
                    }
                  >
                    <X className="h-3.5 w-3.5" /> {app.t("ticket.reopen")}
                  </button>
                </div>
              )}
              <label className="block">
                <span className="label">{app.t("ticket.priority")}</span>
                <select className="input mt-1" value={t.priority} disabled={["Closed", "Cancelled"].includes(t.status)} onChange={(e) => act(api.ticketAction(project, t.key, "priority", { priority: Number(e.target.value) }))}>
                  {[0, 1, 2, 3].map((p) => (
                    <option key={p} value={p}>
                      P{p}
                    </option>
                  ))}
                </select>
              </label>
              {["New", "Assigned", "Reopened"].includes(t.status) && (
                <label className="block">
                  <span className="label">{app.t("ticket.assignee")}</span>
                  <select className="input mt-1" value={t.assignee ?? ""} onChange={(e) => e.target.value && act(api.ticketAction(project, t.key, "assign", { agent: e.target.value }))}>
                    <option value="">–</option>
                    {workers.map((w) => (
                      <option key={w.agent.id} value={w.agent.id}>
                        {w.agent.name} ({w.agent.role})
                      </option>
                    ))}
                  </select>
                </label>
              )}
              {!["Closed", "Cancelled"].includes(t.status) && (
                <div className="space-y-2">
                  <input className="input" placeholder={app.t("common.reason")} value={reason} onChange={(e) => setReason(e.target.value)} />
                  <div className="flex gap-2">
                    {t.status === "Resolved" && (
                      <button className="btn flex-1 justify-center" disabled={reason.trim().length < 5} onClick={() => act(api.ticketAction(project, t.key, "close", { reason }))}>
                        {app.t("ticket.close")}
                      </button>
                    )}
                    <button className="btn-danger flex-1 justify-center" disabled={reason.trim().length < 5} onClick={() => act(api.ticketAction(project, t.key, "cancel", { reason }))}>
                      {app.t("ticket.cancel")}
                    </button>
                  </div>
                </div>
              )}
            </div>
          </Section>
        </div>
      </div>
    </div>
  );
}

const kindTone: Record<string, string> = {
  Question: "border-l-blue-500/60",
  Answer: "border-l-emerald-500/60",
  Report: "border-l-purple-500/60",
  Review: "border-l-purple-500/60",
  Permission: "border-l-amber-500/60",
  System: "border-l-line-strong",
  User: "border-l-cyan-500/60",
  Handoff: "border-l-cyan-500/60",
  Progress: "border-l-line-strong",
};

function CommentCard({ c, project, ticket }: { c: CommentRow; project: string; ticket: string }) {
  const app = useApp();
  const [ans, setAns] = useState("");
  const d = c.body.data ?? {};
  let content: ReactNode;
  switch (c.body.type) {
    case "Question":
      content = (
        <div>
          <div className="text-xs font-medium">{d.question}</div>
          {d.context && <Markdown className="mt-1 text-[11px] text-muted" text={d.context} />}
          {d.options?.length > 0 && (
            <div className="mt-2 flex flex-wrap gap-1.5">
              {d.options.map((o: { label: string }, i: number) => (
                <button key={i} disabled={c.resolved} className={cn("btn", d.recommendation === i && "border-accent/60")} onClick={() => api.answer(project, ticket, c.id, o.label, i).then(app.refresh)}>
                  {o.label}
                </button>
              ))}
            </div>
          )}
          {!c.resolved && (
            <div className="mt-2 flex gap-2">
              <input className="input" placeholder={app.t("ticket.answerAs")} value={ans} onChange={(e) => setAns(e.target.value)} />
              <button className="btn-primary" disabled={!ans.trim()} onClick={() => api.answer(project, ticket, c.id, ans).then(app.refresh)}>
                <Send className="h-3.5 w-3.5" />
              </button>
            </div>
          )}
        </div>
      );
      break;
    case "Answer":
      content = <Markdown className="text-xs" text={d.resolution} />;
      break;
    case "Report":
      content = (
        <div className="space-y-2 text-xs">
          <Markdown text={d.summary} />
          <div className="space-y-1">
            {(d.acceptance ?? []).map((a: { ac_id: string; status: string; evidence: string }) => (
              <div key={a.ac_id} className="flex gap-2">
                <Mono className="shrink-0 whitespace-nowrap text-cyan-600 dark:text-cyan-400">{a.ac_id}</Mono>
                <span className={cn("chip", a.status === "Met" ? "border-emerald-500/30 text-emerald-600 dark:text-emerald-300" : "border-amber-500/30 text-amber-600 dark:text-amber-300")}>{a.status}</span>
                <span className="text-muted">{a.evidence}</span>
              </div>
            ))}
          </div>
          {(d.risks?.length > 0 || d.follow_ups?.length > 0) && (
            <div className="text-[11px] text-subtle">
              {d.risks?.length > 0 && <div>risks: {d.risks.join("; ")}</div>}
              {d.follow_ups?.length > 0 && <div>follow-ups: {d.follow_ups.join("; ")}</div>}
            </div>
          )}
        </div>
      );
      break;
    case "Review":
      content = (
        <div className="text-xs">
          <span className="font-semibold">{d.decision}</span>
          {(d.ac_feedback ?? []).map((f: { ac_id: string; feedback: string }) => (
            <div key={f.ac_id} className="mt-1 flex gap-2">
              <Mono className="shrink-0 whitespace-nowrap text-cyan-600 dark:text-cyan-400">{f.ac_id}</Mono>
              <span className="text-muted">{f.feedback}</span>
            </div>
          ))}
        </div>
      );
      break;
    case "Permission":
      content = (
        <div className="text-xs">
          <div>
            {d.request?.kind}: {d.request?.reason}
          </div>
          <div className="mt-1 font-mono text-[11px] text-muted">{[d.request?.command, ...(d.request?.domains ?? []), ...(d.request?.paths ?? [])].filter(Boolean).join(" · ")}</div>
          <div className="mt-1 text-[11px]">{d.outcome ? `${d.outcome.decision} · ${d.outcome.decided_by} (${d.outcome.scope})` : app.t("ticket.open")}</div>
        </div>
      );
      break;
    case "System":
    case "Text":
      content = <Markdown className="text-xs text-muted" text={d.text} />;
      break;
    default:
      content = <pre className="whitespace-pre-wrap font-mono text-[11px] text-muted">{JSON.stringify(d, null, 2)}</pre>;
  }
  return (
    <div className={cn("card border-l-4 p-3", kindTone[c.kind] ?? "border-l-line")}>
      <div className="mb-1.5 flex items-center gap-2">
        <span className="label">{c.kind}</span>
        <span className="text-[11px] font-medium">{c.author}</span>
        {c.kind === "Question" && <span className={cn("chip", c.resolved ? "border-emerald-500/30 text-emerald-600" : "border-amber-500/30 text-amber-600")}>{c.resolved ? app.t("ticket.answered") : app.t("ticket.open")}</span>}
        <Mono className="ml-auto text-subtle">{ago(c.created_at)}</Mono>
      </div>
      {content}
    </div>
  );
}
