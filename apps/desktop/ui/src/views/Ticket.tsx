import { AlertTriangle, Check, FileCode2, GitBranch, Lock, MessageSquare, MessageSquarePlus, RotateCcw, Send, X } from "lucide-react";
import { useEffect, useMemo, useState, type ReactNode } from "react";
import { AnimatePresence } from "motion/react";
import { DiffView } from "../components/DiffView";
import { ListItem, Overlay, PanelSwap, Segmented } from "../components/motion";
import { Markdown } from "../components/Markdown";
import { Avatar, Empty, Mono, OutcomeBadge, PriorityBadge, Section, TicketStatusBadge, ago, cn, tokens } from "../components/ui";
import { api } from "../lib/api";
import type { Key } from "../lib/i18n";
import { useApp } from "../lib/store";
import { askLead } from "./Chat";
import type { AgentEntry, CommentRow, TicketDetail } from "../lib/types";
import { CommentCard, acTone } from "./ticket/CommentCard";

type Tab = "timeline" | "changes" | "runs" | "audit";

/** A link's kind read from this ticket's side: "B follows up A" shows on A as "followed up by B". */
function linkLabel(t: (k: Key) => string, kind: string, out: boolean): string {
  const known = ["aggregates", "blocks", "follow_up_of", "duplicates", "related_to"];
  if (!known.includes(kind)) return kind.replace(/_/g, " ");
  return kind === "related_to" ? t("ticket.link.related_to") : t(`ticket.link.${kind}.${out ? "out" : "in"}` as Key);
}

/** The ticket open in the app (`app.ticket`), in a popup over the current view: Esc, a click outside or Close
 * closes it, a link to another ticket shows that one instead. */
