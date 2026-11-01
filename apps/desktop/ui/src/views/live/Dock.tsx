import { Bot, Crown, Lock, MessageSquareReply, Users } from "lucide-react";
import { ContextGauge } from "../../components/ContextGauge";
import { useEffect, useState } from "react";
import { ChatPane, type Prefill } from "../../components/ChatPane";
import { Avatar, CacheHit, Empty, Mono, OutcomeBadge, TicketStatusBadge, ago, cn, modelLabel, tokens } from "../../components/ui";
import { VendorBadge } from "../../components/VendorBadge";
import { api } from "../../lib/api";
import { useApp } from "../../lib/store";
import type { AgentTrace, LiveAgent, LiveData } from "../../lib/types";
import { Act, Dot, kindOf } from "./activity";
import { CallBadge, CallRow, JudgeRow } from "./calls";
import { Delegations } from "./Delegations";
import { WorkerBus } from "./WorkerBus";

/** The selected agent: who it is and what it works on, then its conversation (`view` agent) or its tool, skill and MCP
 *  calls (`view` tools), each with the whole panel. */
export function Dock({ a, data, act, prefill, onPick, onAskLead, view }: { a: LiveAgent; data: LiveData; act?: Act; prefill: Prefill | null; onPick: (id: string) => void; onAskLead: (w: LiveAgent) => void; view: "agent" | "tools" }) {
  const app = useApp();
  const k = kindOf(a);
  const team = data.teams.find((t) => t.id === a.team);
  const lead = team?.lead ? data.agents.find((x) => x.agent.id === team.lead) : undefined;
  const [trace, setTrace] = useState<AgentTrace | null>(null);
  const live = !!a.run;
  useEffect(() => {
    setTrace(null);
  }, [a.agent.id]);
  useEffect(() => {
    let stop = false;
    const load = () => api.agentTrace(a.agent.id).then((t) => !stop && setTrace(t));
    load();
    // While a run is live, poll its log (tool results land there); otherwise refresh on events.
    const h = live ? window.setInterval(load, 1500) : undefined;
    return () => {
      stop = true;
      if (h) window.clearInterval(h);
    };
  }, [a.agent.id, live, app.tick]);
  // The run's tool, skill and MCP calls, and what the Jev judge said for it (a skill pick, an evidence read).
  const calls = (trace?.steps ?? []).filter((e) => e.kind === "tool" || e.kind === "judge");
  const stream = act?.stream && !act.stream.done ? act.stream : undefined;
  const task = a.run?.ticket_key ? a.open_tickets.find((x) => x.key === a.run!.ticket_key) : a.open_tickets[0];
  const status: [string, string] =
    act?.activity === "tool" || act?.activity === "shell"
      ? [app.t("live.stTool"), "text-purple-600 dark:text-purple-300"]
      : act?.activity === "thinking"
        ? [app.t("live.stThinking"), "text-blue-600 dark:text-blue-300"]
        : act?.activity === "text"
          ? [app.t("live.stWriting"), "text-blue-600 dark:text-blue-300"]
          : act?.activity === "waiting"
            ? [app.t("live.stWaiting"), "text-amber-600 dark:text-amber-300"]
            : act?.activity === "off"
              ? [app.t("live.stOffline"), "text-rose-600 dark:text-rose-300"]
              : [app.t("live.stIdle"), "text-subtle"];
  return (
    <div className="flex min-h-0 flex-1 flex-col">
      {/* who and what */}
      <div className="border-b border-line px-4 py-3">
        <div className="flex items-center gap-2.5">
          <Avatar name={a.agent.name} className="h-9 w-9" />
          <div className="min-w-0 flex-1">
            <div className="flex items-center gap-1.5">
              {k === "lead" ? <Crown className="h-3.5 w-3.5 text-amber-500" /> : <Bot className="h-3.5 w-3.5 text-muted" />}
              <span className="truncate text-sm font-semibold">{a.agent.name}</span>
              {k === "worker" && <Lock className="h-3 w-3 text-subtle" />}
              <span className={cn("ml-1 flex items-center gap-1 font-mono text-[11px] font-semibold", status[1])}>
                <Dot a={act?.activity ?? "idle"} /> {status[0]}
              </span>
            </div>
            <div className="truncate text-[11px] text-subtle">
              {k === "lead" || k === "worker" ? (
                <>
                  {team?.name} ·{" "}
                  {k === "worker" && lead ? (
                    <button className="hover:text-accent" onClick={() => onPick(lead.agent.id)}>
                      Lead: 👑 {lead.agent.name}
                    </button>
                  ) : (
                    app.t("live.teamLead")
                  )}
                </>
              ) : k === "standalone" ? (
                `Solo · ${a.project_name ?? ""}`
              ) : (
                app.t("live.inPool")
              )}{" "}
              · {a.agent.role}
            </div>
          </div>
          <button className="btn px-2 py-1 text-[11px]" onClick={() => app.go("agent", a.agent.id)}>
            {app.t("live.profile")}
          </button>
        </div>
        <div className="mt-2 flex items-center gap-2 text-[11px]">
          <span className="label">{app.t("live.task")}</span>
          {task ? (
            <>
              <Mono className="text-cyan-600 dark:text-cyan-400">{task.key}</Mono>
              <span className="truncate">{task.title}</span>
              <TicketStatusBadge status={task.status} />
            </>
          ) : (
            <span className="text-subtle">{a.run?.kind === "lead" ? app.t("live.leading") : "–"}</span>
          )}
        </div>
        <div className="mt-1.5 flex flex-wrap items-center gap-1.5 text-[11px]">
          <span className="chip border-line-strong">
            <VendorBadge provider={a.agent.model.provider} model={a.agent.model.model} size={14} />
            <span className="text-subtle">{app.t("common.model")}</span>
            <b className="font-mono">
              {modelLabel(a.agent.backend, a.agent.model, app.t("model.default"))}
            </b>
          </span>
          <span className="chip border-line-strong">
            <span className="text-subtle">{app.t("live.effort")}</span>
            <b className="font-mono">{a.agent.model.effort || "–"}</b>
          </span>
        </div>
      </div>
      {view === "tools" ? (
        <div className="flex min-h-0 flex-1 flex-col px-4 py-2.5">
          {trace?.run && (
            <div className="mb-2 flex flex-wrap items-center gap-1.5 text-[11px]">
              <OutcomeBadge outcome={trace.run.outcome} />
              <Mono className="text-subtle">
                {trace.run.steps} step · {tokens(trace.run.usage.tokens_in + trace.run.usage.tokens_out)} tok · {ago(trace.run.started_at)}
              </Mono>
              <CacheHit value={trace.run.cache_hit} />
              <ContextGauge usage={trace.run.usage} />
            </div>
          )}
          {stream?.phase === "tool" && act?.call && (
            <div className="mb-1.5 flex items-center gap-2 rounded-lg border border-purple-500/40 bg-purple-500/5 px-2.5 py-1.5 text-[11px]">
              <span className="h-2 w-2 animate-ping rounded-full bg-purple-500" />
              <CallBadge kind={act.call.kind} />
              <span className="truncate font-mono">{act.call.label}</span>
            </div>
          )}
          <div className="min-h-0 flex-1 space-y-1 overflow-y-auto">
            {[...calls].reverse().map((e) => (e.kind === "judge" ? <JudgeRow key={e.n} e={e} /> : <CallRow key={e.n} e={e} />))}
            {calls.length === 0 && <div className="text-[11px] text-subtle">{trace?.run ? app.t("live.noCalls") : app.t("live.noRun")}</div>}
          </div>
        </div>
      ) : (
      // The conversation, by role, like a CLI chat (thinking folded in).
      <div className="relative flex min-h-0 flex-1 flex-col">
        {(k === "lead" || k === "standalone") && a.project ? (
            <>
              {k === "lead" && <Delegations project={a.project} data={data} onWatch={onPick} />}
              <ChatPane key={a.project} project={a.project} agentName={a.agent.name} prefill={k === "lead" ? prefill : null} stream={stream} className="border-t border-line" />
            </>
          ) : k === "worker" ? (
            <>
              <WorkerBus agent={a} lead={lead?.agent.name} stream={stream} />
              <div className="border-t border-line p-3">
                <div className="rounded-xl border border-amber-500/40 bg-amber-500/5 p-3">
                  <div className="flex items-start gap-3">
                    <Lock className="mt-0.5 h-5 w-5 shrink-0 text-amber-500" />
                    <div className="min-w-0 flex-1 text-xs">
                      <div className="font-semibold">{app.t("live.readOnly")}</div>
                      <div className="text-muted">
                        {app.t("live.readOnlyNote")} <b>👑 {lead?.agent.name ?? app.t("live.teamLead")}</b>.
                      </div>
                    </div>
                  </div>
                  <div className="mt-2 flex justify-end">
                    <button className="btn-primary" disabled={!lead} onClick={() => onAskLead(a)}>
                      <MessageSquareReply className="h-3.5 w-3.5" /> {app.t("live.askLead")} @{a.agent.name}
                    </button>
                  </div>
                </div>
              </div>
            </>
          ) : (
            <div className="p-4">
              <Empty>
                {k === "pool" ? app.t("live.poolNote") : app.t("live.noProject")}
                <div className="mt-3 flex justify-center">
                  <button className="btn" onClick={() => app.go("teams")}>
                    <Users className="h-3.5 w-3.5" /> {app.t("nav.teams")}
                  </button>
                </div>
              </Empty>
            </div>
          )}
      </div>
      )}
    </div>
  );
}
