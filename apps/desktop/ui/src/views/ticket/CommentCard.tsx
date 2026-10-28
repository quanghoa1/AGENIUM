import { CheckCircle2, Crown, HelpCircle, Package, RotateCcw, Send, ShieldAlert, Sparkles } from "lucide-react";
import { useState } from "react";
import { Markdown } from "../../components/Markdown";
import { Avatar, Mono, ago, cn } from "../../components/ui";
import { api } from "../../lib/api";
import { useApp } from "../../lib/store";
import type { CommentRow } from "../../lib/types";

/* ─── STRUCTURED COMMENT CARDS (Worker Report / Lead Review / Q&A Thread / Compact System Notice) ─── */

/** The tone of an acceptance result: met, partly met, not met. */
export function acTone(status: string): string {
  return status === "Met"
    ? "border-emerald-500/30 bg-emerald-500/10 text-emerald-600 dark:text-emerald-300"
    : status === "NotMet"
      ? "border-rose-500/30 bg-rose-500/10 text-rose-600 dark:text-rose-300"
      : "border-amber-500/30 bg-amber-500/10 text-amber-600 dark:text-amber-300";
}

export function CommentCard({
  c,
  replies,
  assigneeName,
  roleInfo,
  project,
  ticket,
}: {
  c: CommentRow;
  replies: CommentRow[];
  assigneeName: string | null;
  roleInfo?: { role: string; isLead: boolean; isSolo: boolean };
  project: string;
  ticket: string;
}) {
  const app = useApp();
  const [ans, setAns] = useState("");
  const d = c.body.data ?? {};
  const isAssignee = assigneeName && c.author === assigneeName;
  const isLead = roleInfo?.isLead || c.kind === "Review";

  // 1. Compact 1-line inline event for System / Progress updates (prevents card bloat)
  if (c.body.type === "System" || c.kind === "System" || c.kind === "Progress") {
    return (
      <div className="flex items-start gap-2.5 px-3 py-1.5 text-xs text-subtle">
        <span className="mt-1.5 h-2 w-2 shrink-0 rounded-full bg-line-strong" />
        <Mono className="shrink-0 font-semibold text-muted">{c.author}</Mono>
        <div className="min-w-0 flex-1">
          <Markdown className="text-xs text-muted" text={d.text ?? d.current ?? d.summary ?? JSON.stringify(d)} />
        </div>
        <Mono className="shrink-0 text-subtle">{ago(c.created_at)}</Mono>
      </div>
    );
  }

  // 2. Structured Worker Report Card (`WorkReport`)
  if (c.body.type === "Report") {
    const confidencePct = typeof d.confidence === "number" ? Math.round(d.confidence * 100) : null;
    return (
      <div className="card overflow-hidden border-l-4 border-l-blue-500">
        {/* Card Header */}
        <div className="flex flex-wrap items-center justify-between gap-2 border-b border-line bg-blue-500/5 px-4 py-2.5">
          <div className="flex flex-wrap items-center gap-2">
            <Avatar name={c.author} className="h-6 w-6 text-[10px]" />
            <span className="text-xs font-bold text-fg">{c.author}</span>
            <span className="chip border-blue-500/30 bg-blue-500/10 text-blue-600 dark:text-blue-300">
              {isAssignee ? app.t("ticket.assignee") : "Worker"}
            </span>
            <span className="chip border-cyan-500/30 bg-cyan-500/10 text-cyan-600 dark:text-cyan-300">
              <Package className="h-3 w-3" /> {app.t("ticket.workReport")}
            </span>
          </div>
          <div className="flex items-center gap-2">
            {confidencePct !== null && (
              <span
                className={cn(
                  "chip font-mono",
                  confidencePct >= 80
                    ? "border-emerald-500/30 bg-emerald-500/10 text-emerald-600 dark:text-emerald-300"
                    : "border-amber-500/30 bg-amber-500/10 text-amber-600 dark:text-amber-300",
                )}
              >
                {app.t("ticket.confidence")} {confidencePct}%
              </span>
            )}
            <Mono className="text-subtle">{ago(c.created_at)}</Mono>
          </div>
        </div>

        {/* Card Body: Distinct Labeled Fields */}
        <div className="space-y-4 p-4 text-xs">
          {/* Field 1: Summary */}
          {d.summary && (
            <div>
              <div className="label mb-1.5">{app.t("ticket.summary")}</div>
              <div className="rounded-lg border border-line bg-background/50 p-3">
                <Markdown className="text-xs leading-relaxed text-fg" text={d.summary} />
              </div>
            </div>
          )}

          {/* Field 2: Acceptance Criteria Table */}
          {Array.isArray(d.acceptance) && d.acceptance.length > 0 && (
            <div>
              <div className="label mb-1.5">{app.t("ticket.acCheck")}</div>
              <div className="overflow-hidden rounded-lg border border-line">
                <table className="w-full text-left text-xs">
                  <thead>
                    <tr className="border-b border-line bg-elevated/60 font-mono text-[10px] uppercase text-subtle">
                      <th className="w-20 px-3 py-2">AC</th>
                      <th className="w-28 px-3 py-2">{app.t("ticket.status")}</th>
                      <th className="px-3 py-2">{app.t("ticket.evidence")}</th>
                    </tr>
                  </thead>
                  <tbody className="divide-y divide-line bg-background/30">
                    {d.acceptance.map((a: { ac_id: string; status: string; evidence: string }) => (
                      <tr key={a.ac_id}>
                        <td className="px-3 py-2 font-mono font-bold text-cyan-600 dark:text-cyan-400">
                          {a.ac_id}
                        </td>
                        <td className="px-3 py-2">
                          <span className={cn("chip", acTone(a.status))}>{a.status}</span>
                        </td>
                        <td className="px-3 py-2">
                          <Markdown className="text-xs text-muted" text={a.evidence ?? ""} />
                        </td>
                      </tr>
                    ))}
                  </tbody>
                </table>
              </div>
            </div>
          )}

          {/* Field 3: Risks & Follow-ups */}
          {(d.risks?.length > 0 || d.follow_ups?.length > 0) && (
            <div className="grid gap-3 sm:grid-cols-2">
              {d.risks?.length > 0 && (
                <div className="rounded-lg border border-amber-500/30 bg-amber-500/5 p-3">
                  <div className="font-mono text-[10px] font-bold uppercase tracking-wider text-amber-600 dark:text-amber-400">
                    {app.t("ticket.risks")}
                  </div>
                  <ul className="mt-1.5 list-disc space-y-1 pl-4 text-xs text-muted">
                    {d.risks.map((r: string, i: number) => (
                      <li key={i}>{r}</li>
                    ))}
                  </ul>
                </div>
              )}
              {d.follow_ups?.length > 0 && (
                <div className="rounded-lg border border-line bg-background/50 p-3">
                  <div className="label">{app.t("ticket.followUps")}</div>
                  <ul className="mt-1.5 list-disc space-y-1 pl-4 text-xs text-muted">
                    {d.follow_ups.map((f: string, i: number) => (
                      <li key={i}>{f}</li>
                    ))}
                  </ul>
                </div>
              )}
            </div>
          )}
        </div>
      </div>
    );
  }

  // 3. Structured Lead Review Card (`ReviewDecision`: Approve or Reopen)
  if (c.body.type === "Review") {
    const approved = d.decision === "Approve";
    return (
      <div
        className={cn(
          "card overflow-hidden border-l-4",
          approved ? "border-l-emerald-500" : "border-l-rose-500",
        )}
      >
        {/* Review Header */}
        <div
          className={cn(
            "flex flex-wrap items-center justify-between gap-2 border-b border-line px-4 py-2.5",
            approved ? "bg-emerald-500/10" : "bg-rose-500/10",
          )}
        >
          <div className="flex flex-wrap items-center gap-2">
            <Avatar name={c.author} className="h-6 w-6 text-[10px]" />
            <span className="text-xs font-bold text-fg">{c.author}</span>
            <span className="chip border-purple-500/30 bg-purple-500/10 text-purple-600 dark:text-purple-300">
              {c.author === "user" ? (
                app.t("ticket.reviewByYou")
              ) : roleInfo?.isSolo ? (
                app.t("ticket.reviewBySolo")
              ) : (
                <>
                  <Crown className="h-3 w-3" /> {app.t("ticket.reviewByLead")}
                </>
              )}
            </span>
            <span
              className={cn(
                "chip font-bold",
                approved
                  ? "border-emerald-500/40 bg-emerald-500/15 text-emerald-600 dark:text-emerald-300"
                  : "border-rose-500/40 bg-rose-500/15 text-rose-600 dark:text-rose-300",
              )}
            >
              {approved ? (
                <>
                  <CheckCircle2 className="h-3 w-3" /> {app.t("ticket.verdictApprove")}
                </>
              ) : (
                <>
                  <RotateCcw className="h-3 w-3" /> {app.t("ticket.verdictReopen")}
                </>
              )}
            </span>
          </div>
          <Mono className="text-subtle">{ago(c.created_at)}</Mono>
        </div>

        {/* Review Body */}
        <div className="space-y-3 p-4 text-xs">
          {Array.isArray(d.ac_feedback) && d.ac_feedback.length > 0 && (
            <div>
              <div className="label mb-1.5">{app.t("ticket.acFeedback")}</div>
              <div className="divide-y divide-line overflow-hidden rounded-lg border border-line bg-background/40">
                {d.ac_feedback.map((f: { ac_id: string; feedback: string }) => (
                  <div key={f.ac_id} className="flex items-start gap-3 p-2.5">
                    <Mono className="w-14 shrink-0 font-bold text-cyan-600 dark:text-cyan-400">
                      {f.ac_id}
                    </Mono>
                    <Markdown className="min-w-0 flex-1 text-xs text-fg" text={f.feedback ?? ""} />
                  </div>
                ))}
              </div>
            </div>
          )}

          {Array.isArray(d.required_changes) && d.required_changes.length > 0 && (
            <div className="rounded-lg border border-rose-500/30 bg-rose-500/5 p-3">
              <div className="font-mono text-[10px] font-bold uppercase tracking-wider text-rose-600 dark:text-rose-300">
                {app.t("ticket.requiredChanges")}
              </div>
              <ul className="mt-1.5 list-disc space-y-1 pl-4 text-xs text-fg">
                {d.required_changes.map((rc: string, i: number) => (
                  <li key={i}>{rc}</li>
                ))}
              </ul>
            </div>
          )}
        </div>
      </div>
    );
  }

  // 4. Structured Question Card (with embedded Answer thread)
  if (c.body.type === "Question") {
    return (
      <div className="card overflow-hidden border-l-4 border-l-amber-500">
        <div className="flex flex-wrap items-center justify-between gap-2 border-b border-line bg-amber-500/5 px-4 py-2.5">
          <div className="flex flex-wrap items-center gap-2">
            <Avatar name={c.author} className="h-6 w-6 text-[10px]" />
            <span className="text-xs font-bold text-fg">{c.author}</span>
            <span className="chip border-amber-500/30 bg-amber-500/10 text-amber-600 dark:text-amber-300">
              <HelpCircle className="h-3 w-3" /> {app.t("ticket.question")}
            </span>
            <span
              className={cn(
                "chip",
                c.resolved
                  ? "border-emerald-500/30 bg-emerald-500/10 text-emerald-600 dark:text-emerald-300"
                  : "border-amber-500/40 bg-amber-500/15 text-amber-600 dark:text-amber-300",
              )}
            >
              {c.resolved ? app.t("ticket.answered") : app.t("ticket.open")}
            </span>
          </div>
          <Mono className="text-subtle">{ago(c.created_at)}</Mono>
        </div>

        <div className="space-y-3 p-4 text-xs">
          <div className="font-medium text-fg">{d.question}</div>
          {d.context && (
            <div className="rounded-lg border border-line bg-background/50 p-2.5">
              <Markdown className="text-[11px] text-muted" text={d.context} />
            </div>
          )}

          {d.options?.length > 0 && (
            <div className="flex flex-wrap gap-1.5">
              {d.options.map((o: { label: string }, i: number) => (
                <button
                  key={i}
                  disabled={c.resolved}
                  className={cn("btn", d.recommendation === i && "border-accent/60")}
                  onClick={() => api.answer(project, ticket, c.id, o.label, i).then((r) => (r?.routed_to === "lead" && app.notify(app.t("needs.toLead")), app.refresh()))}
                >
                  {d.recommendation === i && <Sparkles className="h-3 w-3 text-cyan-400" />}
                  {o.label}
                </button>
              ))}
            </div>
          )}

          {/* Embedded Reply Thread (`Answer` linked via `reply_to`) */}
          {replies.length > 0 && (
            <div className="space-y-2 border-l-2 border-emerald-500/50 pl-3 pt-1">
              {replies.map((r) => (
                <div
                  key={r.id}
                  className="rounded-lg border border-emerald-500/25 bg-emerald-500/5 p-3 text-xs"
                >
                  <div className="mb-1 flex items-center gap-2">
                    <span className="chip border-emerald-500/30 bg-emerald-500/10 text-emerald-600 dark:text-emerald-300">
                      ↳ {app.t("ticket.answer")}
                    </span>
                    <span className="font-semibold text-fg">{r.author}</span>
                    <Mono className="ml-auto text-subtle">{ago(r.created_at)}</Mono>
                  </div>
                  <Markdown className="text-xs text-fg" text={r.body.data?.resolution ?? ""} />
                </div>
              ))}
            </div>
          )}

          {!c.resolved && (
            <div className="flex gap-2 pt-1">
              <input
                className="input"
                placeholder={app.t("ticket.answerAs")}
                value={ans}
                onChange={(e) => setAns(e.target.value)}
              />
              <button
                className="btn-primary shrink-0"
                disabled={!ans.trim()}
                onClick={() => api.answer(project, ticket, c.id, ans).then((r) => (r?.routed_to === "lead" && app.notify(app.t("needs.toLead")), app.refresh()))}
              >
                <Send className="h-3.5 w-3.5" />
              </button>
            </div>
          )}
        </div>
      </div>
    );
  }

  // 5. Permission Request Card
  if (c.body.type === "Permission") {
    return (
      <div className="card overflow-hidden border-l-4 border-l-amber-500">
        <div className="flex flex-wrap items-center justify-between gap-2 border-b border-line bg-amber-500/5 px-4 py-2.5">
          <div className="flex items-center gap-2">
            <Avatar name={c.author} className="h-6 w-6 text-[10px]" />
            <span className="text-xs font-bold text-fg">{c.author}</span>
            <span className="chip border-amber-500/30 bg-amber-500/10 text-amber-600 dark:text-amber-300">
              <ShieldAlert className="h-3 w-3" /> {app.t("ticket.permission")}
            </span>
          </div>
          <Mono className="text-subtle">{ago(c.created_at)}</Mono>
        </div>
        <div className="space-y-2 p-4 text-xs">
          <div className="font-medium text-fg">
            {d.request?.kind}: {d.request?.reason}
          </div>
          <div className="rounded border border-line bg-background/60 px-2.5 py-1.5 font-mono text-[11px] text-muted">
            {[d.request?.command, ...(d.request?.domains ?? []), ...(d.request?.paths ?? [])]
              .filter(Boolean)
              .join(" · ")}
          </div>
          <div className="text-[11px] text-subtle">
            {d.outcome
              ? `${d.outcome.decision} · ${d.outcome.decided_by} (${d.outcome.scope})`
              : app.t("ticket.open")}
          </div>
        </div>
      </div>
    );
  }

  // 6. Standard Comment Card (User / Standalone Answer / Handoff / Text)
  return (
    <div className="card overflow-hidden border-l-4 border-l-cyan-500/60">
      <div className="flex flex-wrap items-center justify-between gap-2 border-b border-line bg-elevated/30 px-4 py-2">
        <div className="flex items-center gap-2">
          <Avatar name={c.author} className="h-6 w-6 text-[10px]" />
          <span className="text-xs font-bold text-fg">{c.author}</span>
          <span className="chip border-line-strong bg-elevated text-muted">
            {isLead ? (
              <>
                <Crown className="h-3 w-3 text-amber-500" /> Lead
              </>
            ) : isAssignee ? (
              app.t("ticket.assignee")
            ) : (
              c.kind
            )}
          </span>
        </div>
        <Mono className="text-subtle">{ago(c.created_at)}</Mono>
      </div>
      <div className="p-4 text-xs">
        {c.body.type === "Answer" ? (
          <Markdown className="text-xs text-fg" text={d.resolution} />
        ) : c.body.type === "Text" ? (
          <Markdown className="text-xs text-fg" text={d.text} />
        ) : (
          <pre className="whitespace-pre-wrap font-mono text-[11px] text-muted">
            {JSON.stringify(d, null, 2)}
          </pre>
        )}
      </div>
    </div>
  );
}
