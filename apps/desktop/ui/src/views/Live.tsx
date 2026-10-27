// The Live tab: the agents at work, room by room.
//   top     telemetry (agents running, MCP servers enabled in the room, tokens today) and Pause all;
//           room tabs: the shared Solo office, one office per team, and all rooms at once (CCTV);
//   left    the pixel office of the room (click someone to inspect them): the Lead's desk and whiteboard (plan
//           progress), delegation lines to the workers (with the ticket), a sign over each desk with the MCP / Skill
//           / Tool being called; below it the room's live log (assignments, skills loaded, MCP and tool calls,
//           reports);
//   right   two tabs: the selected agent (what it is doing, its model and effort, live thinking, the tool / skill /
//           MCP calls with arguments, output and duration, and the conversation its role allows: chat with a Solo
//           agent or a team's Lead; a worker is read-only with a button that sends you to the Lead about it), or the
//           room's plan as a task list (playing = being worked on, tick = done).
import { useReducedMotion } from "motion/react";
import {
  BookOpen,
  Bot,
  Building2,
  ChevronDown,
  ChevronRight,
  CircleCheck,
  CircleDashed,
  CircleX,
  Coins,
  Crown,
  Eye,
  GitBranch,
  Inbox,
  Link2,
  ListChecks,
  Lock,
  MessageSquareReply,
  MonitorPlay,
  OctagonPause,
  Pause,
  Play,
  Plug,
  Radio,
  ScrollText,
  Terminal,
  Users,
  Wrench,
} from "lucide-react";
import { useEffect, useMemo, useRef, useState, type ReactNode } from "react";
import { ChatPane, LiveTurn, type Prefill } from "../components/ChatPane";
import { GettingStarted } from "../components/GettingStarted";
import { Markdown } from "../components/Markdown";
import { ListItem, PanelSwap, ProgressBar, Segmented } from "../components/motion";
import { PixelOffice, type Activity, type CallKind, type OfficeAgent } from "../components/PixelOffice";
import { Avatar, CacheHit, Empty, Mono, OutcomeBadge, TicketStatusBadge, ago, cn, modelLabel, tokens } from "../components/ui";
import { VendorBadge } from "../components/VendorBadge";
import { api, onEvent, onRun } from "../lib/api";
import { useApp, useLive, type LiveRun as Stream } from "../lib/store";
import type { AgentTrace, BusItem, Column, EventRecord, LiveAgent, LiveData, LogEntry, PlanView, TicketStatus } from "../lib/types";

const SHELL = new Set(["shell", "Bash", "PowerShell", "run_command"]);
/** Agenium's protocol tools (the team's own moves: tickets, reports, questions, plans). */
const PROTOCOL = new Set([
  "answer_question",
  "approve_permission",
  "ask_question",
  "assign_ticket",
  "cancel_ticket",
  "close_ticket",
  "confirm_link",
  "create_aggregate",
  "create_handoff",
  "create_plan",
  "create_ticket",
  "get_team",
  "post_progress",
  "propose_role",
  "propose_role_change",
  "propose_skill_update",
  "propose_soul_change",
  "propose_spec",
  "request_permission",
  "review_ticket",
  "revise_plan",
  "submit_plan_draft",
  "submit_report",
  "suggest_link",
]);

type Kind = "lead" | "worker" | "standalone" | "pool";

function kindOf(a: LiveAgent): Kind {
  if (!a.team) return "pool";
  if (a.solo) return "standalone";
  return a.is_lead ? "lead" : "worker";
}

/** A short view of a tool call's arguments (path, command, pattern…). */
function argPreview(args: any): string {
  if (!args || typeof args !== "object") return "";
  const v = args.command ?? args.cmd ?? args.path ?? args.pattern ?? args.query ?? args.url ?? args.name ?? args.ticket_id ?? Object.values(args)[0];
  const s = typeof v === "string" ? v : JSON.stringify(v ?? "");
  return s.length > 60 ? s.slice(0, 59) + "…" : s;
}

export interface Call {
  kind: CallKind;
  /** Short label: `server :: tool`, the skill name, `bash: <command>`, `read_file: <path>`. */
  label: string;
  name: string;
}

