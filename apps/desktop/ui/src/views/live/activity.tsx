import { useEffect, useMemo, useRef, type ReactNode } from "react";
import type { Activity } from "../../components/PixelOffice";
import { cn } from "../../components/ui";
import { useApp, type LiveRun as Stream } from "../../lib/store";
import type { LiveAgent, LiveData } from "../../lib/types";
import { Call, SHELL, classify } from "./calls";

type Kind = "lead" | "worker" | "standalone" | "pool";

export function kindOf(a: LiveAgent): Kind {
  if (!a.team) return "pool";
  if (a.solo) return "standalone";
  return a.is_lead ? "lead" : "worker";
}

const dotTone: Record<Activity, string> = {
  idle: "bg-zinc-400 dark:bg-zinc-500",
  thinking: "bg-blue-500",
  text: "bg-blue-500",
  tool: "bg-purple-500",
  shell: "bg-purple-500",
  waiting: "bg-amber-500",
  off: "bg-rose-500",
};

export function Dot({ a }: { a: Activity }) {
  const live = a === "thinking" || a === "text" || a === "tool" || a === "shell";
  return (
    <span className="relative inline-flex h-2 w-2 shrink-0">
      {live && <span className={cn("absolute inline-flex h-full w-full animate-ping rounded-full opacity-60", dotTone[a])} />}
      <span className={cn("relative inline-flex h-2 w-2 rounded-full", dotTone[a])} />
    </span>
  );
}

export interface Act {
  activity: Activity;
  ticker: string;
  stream?: Stream;
  /** The call in progress (or the run's last one). */
  call?: Call;
}

/** What each agent does now, from the live stream of its run (or the run's last tool call). */
export function useActivity(data: LiveData | null, live: Record<string, Stream>, t: (k: any) => string) {
  return useMemo(() => {
    const out = new Map<string, Act>();
    for (const a of data?.agents ?? []) {
      const st = a.agent.status;
      const run = a.run;
      const stream = run ? live[run.run.id] : undefined;
      let activity: Activity = "idle";
      let ticker = "";
      let call: Call | undefined;
      const lastTool = stream?.tools.length ? stream.tools[stream.tools.length - 1] : run?.last_tool ? { name: run.last_tool.tool, args: run.last_tool.args } : undefined;
      if (run && lastTool) call = classify(lastTool.name, lastTool.args);
      if (st === "Waiting") {
        activity = "waiting";
        const pend = a.open_tickets.find((x) => x.status === "Pending");
        ticker = `⏳ ${t("live.waiting")}${pend ? ` · ${pend.key}` : ""}`;
      } else if (st === "QuotaExhausted" || st === "Offline") {
        activity = "off";
        ticker = st === "QuotaExhausted" ? `⛔ ${t("live.quota")}` : "⛔ offline";
      } else if (run) {
        const phase = stream && !stream.done ? stream.phase : "thinking";
        if (phase === "tool" && (stream?.tool || call)) {
          const c = stream?.tool ? classify(stream.tool, lastTool?.args) : call!;
          call = c;
          activity = SHELL.has(c.name) ? "shell" : "tool";
          ticker = `⚡ ${c.kind === "mcp" ? "MCP" : c.kind === "skill" ? "Skill" : "tool"}: ${c.label}`;
        } else if (phase === "text" && stream?.text) {
          activity = "text";
          ticker = `✍ ${stream.text.slice(-70).replace(/\s+/g, " ")}`;
        } else {
          activity = "thinking";
          const th = stream?.thinking.trim();
          ticker = th ? `🧠 ${th.slice(-70).replace(/\s+/g, " ")}` : `🧠 ${run.kind === "lead" ? t("live.leading") : run.kind === "review" ? `${t("live.reviewing")} ${run.ticket_key ?? ""}` : `${t("live.working")} ${run.ticket_key ?? ""}`}`;
        }
      } else if (st === "Working") {
        activity = "thinking";
        ticker = `🧠 ${t("live.starting")}`;
      } else {
        const open = a.open_tickets.length;
        ticker = open ? `📋 ${open} ticket · ${a.open_tickets[0].key}` : a.team ? `○ ${t("live.idle")}` : `◌ ${t("live.inPool")}`;
      }
      out.set(a.agent.id, { activity, ticker, stream, call });
    }
    return out;
  }, [data, live, t]);
}

export interface Room {
  id: string; // team id, or "solo"
  name: string;
  sub: string;
  members: string[];
  lead?: string | null;
  project?: string | null;
  deskless: string[];
}

export function roomsOf(data: LiveData, soloName: string): Room[] {
  const teams: Room[] = data.teams
    .filter((t) => !t.solo && t.members.length)
    .map((t) => ({ id: t.id, name: t.name, sub: t.project_name ?? "", members: [t.lead, ...t.members.filter((m) => m !== t.lead)].filter(Boolean) as string[], lead: t.lead, project: t.project, deskless: [] }));
  const solo = data.agents.filter((a) => kindOf(a) === "standalone" || kindOf(a) === "pool");
  const rooms = solo.length ? [{ id: "solo", name: soloName, sub: "", members: solo.map((a) => a.agent.id), deskless: solo.filter((a) => kindOf(a) === "pool").map((a) => a.agent.id) } as Room] : [];
  return [...rooms, ...teams];
}

export interface LogLine {
  at: number;
  agent?: string;
  project?: string;
  icon: ReactNode;
  text: ReactNode;
}

export function RoomLog({ lines }: { lines: LogLine[] }) {
  const app = useApp();
  const end = useRef<HTMLDivElement>(null);
  useEffect(() => {
    // Scroll this list only (scrollIntoView would scroll the page around it too).
    const el = end.current?.parentElement;
    if (el) el.scrollTop = el.scrollHeight;
  }, [lines.length]);
  return (
    <div className="h-full overflow-y-auto px-3 py-2 font-mono text-[11px] leading-relaxed">
      {lines.map((l, i) => (
        <div key={i} className="flex items-start gap-2">
          <span className="shrink-0 text-subtle">{new Date(l.at).toLocaleTimeString(app.lang === "vi" ? "vi-VN" : "en-GB", { hour12: false })}</span>
          <span className="mt-0.5 shrink-0">{l.icon}</span>
          <span className="min-w-0 text-muted">{l.text}</span>
        </div>
      ))}
      {lines.length === 0 && <div className="text-subtle">{app.t("live.logEmpty")}</div>}
      <div ref={end} />
    </div>
  );
}
