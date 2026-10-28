import { Bot, CircleCheck, CircleX, Crown, Inbox } from "lucide-react";
import { useEffect, useRef, useState } from "react";
import { LiveTurn } from "../../components/ChatPane";
import { Markdown } from "../../components/Markdown";
import { ListItem } from "../../components/motion";
import { Empty, Mono, TicketStatusBadge, ago, cn } from "../../components/ui";
import { api } from "../../lib/api";
import { useApp, type LiveRun as Stream } from "../../lib/store";
import type { BusItem, LiveAgent } from "../../lib/types";

function body(b: { type: string; data: any }): string {
  const d = b.data ?? {};
  if (typeof d === "string") return d;
  // A review: the decision, then the reviewer's word on each criterion and the changes it asks for.
  if (d.decision) {
    const lines = [`**${d.decision}**`];
    for (const f of d.ac_feedback ?? []) lines.push(`- \`${f.ac_id}\` ${f.feedback}`);
    for (const c of d.required_changes ?? []) lines.push(`- ${c}`);
    return lines.join("\n");
  }
  if (d.request) return `${d.request.kind ?? ""}: ${d.request.reason ?? ""}`;
  // Answer bodies carry `resolution`, progress notes `current`.
  return d.text ?? d.question ?? d.resolution ?? d.answer ?? d.summary ?? d.current ?? d.note ?? d.message ?? JSON.stringify(d, null, 2);
}

/** A worker's message bus (read-only): assignments from the Lead, questions, answers, reports, reviews. */
export function WorkerBus({ agent, lead, stream }: { agent: LiveAgent; lead?: string | null; stream?: Stream }) {
  const app = useApp();
  const [items, setItems] = useState<BusItem[]>([]);
  const end = useRef<HTMLDivElement>(null);
  useEffect(() => {
    api.agentBus(agent.agent.id).then((x) => setItems(x ?? []));
  }, [agent.agent.id, app.tick]);
  useEffect(() => {
    // Scroll this list only (scrollIntoView would scroll the page around it too).
    const el = end.current?.parentElement;
    if (el) el.scrollTop = el.scrollHeight;
  }, [items.length, stream?.thinking.length, stream?.text.length]);
  const kindTone: Record<string, string> = {
    Question: "border-amber-500/40 bg-amber-500/5",
    Answer: "border-purple-500/40 bg-purple-500/5",
    Report: "border-emerald-500/40 bg-emerald-500/5",
    Review: "border-purple-500/40 bg-purple-500/5",
    Permission: "border-rose-500/40 bg-rose-500/5",
    Progress: "border-blue-500/30 bg-blue-500/5",
    System: "border-line bg-elevated/40",
    Handoff: "border-cyan-500/40 bg-cyan-500/5",
    User: "border-accent/40 bg-blue-600/5",
  };
  return (
    <div className="min-h-0 flex-1 space-y-2.5 overflow-y-auto p-3">
      {items.map((it, i) =>
        it.type === "assignment" ? (
          <ListItem key={"a" + it.ticket + i} id={"a" + it.ticket}>
            <div className="rounded-xl border border-purple-500/40 bg-purple-500/5 p-3">
              <div className="flex flex-wrap items-center gap-1.5 text-[11px]">
                <Crown className="h-3.5 w-3.5 text-amber-500" />
                <b>{lead ?? "Lead"}</b>
                <span className="text-subtle">→</span>
                <b>{agent.agent.name}</b>
                <Mono className="text-cyan-600 dark:text-cyan-400">{it.ticket}</Mono>
                <TicketStatusBadge status={it.status} />
                <span className="ml-auto text-[11px] text-subtle">{ago(it.at)}</span>
              </div>
              <div className="mt-1.5 text-xs font-semibold">{it.title}</div>
              <div className="mt-1 line-clamp-4 text-[11px] text-muted">{it.description}</div>
              <ul className="mt-1.5 space-y-0.5">
                {it.acceptance.map((c) => (
                  <li key={c.id} className="text-[11px]">
                    <Mono className="text-subtle">{c.id}</Mono> {c.text}
                  </li>
                ))}
              </ul>
            </div>
          </ListItem>
        ) : (
          <ListItem key={it.id} id={it.id}>
            <div className={cn("rounded-xl border p-3", kindTone[it.kind] ?? "border-line", it.mine ? "ml-6" : "mr-6")}>
              <div className="flex flex-wrap items-center gap-1.5 text-[11px]">
                {it.author === lead ? <Crown className="h-3.5 w-3.5 text-amber-500" /> : it.mine ? <Bot className="h-3.5 w-3.5 text-muted" /> : <Inbox className="h-3.5 w-3.5 text-subtle" />}
                <b>{it.author}</b>
                <span className="chip border-line-strong text-muted">{it.kind}</span>
                <Mono className="text-cyan-600 dark:text-cyan-400">{it.ticket}</Mono>
                {it.resolved && <CircleCheck className="h-3 w-3 text-emerald-500" />}
                <span className="ml-auto text-[11px] text-subtle">{ago(it.at)}</span>
              </div>
              {it.kind === "Report" && Array.isArray(it.body.data?.acceptance) ? (
                <div className="mt-1.5 space-y-1">
                  {it.body.data.summary && <Markdown className="text-[11px]" text={it.body.data.summary} />}
                  {it.body.data.acceptance.map((c: { ac_id: string; status: string; evidence: string }) => (
                    <div key={c.ac_id} className="flex gap-1.5 text-[11px]">
                      {c.status === "Met" ? <CircleCheck className="mt-0.5 h-3 w-3 shrink-0 text-emerald-500" /> : <CircleX className="mt-0.5 h-3 w-3 shrink-0 text-rose-500" />}
                      <Mono className="text-subtle">{c.ac_id}</Mono>
                      <Markdown className="min-w-0 flex-1 text-muted" text={c.evidence ?? ""} />
                    </div>
                  ))}
                </div>
              ) : (
                <div className="mt-1.5 text-[11px]">
                  <Markdown text={body(it.body)} />
                </div>
              )}
            </div>
          </ListItem>
        ),
      )}
      {items.length === 0 && !agent.run && <Empty>{app.t("live.busEmpty")}</Empty>}
      {agent.run && <LiveTurn name={agent.agent.name} stream={stream} />}
      <div ref={end} />
    </div>
  );
}