/** Sorts a call into MCP (`mcp__<server>__<tool>`), Skill (`load_skill`), Agenium (protocol) or Tool. */
export function classify(tool: string, args: any): Call {
  const name = tool.replace(/^cli:/, "");
  if (name.startsWith("mcp__")) {
    const [, server, ...rest] = name.split("__");
    return { kind: "mcp", label: `${server} :: ${rest.join("__")}`, name };
  }
  if (name === "load_skill" || name === "Skill") return { kind: "skill", label: String(args?.name ?? args?.skill ?? args?.command ?? "skill"), name };
  if (PROTOCOL.has(name)) return { kind: "agenium", label: `${name}${args?.ticket_id ? ` ${args.ticket_id}` : ""}`, name };
  if (SHELL.has(name)) return { kind: "tool", label: `bash: ${argPreview(args)}`, name };
  return { kind: "tool", label: `${name}: ${argPreview(args)}`, name };
}

const CALL_TONE: Record<CallKind, string> = {
  mcp: "border-purple-500/40 bg-purple-500/10 text-purple-600 dark:text-purple-300",
  skill: "border-teal-500/40 bg-teal-500/10 text-teal-600 dark:text-teal-300",
  tool: "border-line-strong bg-elevated text-muted",
  agenium: "border-blue-500/40 bg-blue-500/10 text-blue-600 dark:text-blue-300",
};

