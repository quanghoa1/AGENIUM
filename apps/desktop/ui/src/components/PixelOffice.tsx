import { useEffect, useRef, useState } from "react";
import type { LiveAgent } from "../lib/types";
import { R, T, look, pose, sprite } from "./office/sprites";
import { ROOM, armchair, bookshelf, chair, clock, coffeeTable, cooler, desk, floor, hangingPlant, picture, plant, rug, sofa, wall, whiteboard } from "./office/furniture";
import { bubble, crown, envelope, label, labelWidth, sign, vendorChip } from "./office/overlays";
import { Walker, buildLayout, deskSeat, key, path, tileCentre } from "./office/layout";

export type Activity = "idle" | "thinking" | "tool" | "shell" | "text" | "waiting" | "off";

export interface OfficeAgent {
  agent: LiveAgent;
  activity: Activity;
  ticker: string;
  /** Has a task (open ticket, run, or a team at work for a Lead): sits at the desk even while waiting. */
  duty: boolean;
}

interface Props {
  agents: OfficeAgent[];
  /** The team's Lead (a bigger desk at the top, a crown); none for the Solo office. */
  lead?: string | null;
  /** Agents without a desk (the pool): they live in the lounge. */
  deskless?: string[];
  selected?: string | null;
  onSelect: (id: string) => void;
  flights: { id: number; from: string; to: string; kind: string }[];
  reduceMotion?: boolean;
  /** Pixel zoom; `undefined` = fit the width (2x or 3x). */
  zoom?: number;
  /** What each agent calls right now, shown as a sign over its desk. */
  signs?: Record<string, { kind: CallKind; text: string }>;
  /** Delegations to draw as flow lines: Lead -> worker, with the ticket. */
  flows?: [string, string, string?][];
  /** Whiteboard lines (the team's plan progress). */
  board?: string[];
  /** Smaller, calmer rendering for the all-rooms (CCTV) grid. */
  compact?: boolean;
}

export type CallKind = "mcp" | "skill" | "tool" | "agenium";

