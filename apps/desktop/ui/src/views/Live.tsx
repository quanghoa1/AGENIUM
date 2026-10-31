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
import { BookOpen, Bot, Building2, CircleCheck, Coins, Crown, GitBranch, ListChecks, MessageSquareReply, MonitorPlay, OctagonPause, Play, Plug, Radio, ScrollText, Terminal, Users, Wrench } from "lucide-react";
import { useEffect, useRef, useState, type ReactNode } from "react";
import type { Prefill } from "../components/ChatPane";
import { GettingStarted } from "../components/GettingStarted";
import { PanelSwap, Segmented } from "../components/motion";
import { PixelOffice, type CallKind, type OfficeAgent } from "../components/PixelOffice";
import { Empty, Mono, cn, tokens } from "../components/ui";
import { api, onEvent, onRun } from "../lib/api";
import { useApp, useLive } from "../lib/store";
import type { EventRecord, LiveAgent, LiveData } from "../lib/types";
import { classify } from "./live/calls";
import { Dot, LogLine, Room, RoomLog, roomsOf, useActivity } from "./live/activity";
import { Dock } from "./live/Dock";
import { PlanList } from "./live/PlanList";

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

  const notifyRef = useRef(app.notify);
  notifyRef.current = app.notify;
  useEffect(() => {
    // Told once when it starts failing, not every 3 s while it does (audit L24, 2026-10-31).
    let failing = false;
    const load = () =>
      api
        .live()
        .then((d) => {
          failing = false;
          if (d) setData(d);
        })
        .catch((e) => {
          if (!failing) notifyRef.current(String(e));
          failing = true;
        });
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