function CallBadge({ kind }: { kind: CallKind }) {
  const icon = kind === "mcp" ? <Plug className="h-3 w-3" /> : kind === "skill" ? <BookOpen className="h-3 w-3" /> : kind === "agenium" ? <GitBranch className="h-3 w-3" /> : <Wrench className="h-3 w-3" />;
  return (
    <span className={cn("chip shrink-0", CALL_TONE[kind])}>
      {icon}
      {kind === "mcp" ? "MCP" : kind === "skill" ? "Skill" : kind === "agenium" ? "Agenium" : "Tool"}
    </span>
  );
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

function Dot({ a }: { a: Activity }) {
  const live = a === "thinking" || a === "text" || a === "tool" || a === "shell";
  return (
    <span className="relative inline-flex h-2 w-2 shrink-0">
      {live && <span className={cn("absolute inline-flex h-full w-full animate-ping rounded-full opacity-60", dotTone[a])} />}
      <span className={cn("relative inline-flex h-2 w-2 rounded-full", dotTone[a])} />
    </span>
  );
}

interface Act {
  activity: Activity;
  ticker: string;
  stream?: Stream;
  /** The call in progress (or the run's last one). */
  call?: Call;
}

/** What each agent does now, from the live stream of its run (or the run's last tool call). */
function useActivity(data: LiveData | null, live: Record<string, Stream>, t: (k: any) => string) {
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

// ---------------------------------------------------------------- rooms

interface Room {
  id: string; // team id, or "solo"
  name: string;
  sub: string;
  members: string[];
  lead?: string | null;
  project?: string | null;
  deskless: string[];
}

function roomsOf(data: LiveData, soloName: string): Room[] {
  const teams: Room[] = data.teams
    .filter((t) => !t.solo && t.members.length)
    .map((t) => ({ id: t.id, name: t.name, sub: t.project_name ?? "", members: [t.lead, ...t.members.filter((m) => m !== t.lead)].filter(Boolean) as string[], lead: t.lead, project: t.project, deskless: [] }));
  const solo = data.agents.filter((a) => kindOf(a) === "standalone" || kindOf(a) === "pool");
  const rooms = solo.length ? [{ id: "solo", name: soloName, sub: "", members: solo.map((a) => a.agent.id), deskless: solo.filter((a) => kindOf(a) === "pool").map((a) => a.agent.id) } as Room] : [];
  return [...rooms, ...teams];
}

// ---------------------------------------------------------------- room log

interface LogLine {
  at: number;
  agent?: string;
  project?: string;
  icon: ReactNode;
  text: ReactNode;
}

function RoomLog({ lines }: { lines: LogLine[] }) {
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

// ---------------------------------------------------------------- right dock

const PROGRESS: Record<TicketStatus, number> = { New: 0.05, Assigned: 0.15, InProgress: 0.45, Pending: 0.55, Reopened: 0.35, Resolved: 0.85, Closed: 1, Cancelled: 0 };

/** The Lead's delegations: every ticket of the project with its worker, progress and "inspect". */
function Delegations({ project, data, onWatch }: { project: string; data: LiveData; onWatch: (id: string) => void }) {
  const app = useApp();
  const [cols, setCols] = useState<Column[]>([]);
  useEffect(() => {
    api.board(project).then((c) => setCols(c ?? []));
  }, [project, app.tick]);
  const byName = new Map(data.agents.map((a) => [a.agent.name, a]));
  const cards = cols.flatMap((c) => c.cards.map((card) => ({ ...card, status: c.status }))).filter((c) => c.assignee && c.status !== "Cancelled");
  if (!cards.length) return null;
  return (
    <div className="rounded-xl border border-purple-500/30 bg-purple-500/5 p-3">
      <div className="mb-2 flex items-center gap-1.5 text-[11px] font-semibold text-purple-600 dark:text-purple-300">
        <GitBranch className="h-3.5 w-3.5" /> {app.t("live.delegations")}
      </div>
      <div className="space-y-1.5">
        {cards.map((c) => {
          const w = c.assignee ? byName.get(c.assignee) : undefined;
          return (
            <div key={c.key} className="flex items-center gap-2 rounded-lg border border-line bg-surface p-2">
              <Avatar name={c.assignee ?? "?"} className="h-6 w-6 text-[9px]" />
              <div className="min-w-0 flex-1">
                <div className="flex items-center gap-1.5">
                  <Mono className="text-cyan-600 dark:text-cyan-400">{c.key}</Mono>
                  <span className="truncate text-[11px] font-medium">{c.title}</span>
                </div>
                <div className="mt-1 flex items-center gap-2">
                  <div className="h-1 flex-1 overflow-hidden rounded-full bg-line">
                    <ProgressBar value={PROGRESS[c.status]} className={c.status === "Pending" ? "bg-amber-500" : c.status === "Reopened" ? "bg-rose-500" : undefined} />
                  </div>
                  <span className="text-[11px] text-subtle">@{c.assignee}</span>
                  <TicketStatusBadge status={c.status} />
                </div>
              </div>
              {w && (
                <button className="btn shrink-0 px-2 py-1 text-[11px]" onClick={() => onWatch(w.agent.id)}>
                  <Eye className="h-3 w-3" /> {app.t("live.watch")}
                </button>
              )}
            </div>
          );
        })}
      </div>
    </div>
  );
}

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
function WorkerBus({ agent, lead, stream }: { agent: LiveAgent; lead?: string | null; stream?: Stream }) {
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

/** What the Jev judge said for this run (ADR-75): what it was asked, and its answer when opened. */
function JudgeRow({ e }: { e: LogEntry }) {
  const [open, setOpen] = useState(false);
  return (
    <div className="rounded-lg border border-fuchsia-500/30">
      <button className="flex w-full items-center gap-2 px-2.5 py-1.5 text-left" onClick={() => setOpen(!open)}>
        {open ? <ChevronDown className="h-3 w-3 shrink-0 text-subtle" /> : <ChevronRight className="h-3 w-3 shrink-0 text-subtle" />}
        <span className="chip shrink-0 border-fuchsia-500/40 bg-fuchsia-500/10 text-fuchsia-600 dark:text-fuchsia-300">Jev</span>
        <span className="truncate font-mono text-[11px]">{e.data.use}</span>
      </button>
      {open && (
        <div className="border-t border-line p-2.5 text-[11px]">
          <Markdown text={String(e.data.text ?? "")} />
        </div>
      )}
    </div>
  );
}

/** One call of the trace: badge, label, duration, ok; arguments and output when opened. */
function CallRow({ e, open: start = false }: { e: LogEntry; open?: boolean }) {
  const [open, setOpen] = useState(start);
  const c = classify(e.data.tool, e.data.args);
  const ok = e.data.ok !== false;
  return (
    <div className={cn("rounded-lg border", ok ? "border-line" : "border-rose-500/40")}>
      <button className="flex w-full items-center gap-2 px-2.5 py-1.5 text-left" onClick={() => setOpen(!open)}>
        {open ? <ChevronDown className="h-3 w-3 shrink-0 text-subtle" /> : <ChevronRight className="h-3 w-3 shrink-0 text-subtle" />}
        <CallBadge kind={c.kind} />
        <span className="truncate font-mono text-[11px]">{c.label}</span>
        <span className="ml-auto flex shrink-0 items-center gap-1.5">
          {e.data.ms != null && <Mono className="text-subtle">{e.data.ms} ms</Mono>}
          {ok ? <CircleCheck className="h-3.5 w-3.5 text-emerald-500" /> : <CircleX className="h-3.5 w-3.5 text-rose-500" />}
        </span>
      </button>
      {open && (
        <div className="space-y-1.5 border-t border-line p-2.5">
          <div className="label">Args</div>
          <pre className="max-h-40 overflow-auto rounded bg-elevated p-2 font-mono text-[11px]">{JSON.stringify(e.data.args, null, 2)}</pre>
          <div className="label">Out</div>
          <pre className={cn("max-h-56 overflow-auto whitespace-pre-wrap rounded p-2 font-mono text-[11px]", SHELL.has(c.name) ? "bg-[#0a0e14] text-zinc-300" : ok ? "bg-elevated" : "bg-rose-500/10 text-rose-600 dark:text-rose-300")}>
            {String(e.data.result ?? "")}
          </pre>
        </div>
      )}
    </div>
  );
}

/** The selected agent: who it is and what it works on, then its conversation (`view` agent) or its tool, skill and MCP
 *  calls (`view` tools), each with the whole panel. */
function Dock({ a, data, act, prefill, onPick, onAskLead, view }: { a: LiveAgent; data: LiveData; act?: Act; prefill: Prefill | null; onPick: (id: string) => void; onAskLead: (w: LiveAgent) => void; view: "agent" | "tools" }) {
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
      <div className="flex min-h-0 flex-1 flex-col">
        {(k === "lead" || k === "standalone") && a.project ? (
            <>
              {k === "lead" && (
                <div className="max-h-[34%] shrink-0 overflow-y-auto border-t border-line px-4 py-2.5 empty:hidden">
                  <Delegations project={a.project} data={data} onWatch={onPick} />
                </div>
              )}
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

// ---------------------------------------------------------------- plan of the room

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
function PlanList({ project, data }: { project?: string | null; data: LiveData }) {
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

// ---------------------------------------------------------------- page

function Pill({ icon, label, value }: { icon: ReactNode; label: string; value: ReactNode }) {
  return (
    <span className="inline-flex items-center gap-1.5 rounded-full border border-line-strong bg-elevated px-2.5 py-1 text-[11px]">
      {icon}
      <span className="text-subtle">{label}</span>
      <b className="font-mono">{value}</b>
    </span>
  );
}

const ZOOMS: (number | null)[] = [null, 2, 3, 4];

export function LiveView() {
  const app = useApp();
  const reduce = useReducedMotion();
  const [data, setData] = useState<LiveData | null>(null);
  const [room, setRoom] = useState<string | null>(null);
  const [focus, setFocus] = useState<string | null>(null);
  const [prefill, setPrefill] = useState<Prefill | null>(null);
  const [zoom, setZoom] = useState<number | null>(null);
  const [side, setSide] = useState<"agent" | "tools" | "plan">("agent");
  const [flights, setFlights] = useState<{ id: number; from: string; to: string; kind: string }[]>([]);
  const [log, setLog] = useState<LogLine[]>([]);
  const [boards, setBoards] = useState<Record<string, string[]>>({});
  const dataRef = useRef<LiveData | null>(null);
  dataRef.current = data;
  const tRef = useRef(app.t);
  tRef.current = app.t;
  // Another project chosen in the header: its room comes up.
  useEffect(() => {
    setRoom((r) => (r === "cctv" ? r : null));
  }, [app.project]);

  useEffect(() => {
    const load = () => api.live().then((d) => d && setData(d));
    load();
    const h = window.setInterval(load, 3000);
    return () => window.clearInterval(h);
  }, [app.tick]);

  const name = (id?: string) => dataRef.current?.agents.find((a) => a.agent.id === id)?.agent.name ?? "?";
  const projectOfKey = (key?: string) => {
    if (!key) return undefined;
    const prefix = key.split("-")[0];
    return app.projects.find((p) => p.key_prefix === prefix)?.id;
  };
  /** A domain event as a log line (or nothing when it says little). */
  const lineOf = (e: EventRecord): LogLine | null => {
    const at = new Date(e.at).getTime();
    const actor = String(e.actor).startsWith("agent:") ? String(e.actor).slice(6) : undefined;
    const p = e.payload ?? {};
    const t = tRef.current;
    switch (e.kind) {
      case "TicketAssigned":
        return { at, agent: p.agent, project: projectOfKey(p.key), icon: <Crown className="h-3 w-3 text-amber-500" />, text: <>{actor ? <b>{name(actor)}</b> : t("live.user")} {t("live.assigned")} <Mono>{p.key}</Mono> → <b>{name(p.agent)}</b></> };
      case "CommentAdded":
        return { at, agent: actor, project: projectOfKey(p.key), icon: <MessageSquareReply className="h-3 w-3 text-cyan-500" />, text: <><b>{actor ? name(actor) : e.actor}</b> · {p.kind} · <Mono>{p.key}</Mono></> };
      case "TicketStatusChanged":
        return { at, project: projectOfKey(p.key), icon: <ListChecks className="h-3 w-3 text-blue-500" />, text: <><Mono>{p.key}</Mono> {p.from} → <b>{p.to}</b></> };
      case "TicketMerged":
        return { at, project: projectOfKey(p.key), icon: <GitBranch className="h-3 w-3 text-emerald-500" />, text: <><Mono>{p.key}</Mono> merged</> };
      case "RunStarted":
        return { at, agent: p.agent, icon: <Play className="h-3 w-3 text-blue-500" />, text: <><b>{name(p.agent)}</b> {t("live.runStarted")}</> };
      case "RunEnded":
        return { at, agent: p.agent, icon: <CircleCheck className="h-3 w-3 text-emerald-500" />, text: <><b>{name(p.agent)}</b> {t("live.runEnded")} ({String(p.outcome).split(":")[0]})</> };
      case "PlanProposed":
      case "PlanApproved":
      case "SpecProposed":
      case "SpecApproved":
        return { at, agent: actor, icon: <ScrollText className="h-3 w-3 text-purple-500" />, text: <>{e.kind} v{p.version}</> };
      default:
        return null;
    }
  };

  // The log starts from the recent events, then grows live.
  useEffect(() => {
    let alive = true;
    const t0 = window.setTimeout(() => {
      api.events(undefined, 150).then((es) => {
        if (!alive || !es) return;
        const lines = [...es].sort((a, b) => a.seq - b.seq).map(lineOf).filter(Boolean) as LogLine[];
        setLog((cur) => [...lines, ...cur].slice(-300));
      });
    }, 300);
    let n = 0;
    const un = onEvent((e) => {
      const l = lineOf(e);
      if (l) setLog((cur) => [...cur.slice(-299), l]);
      // A comment a Lead or a worker adds to a ticket flies to the other as an envelope (Agenium hands it on through
      // the ticket: the worker's next run reads it, a question goes to the Lead's inbox).
      const d = dataRef.current;
      if (!d || e.kind !== "CommentAdded" || !String(e.actor).startsWith("agent:")) return;
      const from = String(e.actor).slice(6);
      const author = d.agents.find((a) => a.agent.id === from);
      if (!author) return;
      const team = d.teams.find((t) => t.id === author.team);
      const to = author.is_lead ? d.agents.find((a) => a.open_tickets.some((t) => t.key === e.payload?.key) && a.agent.id !== from)?.agent.id : (team?.lead ?? undefined);
      if (to) setFlights((f) => [...f.slice(-10), { id: Date.now() + n++, from, to, kind: String(e.payload?.kind ?? "") }]);
    });
    // Tool, skill and MCP calls as they happen.
    const unRun = onRun((run, ev) => {
      if (ev.type !== "tool") return;
      const a = dataRef.current?.agents.find((x) => x.run?.run.id === run);
      if (!a) return;
      const c = classify(ev.name, ev.args);
      const t = tRef.current;
      setLog((cur) => [
        ...cur.slice(-299),
        {
          at: Date.now(),
          agent: a.agent.id,
          icon: c.kind === "mcp" ? <Plug className="h-3 w-3 text-purple-500" /> : c.kind === "skill" ? <BookOpen className="h-3 w-3 text-teal-500" /> : c.kind === "agenium" ? <GitBranch className="h-3 w-3 text-blue-500" /> : <Terminal className="h-3 w-3 text-muted" />,
          text: (
            <>
              <b>{a.agent.name}</b> {c.kind === "skill" ? t("live.loads") : t("live.calls")} [{c.kind === "mcp" ? "MCP" : c.kind === "skill" ? "Skill" : c.kind === "agenium" ? "Agenium" : "Tool"}: {c.label}]
            </>
          ),
        },
      ]);
    });
    return () => {
      alive = false;
      window.clearTimeout(t0);
      un.then((f) => f());
      unRun.then((f) => f());
    };
  }, []);

  const live = useLive();
  const acts = useActivity(data, live, app.t);
  const rooms = data ? roomsOf(data, app.t("live.soloOffice")) : [];
  // Until the user picks a room, the room of the project chosen in the header (then the first team's).
  const current =
    rooms.find((r) => r.id === room) ?? (room === "cctv" ? undefined : rooms.find((r) => r.id !== "solo" && r.project === app.project) ?? rooms.find((r) => r.id !== "solo") ?? rooms[0]);
  const cctv = room === "cctv";

  // Whiteboards: each team's plan progress.
  useEffect(() => {
    if (!data) return;
    for (const r of roomsOf(data, "")) {
      if (!r.project) continue;
      api.plan(r.project).then((p) => {
        if (!p) return;
        const done = p.items.filter((i) => i.done).length;
        const lines = p.plan ? [`PLAN v${p.plan.version}`, `${done}/${p.items.length} ${app.t("live.boardDone")}`, p.items.find((i) => !i.done)?.id ?? ""] : p.spec ? [`SPEC v${p.spec.version}`, p.spec.approved_by ? app.t("live.boardApproved") : app.t("live.boardToApprove")] : [];
        setBoards((b) => ({ ...b, [r.id]: lines }));
      });
    }
  }, [data?.teams.map((t) => t.id).join(","), app.tick]);

  // Keep the selection inside the room.
  useEffect(() => {
    if (!current) return;
    if (!focus || !current.members.includes(focus)) setFocus(current.lead ?? current.members[0] ?? null);
  }, [current?.id, current?.members.join(",")]);

  if (!data) return <div className="text-xs text-subtle">{app.t("common.loading")}</div>;
  const byId = new Map(data.agents.map((a) => [a.agent.id, a]));
  const selected = focus ? byId.get(focus) : undefined;
  const toolsNow = [...acts.values()].filter((x) => x.activity === "tool" || x.activity === "shell").length;
  const stopped = !app.projects.some((p) => p.run_state === "Running") && data.totals.live_runs === 0;
  const officeOf = (r: Room): OfficeAgent[] =>
    r.members
      .map((id) => byId.get(id))
      .filter(Boolean)
      .map((a) => ({
        agent: a!,
        activity: acts.get(a!.agent.id)?.activity ?? "idle",
        ticker: acts.get(a!.agent.id)?.ticker ?? "",
        // A task keeps someone at the desk: its open tickets, a run, or (a Lead) a team with open tickets.
        duty: a!.open_tickets.length > 0 || !!a!.run || (a!.agent.id === r.lead && r.members.some((m) => (byId.get(m)?.open_tickets.length ?? 0) > 0)),
      }));
  const signsOf = (r: Room) => {
    const out: Record<string, { kind: CallKind; text: string }> = {};
    for (const id of r.members) {
      const x = acts.get(id);
      if (x?.call && x.activity !== "idle" && x.activity !== "off") out[id] = { kind: x.call.kind, text: x.call.label };
    }
    return out;
  };
  // Delegation lines: the Lead's desk -> each worker holding a ticket, labelled with the ticket it works on.
  const flowsOf = (r: Room): [string, string, string?][] =>
    r.lead
      ? r.members
          .filter((m) => m !== r.lead && (byId.get(m)?.open_tickets.length ?? 0) > 0)
          .map((m) => {
            const a = byId.get(m)!;
            const t = a.run?.ticket_key ?? a.open_tickets.find((x) => x.status === "InProgress")?.key ?? a.open_tickets[0]?.key;
            return [r.lead!, m, t];
          })
      : [];
  const mcpIn = (r?: Room) => new Set((r?.members ?? []).flatMap((id) => (Array.isArray(byId.get(id)?.agent.mcp) ? byId.get(id)!.agent.mcp.map((b: { server: string }) => b.server) : []))).size;
  const busyIn = (r: Room) => r.members.filter((m) => {
    const x = acts.get(m)?.activity;
    return x && x !== "idle" && x !== "off";
  }).length;
  const pick = (id: string) => {
    setFocus(id);
    const r = rooms.find((x) => x.members.includes(id));
    if (r && cctv) setRoom(r.id);
  };
  const askLead = (w: LiveAgent) => {
    const team = data.teams.find((t) => t.id === w.team);
    if (!team?.lead) return;
    const ticket = w.open_tickets[0]?.key;
    setFocus(team.lead);
    setSide("agent");
    setPrefill({ text: `@${w.agent.name}${ticket ? ` (${ticket})` : ""}: `, nonce: Date.now() });
  };
  const roomLog = current ? log.filter((l) => (l.agent && current.members.includes(l.agent)) || (l.project && current.project === l.project)) : log;

  return (
    // Edge to edge over the page's padding; `overflow-x-clip` keeps a panel sliding in (PanelSwap) from scrolling the page sideways.
    <div className="-m-5 flex h-[calc(100%+2.5rem)] min-h-0 flex-col overflow-x-clip">
      {/* top bar */}
      <div className="flex flex-wrap items-center gap-2 border-b border-line bg-surface px-4 py-2">
        <Radio className="h-4 w-4 text-rose-500" />
        <span className="text-sm font-semibold">{app.t("nav.live")}</span>
        <Pill icon={<Bot className="h-3.5 w-3.5 text-blue-500" />} label={app.t("live.active")} value={`${data.totals.working}/${data.agents.length}`} />
        <Pill icon={<Wrench className="h-3.5 w-3.5 text-purple-500" />} label={app.t("live.toolsNow")} value={toolsNow} />
        <Pill icon={<Plug className="h-3.5 w-3.5 text-purple-500" />} label={cctv ? "MCP" : `MCP · ${current?.name ?? ""}`} value={cctv ? mcpIn({ members: data.agents.map((a) => a.agent.id) } as Room) : mcpIn(current)} />
        <Pill icon={<Coins className="h-3.5 w-3.5 text-amber-500" />} label={app.t("live.tokensToday")} value={tokens(data.totals.tokens_today)} />
        <button
          className={cn("ml-auto", stopped ? "btn" : "btn-danger")}
          disabled={stopped}
          onClick={() =>
            api.stopAll().then(() => {
              app.notify(app.t("top.stopped"));
              app.refresh();
            })
          }
        >
          <OctagonPause className="h-3.5 w-3.5" /> {stopped ? app.t("live.paused") : app.t("live.pauseAll")}
        </button>
      </div>
      {/* room tabs */}
      <div className="flex items-center gap-1 overflow-x-auto border-b border-line bg-surface px-3 py-1.5">
        {rooms.map((r) => {
          const on = !cctv && current?.id === r.id;
          const busy = busyIn(r);
          return (
            <button key={r.id} onClick={() => setRoom(r.id)} className={cn("press flex shrink-0 items-center gap-1.5 rounded-lg px-3 py-1.5 text-xs", on ? "bg-blue-600/15 font-semibold text-blue-600 dark:text-blue-400" : "text-muted hover:bg-elevated")}>
              {r.id === "solo" ? <Building2 className="h-3.5 w-3.5" /> : <Users className="h-3.5 w-3.5" />}
              {r.name}
              <span className="font-mono text-[11px] text-subtle">({r.members.length})</span>
              {busy > 0 && <span className="h-1.5 w-1.5 animate-pulse rounded-full bg-blue-500" />}
            </button>
          );
        })}
        <button onClick={() => setRoom("cctv")} className={cn("press ml-1 flex shrink-0 items-center gap-1.5 rounded-lg px-3 py-1.5 text-xs", cctv ? "bg-blue-600/15 font-semibold text-blue-600 dark:text-blue-400" : "text-muted hover:bg-elevated")}>
          <MonitorPlay className="h-3.5 w-3.5" /> {app.t("live.cctv")}
        </button>
      </div>
      {data.agents.length === 0 ? (
        <div className="mx-auto w-full max-w-2xl p-6">
          <div className="mb-3 text-xs text-muted">{app.t("live.empty")}</div>
          <GettingStarted />
        </div>
      ) : (
        <div className="flex min-h-0 flex-1">
          {/* left: office (or every room) and the room's log / plan */}
          <section className="flex min-w-0 flex-[62] flex-col border-r border-line bg-background">
            {cctv ? (
              <div className="grid min-h-0 flex-1 auto-rows-min gap-3 overflow-y-auto p-3 xl:grid-cols-2">
                {rooms.map((r) => (
                  <div key={r.id} className="card overflow-hidden">
                    <button className="flex w-full items-center gap-2 border-b border-line px-3 py-1.5 text-left text-xs hover:bg-elevated" onClick={() => setRoom(r.id)}>
                      <span className="h-1.5 w-1.5 rounded-full bg-rose-500" />
                      <b>{r.name}</b>
                      <span className="text-subtle">{r.sub}</span>
                      <span className="ml-auto font-mono text-[11px] text-subtle">
                        {busyIn(r)}/{r.members.length}
                      </span>
                    </button>
                    <div className="p-2">
                      <PixelOffice compact agents={officeOf(r)} lead={r.lead} deskless={r.deskless} selected={focus} onSelect={pick} flights={flights} reduceMotion={!!reduce} signs={signsOf(r)} flows={flowsOf(r)} board={boards[r.id]} />
                    </div>
                  </div>
                ))}
              </div>
            ) : (
              current && (
                <>
                  <div className="flex items-center gap-2 border-b border-line px-3 py-1.5">
                    <div className="flex min-w-0 flex-1 items-center gap-1 overflow-x-auto">
                      {current.members.map((id) => {
                        const a = byId.get(id);
                        if (!a) return null;
                        return (
                          <button key={id} onClick={() => setFocus(id)} className={cn("press flex shrink-0 items-center gap-1.5 rounded-full border px-2 py-0.5 text-[11px]", focus === id ? "border-accent bg-blue-600/10" : "border-line hover:border-line-strong")}>
                            {id === current.lead ? <Crown className="h-3 w-3 text-amber-500" /> : <Bot className="h-3 w-3 text-subtle" />}
                            {a.agent.name}
                            <Dot a={acts.get(id)?.activity ?? "idle"} />
                          </button>
                        );
                      })}
                    </div>
                    <Segmented
                      id="office-zoom"
                      value={String(zoom ?? "fit")}
                      onChange={(v) => setZoom(v === "fit" ? null : Number(v))}
                      items={ZOOMS.map((z) => ({ value: String(z ?? "fit"), label: z ? `${z}x` : app.t("live.fit") }))}
                    />
                  </div>
                  <div className="min-h-0 flex-1 overflow-auto p-3">
                    <PixelOffice key={current.id} agents={officeOf(current)} lead={current.lead} deskless={current.deskless} selected={focus} onSelect={pick} flights={flights} reduceMotion={!!reduce} zoom={zoom ?? undefined} signs={signsOf(current)} flows={flowsOf(current)} board={boards[current.id]} />
                  </div>
                </>
              )
            )}
            <div className="flex h-[200px] shrink-0 flex-col border-t border-line bg-surface 2xl:h-[240px]">
              <div className="flex items-center gap-2 border-b border-line px-3 py-1.5 text-[11px]">
                <ScrollText className="h-3 w-3 text-muted" />
                <b>{app.t("live.roomLog")}</b>
                <span className="text-[11px] text-subtle">{cctv ? app.t("live.allRooms") : current?.name}</span>
              </div>
              <div className="min-h-0 flex-1">
                <RoomLog lines={roomLog} />
              </div>
            </div>
          </section>
          {/* right: the selected agent, or the room's plan */}
          <aside className="flex min-w-[420px] flex-[38] flex-col bg-surface">
            <div className="flex items-center gap-2 border-b border-line px-3 py-1.5">
              <Segmented
                id="live-side"
                value={side}
                onChange={setSide}
                items={[
                  { value: "agent", label: <><Bot className="h-3 w-3" /> {app.t("live.agentTab")}</> },
                  { value: "tools", label: <><Wrench className="h-3 w-3" /> {app.t("live.toolsTab")}</> },
                  { value: "plan", label: <><ListChecks className="h-3 w-3" /> {app.t("live.planTab")}</> },
                ]}
              />
              <span className="truncate text-[11px] text-subtle">{side === "plan" ? (current?.name ?? selected?.project_name) : selected?.agent.name}</span>
            </div>
            <PanelSwap id={side} className="flex min-h-0 flex-1 flex-col">
              {side === "plan" ? (
                <PlanList project={current?.project ?? selected?.project} data={data} />
              ) : selected ? (
                <Dock key={selected.agent.id} a={selected} data={data} act={acts.get(selected.agent.id)} prefill={prefill} onPick={pick} onAskLead={askLead} view={side} />
              ) : (
                <Empty>{app.t("live.pickAgent")}</Empty>
              )}
            </PanelSwap>
          </aside>
        </div>
      )}
    </div>
  );
}