export function PixelOffice({ agents, lead, deskless = [], selected, onSelect, flights, reduceMotion, zoom, signs, flows, board, compact }: Props) {
  const wrap = useRef<HTMLDivElement>(null);
  const canvas = useRef<HTMLCanvasElement>(null);
  const [width, setWidth] = useState(480);
  const [hover, setHover] = useState<{ id: string; x: number; y: number } | null>(null);
  const walkers = useRef(new Map<string, Walker>());
  const flying = useRef<{ id: number; from: string; to: string; start: number }[]>([]);
  const seen = useRef(new Set<number>());
  const hits = useRef<{ id: string; x: number; y: number; w: number; h: number }[]>([]);

  // Integer zoom (crisp pixels) and as many columns as fit.
  const scale = zoom ?? (compact ? 2 : width >= 900 ? 4 : width >= 560 ? 3 : 2);
  const cols = Math.max(9, Math.min(compact ? 14 : 22, Math.floor(width / (T * scale))));
  const ids = agents.map((a) => a.agent.agent.id);
  const lay = buildLayout(cols, ids, lead, new Set(deskless));
  const layoutKey = `${cols}|${ids.join(",")}|${lead ?? ""}|${deskless.join(",")}`;
  const state = useRef({ agents, selected, reduceMotion, lay, layoutKey, scale, lead, signs, flows, board });
  state.current = { agents, selected, reduceMotion, lay, layoutKey, scale, lead, signs, flows, board };

  useEffect(() => {
    const el = wrap.current;
    if (!el) return;
    const ro = new ResizeObserver(() => setWidth(el.clientWidth));
    ro.observe(el);
    setWidth(el.clientWidth);
    return () => ro.disconnect();
  }, []);

  useEffect(() => {
    for (const f of flights) {
      if (seen.current.has(f.id)) continue;
      seen.current.add(f.id);
      flying.current.push({ id: f.id, from: f.from, to: f.to, start: performance.now() });
    }
  }, [flights]);

  useEffect(() => {
    const c = canvas.current;
    const g = c?.getContext("2d");
    if (!c || !g) return;
    let raf = 0;
    let last = performance.now();
    const draw = (t: number) => {
      const dt = Math.min(100, t - last);
      last = t;
      const { agents, selected, reduceMotion, lay, layoutKey, scale, lead, signs, flows, board } = state.current;
      const W = lay.cols * T;
      const H = lay.rows * T;
      g.setTransform(scale, 0, 0, scale, 0, 0);
      g.imageSmoothingEnabled = false;
      const still = !!reduceMotion;
      const frame = still ? 0 : Math.floor(t / 180);
      // the room
      const tones = document.documentElement.classList.contains("dark") ? ROOM.dark : ROOM.light;
      floor(g, 0, 2 * T, W, H - 2 * T, tones);
      wall(g, 0, W, tones);
      bookshelf(g, T, 4);
      if (lay.cols >= 11) bookshelf(g, W - 3 * T, 4);
      clock(g, Math.floor(W / 2) - 6, 8, t);
      if (board?.length) whiteboard(g, Math.floor(W / 2) - 60, 5, board);
      else picture(g, Math.floor(W / 2) - 28, 9, lay.cols);
      picture(g, Math.floor(W / 2) + 14, 9, lay.cols + 1);
      hangingPlant(g, 2, 3);
      hangingPlant(g, W - 14, 3);
      rug(g, lay.rugAt.tx * T, lay.rugAt.ty * T + 4, lay.rugAt.w * T, lay.rugAt.h * T - 4);
      coffeeTable(g, lay.tableAt.tx * T, lay.tableAt.ty * T);
      cooler(g, lay.coolerAt.tx * T, lay.coolerAt.ty * T);
      for (const p of lay.plants) plant(g, p.tx * T, p.ty * T, t, still);
      // The room's edge: dark on the dark theme, a soft grey on the light one.
      g.strokeStyle = tones === ROOM.dark ? "#11151f" : "#94a3b8";
      g.lineWidth = 2;
      g.strokeRect(1, 1, W - 2, H - 2);

      // Who works (sits at the desk) and who is free (goes to the lounge, one place each, sofa first).
      const byId = new Map(agents.map((a) => [a.agent.agent.id, a]));
      const atDesk = (oa: OfficeAgent) => lay.desks.has(oa.agent.agent.id) && (oa.duty || oa.activity !== "idle");
      const free = agents.filter((a) => !atDesk(a)).map((a) => a.agent.agent.id);
      for (const oa of agents) {
        const id = oa.agent.agent.id;
        const d = lay.desks.get(id);
        const lounge = lay.loungeSeats[Math.max(0, free.indexOf(id)) % Math.max(1, lay.loungeSeats.length)];
        const working = atDesk(oa);
        const seatKey = working ? "desk" : key(lounge.spot.tx, lounge.spot.ty);
        let w = walkers.current.get(id);
        if (!w || w.layoutKey !== layoutKey) {
          const p = working ? deskSeat(d!) : tileCentre(lounge.spot);
          w = { x: p.x, y: p.y, route: [], goal: null, plan: working ? "desk" : "sofa", facing: working ? "up" : lounge.facing, sitting: working ? "desk" : lounge.seat ? "sofa" : null, seatFacing: lounge.facing, seat: seatKey, layoutKey };
          walkers.current.set(id, w);
        }
        if (w.seat !== seatKey) {
          // Going to work, or to the lounge (or another lounge place).
          w.seat = seatKey;
          w.plan = working ? "desk" : "sofa";
          w.seatFacing = lounge.facing;
          const to = working ? d!.chair : lounge.spot;
          const end = working ? deskSeat(d!) : tileCentre(lounge.spot);
          const here = { tx: Math.floor(w.x / T), ty: Math.floor((w.y - 1) / T) };
          w.route = still ? [] : path(lay, here, to);
          w.goal = end;
          w.sitting = null;
          if (still || !w.route.length) {
            w.x = end.x;
            w.y = end.y;
            w.route = [];
          }
        }
        const next = w.route.length ? tileCentre(w.route[0]) : w.goal && (Math.abs(w.goal.x - w.x) > 0.01 || Math.abs(w.goal.y - w.y) > 0.01) ? w.goal : null;
        if (next) {
          const dx = next.x - w.x;
          const dy = next.y - w.y;
          const v = 0.045 * dt;
          if (Math.abs(dx) > 0.01) {
            w.x += Math.sign(dx) * Math.min(v, Math.abs(dx));
            w.facing = dx > 0 ? "right" : "left";
          } else if (Math.abs(dy) > 0.01) {
            w.y += Math.sign(dy) * Math.min(v, Math.abs(dy));
            w.facing = dy > 0 ? "down" : "up";
          } else if (w.route.length) {
            w.route.shift();
          }
        } else if (!w.sitting && w.goal) {
          // Arrived.
          w.goal = null;
          w.sitting = w.plan === "desk" ? "desk" : lounge.seat ? "sofa" : null;
          w.facing = w.plan === "desk" ? "up" : w.seatFacing;
        }
      }

      // delegation flows: Lead's chair -> each worker's chair, a dashed line marching towards the worker
      if (flows?.length) {
        g.save();
        g.lineWidth = 1;
        g.setLineDash([3, 3]);
        g.lineDashOffset = still ? 0 : -Math.floor(t / 60) % 6;
        g.strokeStyle = "#facc15cc";
        for (const [from, to] of flows) {
          const a = lay.desks.get(from);
          const b = lay.desks.get(to);
          if (!a || !b) continue;
          const p = deskSeat(a);
          const q = deskSeat(b);
          const midY = Math.round((p.y + (q.y - 18)) / 2) + 4;
          g.beginPath();
          g.moveTo(p.x + 0.5, p.y + 0.5);
          g.lineTo(p.x + 0.5, midY + 0.5);
          g.lineTo(q.x + 0.5, midY + 0.5);
          g.lineTo(q.x + 0.5, q.y - 30 + 0.5);
          g.stroke();
        }
        g.restore();
        for (const [from, to, ticket] of flows) {
          const a = lay.desks.get(from);
          const b = lay.desks.get(to);
          if (!a || !b || !ticket) continue;
          const p = deskSeat(a);
          const q = deskSeat(b);
          const midY = Math.round((p.y + (q.y - 18)) / 2) + 4;
          label(g, Math.round((p.x + q.x) / 2), midY - 3, ticket, "#facc15", "#1b1726");
        }
      }
      // desks and chairs, the sofa, then people by depth, bubbles on top
      for (const [id, d] of lay.desks) {
        const act = byId.get(id)?.activity ?? "idle";
        chair(g, deskSeat(d).x - T / 2, d.chair.ty * T, id === lead ? "#7a5bd6" : "#4f9a6a");
        desk(g, d.desk.tx * T, d.desk.ty * T, act, t, still, d.wide);
      }
      sofa(g, lay.sofaAt.tx * T, lay.sofaAt.ty * T);
      for (const a of lay.armchairs) armchair(g, a.spot.tx * T, a.spot.ty * T, a.facing);
      const hit: typeof hits.current = [];
      const order = [...agents].sort((a, b) => (walkers.current.get(a.agent.agent.id)?.y ?? 0) - (walkers.current.get(b.agent.agent.id)?.y ?? 0));
      for (const oa of order) {
        const id = oa.agent.agent.id;
        const w = walkers.current.get(id)!;
        const l = look(id);
        const walking = !w.sitting && (w.route.length > 0 || !!w.goal);
        const typing = w.sitting === "desk" && (oa.activity === "tool" || oa.activity === "shell" || oa.activity === "text") && !still;
        const sx = Math.round(w.x - 6);
        let sy = Math.round(w.y - 18);
        let rows = 18;
        if (w.sitting === "desk") {
          sy -= 3; // in the chair, legs under the desk
          rows = 15;
        } else if (w.sitting === "sofa") {
          sy -= 2;
          rows = 15;
        }
        const map = pose(w.facing, l, frame, walking, typing);
        // asleep at the desk: head down
        if (oa.activity === "off" && w.sitting === "desk") sprite(g, map, sx, sy + 3, l, false, rows - 3);
        else sprite(g, map, sx, sy, l, w.facing === "left", rows);
        if (id === selected) R(g, sx - 1, sy + rows + 1, 14, 1, "#3b82f6");
        if (id === lead) crown(g, sx + 2, sy - 14);
        const bx = sx + 7;
        const by = sy - (id === lead ? 26 : 19);
        if (oa.activity === "thinking") bubble(g, bx, by, "dots", Math.floor(t / 350));
        else if (oa.activity === "waiting") bubble(g, bx, by, "q", 0);
        else if (oa.activity === "shell") bubble(g, bx, by, "shell", frame);
        else if (oa.activity === "tool") bubble(g, bx, by, "tool", frame);
        else if (oa.activity === "text") bubble(g, bx, by, "pen", frame);
        else if (oa.activity === "off") bubble(g, bx, by, "z", frame);
        hit.push({ id, x: sx - 2, y: sy - 8, w: 16, h: rows + 10 });
      }
      // what each agent calls now, over its desk
      if (signs) {
        for (const [id, d] of lay.desks) {
          const sgn = signs[id];
          if (!sgn) continue;
          const w = d.wide ? 3 * T : 2 * T;
          sign(g, d.desk.tx * T + w / 2, d.desk.ty * T - 16, sgn.kind, sgn.text);
        }
      }
      // names over the heads (after the signs, so they stay readable); a name that would cover another one (two
      // agents side by side on the sofa) moves up a row
      const placed: { x: number; y: number; w: number }[] = [];
      for (const oa of agents) {
        const id = oa.agent.agent.id;
        const w = walkers.current.get(id)!;
        const top = Math.round(w.y - 18) - (w.sitting === "desk" ? 3 : w.sitting === "sofa" ? 2 : 0);
        const lw = labelWidth(g, oa.agent.agent.name);
        const lx = Math.round(w.x - lw / 2);
        // The name pill and the vendor chip left of it (8 px) take this box.
        const bx = lx - 8;
        const bw = lw + 8;
        let ly = top - 7;
        while (placed.some((p) => bx < p.x + p.w + 1 && p.x < bx + bw + 1 && Math.abs(p.y - ly) < 7)) ly -= 7;
        placed.push({ x: bx, y: ly, w: bw });
        label(g, Math.round(w.x), ly, oa.agent.agent.name, id === selected ? "#2563eb" : "#111827e6", "#ffffff");
        // The vendor behind this agent's model, as a small chip left of the name, centred on the pill.
        vendorChip(g, lx - 1, ly + 3, oa.agent.agent.model.provider, oa.agent.agent.model.model);
      }
      // envelopes: ticket comments between a Lead and a worker
      const now = performance.now();
      flying.current = flying.current.filter((f) => now - f.start < 1500);
      for (const f of flying.current) {
        const a = walkers.current.get(f.from);
        const b = walkers.current.get(f.to);
        if (!a || !b) continue;
        const k = Math.min(1, (now - f.start) / 1300);
        const e = k < 0.5 ? 2 * k * k : 1 - (-2 * k + 2) ** 2 / 2;
        if (still) envelope(g, Math.round(b.x), Math.round(b.y - 26));
        else envelope(g, Math.round(a.x + (b.x - a.x) * e), Math.round(a.y - 20 + (b.y - a.y) * e - Math.sin(Math.PI * e) * 22));
      }
      hits.current = hit;
      raf = requestAnimationFrame(draw);
    };
    raf = requestAnimationFrame(draw);
    return () => cancelAnimationFrame(raf);
  }, []);

  const at = (e: React.MouseEvent) => {
    const r = canvas.current!.getBoundingClientRect();
    const x = (e.clientX - r.left) / scale;
    const y = (e.clientY - r.top) / scale;
    return hits.current.find((h) => x >= h.x && x <= h.x + h.w && y >= h.y && y <= h.y + h.h);
  };
  const hovered = hover && agents.find((a) => a.agent.agent.id === hover.id);
  const W = lay.cols * T * scale;
  const H = lay.rows * T * scale;
  return (
    <div ref={wrap} className="relative flex w-full justify-center">
      <canvas
        ref={canvas}
        width={W}
        height={H}
        style={{ width: W, height: H, imageRendering: "pixelated", cursor: hover ? "pointer" : "default" }}
        onMouseMove={(e) => {
          const h = at(e);
          const r = wrap.current!.getBoundingClientRect();
          setHover(h ? { id: h.id, x: e.clientX - r.left, y: e.clientY - r.top } : null);
        }}
        onMouseLeave={() => setHover(null)}
        onClick={(e) => {
          const h = at(e);
          if (h) onSelect(h.id);
        }}
      />
      {hovered && (
        <div className="pointer-events-none absolute z-10 max-w-xs rounded-lg border border-line-strong bg-surface px-2.5 py-1.5 text-[11px] shadow-lg" style={{ left: Math.min(hover!.x + 12, width - 200), top: hover!.y + 12 }}>
          <div className="font-semibold">{hovered.agent.agent.name}</div>
          <div className="truncate font-mono text-[11px] text-muted">{hovered.ticker}</div>
        </div>
      )}
    </div>
  );
}