export function TicketPopup() {
  const { ticket, openTicket } = useApp();
  useEffect(() => {
    if (!ticket) return;
    const onKey = (e: KeyboardEvent) => e.key === "Escape" && openTicket(null);
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [ticket, openTicket]);
  return (
    <Overlay open={!!ticket} onClose={() => openTicket(null)} kind="dialog" top="pt-[4vh]">
      <div role="dialog" aria-modal="true" aria-label={ticket?.key} className="card max-h-[90vh] w-[min(1100px,94vw)] overflow-y-auto p-5 shadow-2xl">
        {ticket && <TicketView key={`${ticket.project}/${ticket.key}`} id={ticket.key} project={ticket.project} onClose={() => openTicket(null)} />}
      </div>
    </Overlay>
  );
}

function TicketView({ id, project, onClose }: { id: string; project: string; onClose: () => void }) {
  const app = useApp();
  const [d, setD] = useState<TicketDetail | null>(null);
  const [tab, setTab] = useState<Tab>("timeline");
  const [dir, setDir] = useState(1);
  const order: Tab[] = ["timeline", "changes", "runs", "audit"];
  const [diff, setDiff] = useState<{ diff: string; truncated?: boolean; note?: string } | null>(null);
  const [agents, setAgents] = useState<AgentEntry[]>([]);
  const [text, setText] = useState("");
  const [sending, setSending] = useState(false);
  const [reason, setReason] = useState("");
  const [feedback, setFeedback] = useState<Record<string, string>>({});

  useEffect(() => {
    api.ticket(project, id).then(setD).catch((e) => app.notify(String(e)));
  }, [project, id, app.tick]);

  useEffect(() => {
    if (tab === "changes") api.ticketDiff(project, id).then(setDiff).catch((e) => app.notify(String(e)));
  }, [tab, project, id, app.tick]);

  useEffect(() => {
    api.agents().then(setAgents);
  }, []);

  // Group Answer comments under their parent Question (via `reply_to`) so Q&A threads stay together
  const { rootComments, repliesByParent } = useMemo(() => {
    const comments = d?.comments ?? [];
    const byId = new Set(comments.map((c) => c.id));
    const map: Record<string, CommentRow[]> = {};
    const roots: CommentRow[] = [];
    for (const c of comments) {
      if (c.kind === "Answer" && c.reply_to && byId.has(c.reply_to)) {
        (map[c.reply_to] ??= []).push(c);
      } else {
        roots.push(c);
      }
    }
    return { rootComments: roots, repliesByParent: map };
  }, [d?.comments]);

  // Build map of agent names -> role info (Lead vs Worker vs Solo) for clear comment badges
  const agentRoleByName = useMemo(() => {
    const m: Record<string, { role: string; isLead: boolean; isSolo: boolean }> = {};
    for (const a of agents) {
      m[a.agent.name] = {
        role: a.agent.role,
        isLead: a.is_lead || a.agent.role_kind === "lead",
        isSolo: a.solo,
      };
    }
    return m;
  }, [agents]);

  if (!d) return <div className="text-xs text-subtle">{app.t("common.loading")}</div>;
  const t = d.ticket;
  const act = (p: Promise<unknown>) =>
    p.then(() => (app.notify(app.t("common.done")), app.refresh())).catch((e) => app.notify(String(e)));
  const workers = agents.filter((a) => a.project === project && !a.is_planner && (a.solo || !a.is_lead));
  // One comment per send: the box empties at once (text typed meanwhile is kept) and gets the text back on failure.
  const send = () => {
    const msg = text.trim();
    if (!msg || sending) return;
    setSending(true);
    setText("");
    api
      .comment(project, t.key, msg)
      .then(() => app.refresh())
      .catch((e) => (setText((cur) => cur || msg), app.notify(String(e))))
      .finally(() => setSending(false));
  };

  const tabs: [Tab, string, number | undefined][] = [
    ["timeline", app.t("ticket.timeline"), d.comments.length],
    ["changes", app.t("ticket.changes"), undefined],
    ["runs", app.t("ticket.runs"), d.runs.length],
    ["audit", app.t("ticket.audit"), d.audit.length],
  ];

  // Latest AC verification status from the most recent Worker Report (if any)
  const latestAcStatus: Record<string, { status: string; evidence: string }> = {};
  for (const c of d.comments) {
    if (c.body.type === "Report" && Array.isArray(c.body.data?.acceptance)) {
      for (const a of c.body.data.acceptance) {
        if (a?.ac_id) latestAcStatus[a.ac_id] = { status: a.status, evidence: a.evidence };
      }
    }
  }

  return (
    <div className="space-y-5">
      {/* ─── TOP BAR: Breadcrumb Navigation & Ticket Status Strip ─── */}
      <div className="flex flex-wrap items-center justify-between gap-3 border-b border-line pb-3.5">
        <div className="flex flex-wrap items-center gap-2 text-xs">
          <button className="btn" onClick={onClose} title="Esc">
            <X className="h-3.5 w-3.5" /> {app.t("common.close")}
          </button>
          <span className="text-subtle">/</span>
          <Mono className="text-muted">{t.plan_item}</Mono>
          <span className="text-subtle">/</span>
          <Mono className="rounded border border-cyan-500/30 bg-cyan-500/10 px-2 py-0.5 text-xs font-bold text-cyan-600 dark:text-cyan-400">
            {t.key}
          </Mono>
          <span className="chip border-line-strong bg-elevated text-muted">{t.kind}</span>
        </div>

        <div className="flex flex-wrap items-center gap-2">
          {t.status === "Closed" && (
            // A closed ticket is never reopened: more work on it is a new ticket that follows it (ADR-59).
            <button
              className="btn"
              title={app.t("ticket.closedNote")}
              onClick={() => (
                // The ticket's own project's Lead: the popup may show another project's ticket.
                app.setProject(project),
                askLead(app.go, app.t("ticket.followUpDraft").replace("{k}", t.key), app.t("ticket.followUpNote").split("{k}").join(t.key))
              )}
            >
              <MessageSquarePlus className="h-3.5 w-3.5" /> {app.t("ticket.followUp")}
            </button>
          )}
          <TicketStatusBadge status={t.status} />
          {t.reopen_count > 0 && (
            <span className="chip border-amber-500/30 bg-amber-500/10 text-amber-600 dark:text-amber-300">
              <RotateCcw className="h-3 w-3" /> REOPENED ×{t.reopen_count}
            </span>
          )}
          {t.approved && (
            <span className="chip border-purple-500/30 bg-purple-500/10 text-purple-600 dark:text-purple-300">
              <Check className="h-3 w-3" /> APPROVED
            </span>
          )}
          {t.merged && (
            <span className="chip border-emerald-500/30 bg-emerald-500/10 text-emerald-600 dark:text-emerald-300">
              MERGED
            </span>
          )}
          {t.escalated && (
            <span className="chip border-rose-500/40 bg-rose-500/15 text-rose-600 dark:text-rose-300">
              <AlertTriangle className="h-3 w-3" /> ESCALATED
            </span>
          )}
        </div>
      </div>

      {/* ─── MAIN 2-COLUMN JIRA LAYOUT (Left 8 Cols: Specification + Activity | Right 4 Cols: Details Sidebar) ─── */}
      <div className="grid items-start gap-5 xl:grid-cols-12">
        {/* ======================== LEFT COLUMN (8/12) ======================== */}
        <div className="min-w-0 space-y-5 xl:col-span-8">
          {/* 1. TICKET SPECIFICATION BOX (Title + Description + Acceptance Criteria + Scope Paths) */}
          <section className="card overflow-hidden">
            <div className="flex flex-wrap items-center justify-between gap-2 border-b border-line bg-elevated/40 px-5 py-3.5">
              <h1 className="text-lg font-bold tracking-tight text-fg">{t.title}</h1>
              <span className="inline-flex items-center gap-1 font-mono text-[10px] uppercase tracking-wider text-subtle">
                <Lock className="h-3 w-3" /> {app.t("ticket.spec")}
              </span>
            </div>

            <div className="space-y-5 p-5">
              {/* Field: Description */}
              <div>
                <div className="label mb-2">{app.t("ticket.description")}</div>
                <div className="rounded-lg border border-line bg-background/60 p-3.5">
                  <Markdown className="text-xs leading-relaxed text-fg" text={t.description} />
                </div>
              </div>

              {/* Field: Acceptance Criteria Table */}
              {t.acceptance.length > 0 && (
                <div>
                  <div className="mb-2 flex items-center justify-between">
                    <span className="label">
                      {app.t("ticket.acceptance")} ({t.acceptance.length})
                    </span>
                    <span className="text-[11px] text-subtle">{app.t("ticket.acCheckedBy")}</span>
                  </div>
                  <div className="divide-y divide-line overflow-hidden rounded-lg border border-line bg-background/40">
                    {t.acceptance.map((a) => {
                      const reported = latestAcStatus[a.id];
                      return (
                        <div key={a.id} className="p-3 text-xs">
                          <div className="flex items-start gap-3">
                            <Mono className="mt-0.5 w-14 shrink-0 font-bold text-cyan-600 dark:text-cyan-400">
                              {a.id}
                            </Mono>
                            <div className="min-w-0 flex-1 leading-relaxed text-fg">{a.text}</div>
                            {reported && <span className={cn("chip shrink-0", acTone(reported.status))}>{reported.status}</span>}
                          </div>
                          {t.status === "Resolved" && (
                            <div className="mt-2 pl-[68px]">
                              <input
                                className="input"
                                placeholder={`${a.id}: ${app.t("ticket.feedback")}`}
                                value={feedback[a.id] ?? ""}
                                onChange={(e) => setFeedback({ ...feedback, [a.id]: e.target.value })}
                              />
                            </div>
                          )}
                        </div>
                      );
                    })}
                  </div>
                </div>
              )}

              {/* Field: Write Scope Paths */}
              {t.scope_paths.length > 0 && (
                <div>
                  <div className="label mb-2">{app.t("ticket.scope")}</div>
                  <div className="flex flex-wrap gap-1.5">
                    {t.scope_paths.map((s) => (
                      <span
                        key={s}
                        className="inline-flex items-center gap-1.5 rounded-md border border-line-strong bg-elevated px-2.5 py-1 font-mono text-[11px] text-muted"
                      >
                        <FileCode2 className="h-3 w-3 text-cyan-500" />
                        {s}
                      </span>
                    ))}
                  </div>
                </div>
              )}
            </div>
          </section>

          {/* 2. ACTIVITY SECTION (Timeline / Changes / Runs / Audit) */}
          <div className="space-y-3">
            <div className="flex flex-wrap items-center justify-between gap-2">
              <div className="label flex items-center gap-1.5">
                <MessageSquare className="h-3.5 w-3.5 text-cyan-500" />
                {app.t("ticket.activity")}
              </div>
              <Segmented
                id="ticket-tab"
                value={tab}
                onChange={(x) => (setDir(order.indexOf(x) >= order.indexOf(tab) ? 1 : -1), setTab(x))}
                items={tabs.map(([x, label, n]) => ({
                  value: x,
                  label: (
                    <>
                      {label}
                      {n !== undefined && <Mono className="text-subtle">{n}</Mono>}
                    </>
                  ),
                }))}
              />
            </div>

            <PanelSwap id={tab} dir={dir}>
              {tab === "timeline" && (
                <div className="space-y-3">
                  <AnimatePresence initial={false} mode="popLayout">
                    {rootComments.map((c) => (
                      <ListItem key={c.id} id={c.id}>
                        <CommentCard
                          c={c}
                          replies={repliesByParent[c.id] ?? []}
                          assigneeName={d.assignee_name ?? null}
                          roleInfo={agentRoleByName[c.author]}
                          project={project}
                          ticket={t.key}
                        />
                      </ListItem>
                    ))}
                  </AnimatePresence>

                  {d.comments.length === 0 && <Empty>{app.t("common.none")}</Empty>}

                  {/* Comment Composer */}
                  <div className="card flex items-center gap-2 p-3">
                    <input
                      className="input"
                      placeholder={app.t("ticket.comment")}
                      value={text}
                      onChange={(e) => setText(e.target.value)}
                      onKeyDown={(e) => {
                        if (e.key === "Enter" && !e.nativeEvent.isComposing) send();
                      }}
                    />
                    <button className="btn-primary shrink-0" disabled={!text.trim() || sending} onClick={send}>
                      <Send className="h-3.5 w-3.5" /> {app.t("common.send")}
                    </button>
                  </div>
                </div>
              )}

              {tab === "changes" &&
                (diff ? (
                  <DiffView diff={diff.diff} load={(path) => api.ticketFile(project, id, path)} />
                ) : (
                  <div className="text-xs text-subtle">{app.t("common.loading")}</div>
                ))}

              {tab === "runs" && (
                <div className="card divide-y divide-line">
                  {d.runs.map((r) => (
                    <button
                      key={r.id}
                      onClick={() => app.go("run", r.id)}
                      className="flex w-full items-center gap-3 px-4 py-2.5 text-left text-xs hover:bg-elevated/60"
                    >
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
                    <thead>
                      <tr className="label border-b border-line text-left">
                        <th className="px-3 py-2">{app.t("ticket.auditAt")}</th>
                        <th className="px-3">{app.t("ticket.auditActor")}</th>
                        <th className="px-3">{app.t("ticket.auditAction")}</th>
                        <th className="px-3">{app.t("ticket.auditResult")}</th>
                        <th className="px-3">{app.t("ticket.auditDetail")}</th>
                      </tr>
                    </thead>
                    <tbody className="divide-y divide-line">
                      {d.audit.map((e, i) => (
                        <tr key={i}>
                          <td className="whitespace-nowrap px-3 py-2 font-mono text-[11px] text-subtle">
                            {e.at.replace("T", " ").slice(0, 19)}
                          </td>
                          <td className="px-3 font-medium">{e.actor}</td>
                          <td className="px-3">{e.action}</td>
                          <td className="px-3">
                            {e.accepted ? (
                              <span className="text-emerald-600 dark:text-emerald-400">
                                {app.t("ticket.applied")}
                              </span>
                            ) : (
                              <span className="text-rose-600 dark:text-rose-400">
                                {app.t("ticket.refused")}
                              </span>
                            )}
                          </td>
                          <td
                            className="max-w-[360px] truncate px-3 font-mono text-[11px] text-subtle"
                            title={JSON.stringify(e.detail)}
                          >
                            {JSON.stringify(e.detail)}
                          </td>
                        </tr>
                      ))}
                    </tbody>
                  </table>
                </div>
              )}
            </PanelSwap>
          </div>
        </div>

        {/* ======================== RIGHT COLUMN (4/12 — JIRA DETAILS SIDEBAR) ======================== */}
        <div className="space-y-4 xl:col-span-4">
          {/* Box 1: Jira Details Key-Value Table */}
          <Section title={app.t("ticket.details")}>
            <div className="divide-y divide-line text-xs">
              <DetailRow label={app.t("ticket.status")}>
                <TicketStatusBadge status={t.status} />
              </DetailRow>

              <DetailRow label={app.t("ticket.assignee")}>
                {d.assignee_name ? (
                  <div className="flex items-center gap-2">
                    <Avatar name={d.assignee_name} className="h-6 w-6 text-[10px]" />
                    <div className="min-w-0">
                      <div className="truncate font-semibold text-fg">{d.assignee_name}</div>
                      {agentRoleByName[d.assignee_name] && (
                        <div className="font-mono text-[10px] text-subtle">
                          {agentRoleByName[d.assignee_name].role}
                        </div>
                      )}
                    </div>
                  </div>
                ) : (
                  <span className="text-subtle">{app.t("ticket.unassigned")}</span>
                )}
              </DetailRow>

              <DetailRow label={app.t("ticket.priority")}>
                <PriorityBadge p={t.priority} />
              </DetailRow>

              <DetailRow label={app.t("ticket.kind")}>
                <span className="chip border-line-strong bg-elevated text-muted">{t.kind}</span>
              </DetailRow>

              <DetailRow label={app.t("ticket.planItem")}>
                <Mono className="text-fg">{t.plan_item}</Mono>
              </DetailRow>

              <DetailRow label={app.t("ticket.report")}>
                <Mono className="text-muted">{t.report_template}</Mono>
              </DetailRow>

              {t.depends_on.length > 0 && (
                <DetailRow label={app.t("ticket.dependsOn")}>
                  <div className="flex flex-wrap gap-1.5">
                    {t.depends_on.map((k) => (
                      <button
                        key={k}
                        className="chip border-cyan-500/30 bg-cyan-500/10 text-cyan-600 hover:underline dark:text-cyan-400"
                        onClick={() => app.openTicket(k, project)}
                      >
                        {k}
                      </button>
                    ))}
                  </div>
                </DetailRow>
              )}

              {(d.dependents ?? []).length > 0 && (
                <DetailRow label={app.t("ticket.dependents")}>
                  <div className="flex flex-wrap gap-1.5">
                    {d.dependents!.map((k) => (
                      <button
                        key={k}
                        className="chip border-line-strong bg-elevated text-muted hover:underline"
                        onClick={() => app.openTicket(k, project)}
                      >
                        {k}
                      </button>
                    ))}
                  </div>
                </DetailRow>
              )}

              {(d.links ?? []).length > 0 && (
                <DetailRow label={app.t("ticket.links")}>
                  <div className="flex flex-wrap gap-1.5">
                    {d.links!.map((l) => (
                      <button
                        key={`${l.kind}-${l.key}`}
                        className={cn(
                          "chip border-line-strong bg-elevated text-fg hover:border-accent/50",
                          !l.confirmed && "opacity-60",
                        )}
                        title={`${l.title}${l.note ? ` — ${l.note}` : ""} (${l.proposed_by}${
                          l.confirmed ? "" : `, ${app.t("ticket.linkUnconfirmed")}`
                        })`}
                        onClick={() => app.openTicket(l.key, project)}
                      >
                        <span className="text-subtle">{linkLabel(app.t, l.kind, l.out)}</span>
                        <span className="font-mono text-cyan-600 dark:text-cyan-400">{l.key}</span>
                        {!l.confirmed && "?"}
                      </button>
                    ))}
                  </div>
                </DetailRow>
              )}
            </div>
          </Section>

          {/* Box 2: Development / Git Worktree */}
          {d.workspace && (
            <Section title={app.t("ticket.development")}>
              <div className="divide-y divide-line text-xs">
                <DetailRow label={app.t("ticket.branch")}>
                  <span className="inline-flex items-center gap-1 font-mono text-[11px] text-cyan-600 dark:text-cyan-400">
                    <GitBranch className="h-3 w-3" /> {d.workspace.branch}
                  </span>
                </DetailRow>
                <DetailRow label={app.t("ticket.baseSha")}>
                  <Mono className="text-muted">{d.workspace.base_sha.slice(0, 8)}</Mono>
                </DetailRow>
                {d.workspace.head_sha && (
                  <DetailRow label={app.t("ticket.headSha")}>
                    <Mono className="text-fg">{d.workspace.head_sha.slice(0, 8)}</Mono>
                  </DetailRow>
                )}
                <DetailRow label={app.t("ticket.state")}>
                  {t.merged ? (
                    <span className="chip border-emerald-500/30 bg-emerald-500/10 text-emerald-600 dark:text-emerald-300">
                      MERGED
                    </span>
                  ) : t.approved ? (
                    <span className="chip border-purple-500/30 bg-purple-500/10 text-purple-600 dark:text-purple-300">
                      {app.t("ticket.readyToMerge")}
                    </span>
                  ) : (
                    <span className="chip border-line-strong bg-elevated text-muted">
                      {d.workspace.removed ? app.t("ticket.worktreeRemoved") : app.t("ticket.worktreeActive")}
                    </span>
                  )}
                </DetailRow>
              </div>
            </Section>
          )}

          {/* Box 3: Execution & Token Usage */}
          <Section title={app.t("ticket.usage")}>
            <div className="divide-y divide-line text-xs">
              <DetailRow label={app.t("ticket.tokens")}>
                <Mono className="text-fg">
                  {tokens(t.usage.tokens_in + t.usage.tokens_out)}{" "}
                  <span className="text-subtle">
                    ({tokens(t.usage.tokens_in)} in / {tokens(t.usage.tokens_out)} out)
                  </span>
                </Mono>
              </DetailRow>
              <DetailRow label={app.t("ticket.cost")}>
                <Mono className="font-semibold text-emerald-600 dark:text-emerald-400">
                  ${t.usage.estimated_cost_usd.toFixed(3)}
                </Mono>
              </DetailRow>
              <DetailRow label={app.t("ticket.runs")}>
                <Mono className="text-fg">{d.runs.length}</Mono>
              </DetailRow>
              <DetailRow label={app.t("ticket.updated")}>
                <Mono className="text-subtle">{ago(t.updated_at)}</Mono>
              </DetailRow>
            </div>
          </Section>

          {/* Box 4: User Override Controls */}
          <Section title={app.t("ticket.override")}>
            <div className="space-y-3 text-xs">
              {t.status === "Resolved" && (
                <div className="flex gap-2">
                  <button
                    className="btn-primary flex-1 justify-center"
                    onClick={() => act(api.ticketAction(project, t.key, "approve"))}
                  >
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
                <select
                  className="input mt-1"
                  value={t.priority}
                  disabled={["Closed", "Cancelled"].includes(t.status)}
                  onChange={(e) =>
                    act(api.ticketAction(project, t.key, "priority", { priority: Number(e.target.value) }))
                  }
                >
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
                  <select
                    className="input mt-1"
                    value={t.assignee ?? ""}
                    onChange={(e) =>
                      e.target.value && act(api.ticketAction(project, t.key, "assign", { agent: e.target.value }))
                    }
                  >
                    <option value="">–</option>
                    {workers.map((w) => (
                      <option key={w.agent.id} value={w.agent.id}>
                        {w.agent.name} ({w.agent.role})
                      </option>
                    ))}
                  </select>
                </label>
              )}

              {d.waivable && (
                <div className="space-y-2 rounded-lg border border-amber-500/30 bg-amber-500/5 p-3">
                  <div className="text-xs text-muted">{app.t("ticket.waiveHint")}</div>
                  <input
                    className="input"
                    placeholder={app.t("common.reason")}
                    value={reason}
                    onChange={(e) => setReason(e.target.value)}
                  />
                  <button
                    className="btn w-full justify-center"
                    onClick={() => act(api.ticketAction(project, t.key, "waive", { reason }))}
                  >
                    {app.t("ticket.waive")}
                  </button>
                </div>
              )}

              {!["Closed", "Cancelled"].includes(t.status) && (
                <div className="space-y-2">
                  <input
                    className="input"
                    placeholder={app.t("common.reason")}
                    value={reason}
                    onChange={(e) => setReason(e.target.value)}
                  />
                  <div className="flex gap-2">
                    {t.status === "Resolved" && (
                      <button
                        className="btn flex-1 justify-center"
                        disabled={reason.trim().length < 5}
                        onClick={() => act(api.ticketAction(project, t.key, "close", { reason }))}
                      >
                        {app.t("ticket.close")}
                      </button>
                    )}
                    <button
                      className="btn-danger flex-1 justify-center"
                      disabled={reason.trim().length < 5}
                      onClick={() => act(api.ticketAction(project, t.key, "cancel", { reason }))}
                    >
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

function DetailRow({ label, children }: { label: string; children: ReactNode }) {
  return (
    <div className="grid grid-cols-12 items-center gap-2 py-2.5 first:pt-0 last:pb-0">
      <div className="col-span-4 font-mono text-[11px] uppercase tracking-wider text-subtle">{label}</div>
      <div className="col-span-8 min-w-0">{children}</div>
    </div>
  );
}
