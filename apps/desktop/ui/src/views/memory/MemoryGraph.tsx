// The Memory page's graph tab: how an agent's memories hang together. Entities (what the memory is about, coloured by
// type) are joined by their typed edges; each fact, lesson or preference is tied to the entities it names; pairs of
// memories the settling pass judged are joined too (a conflict in red). Click a node for what it is and what it
// touches; scroll to zoom, drag to move. No graph library: a small force layout, run once per data.
import { Network } from "lucide-react";
import { useEffect, useMemo, useRef, useState } from "react";
import { Empty, cn } from "../../components/ui";
import { api } from "../../lib/api";
import { useApp } from "../../lib/store";
import type { Fact, MemoryGraphData } from "../../lib/types";

type Node = { key: string; kind: "entity" | "memory"; label: string; group: string; x: number; y: number; vx: number; vy: number; ref: number };
type Link = { a: string; b: string; kind: "edge" | "about" | "conflict" | "distinct"; label: string };

const SERIES = 7;
const W = 1600;
const H = 900;

/** Positions for the nodes: spring links, repulsion between all, a pull to the middle. Deterministic. */
function layout(nodes: Node[], links: Link[]) {
  const n = nodes.length;
  nodes.forEach((p, i) => {
    const a = (i / Math.max(1, n)) * Math.PI * 2 * 7.3;
    const r = 80 + 340 * Math.sqrt(i / Math.max(1, n));
    p.x = W / 2 + Math.cos(a) * r;
    p.y = H / 2 + Math.sin(a) * r * 0.6;
  });
  const at = new Map(nodes.map((p, i) => [p.key, i]));
  const pairs = links.map((l) => [at.get(l.a)!, at.get(l.b)!, l.kind === "about" ? 70 : 110] as const);
  // shortcut: O(n²) repulsion; fine for the few hundred nodes the page shows, a quadtree past a thousand.
  const steps = n > 400 ? 120 : 260;
  for (let s = 0; s < steps; s++) {
    const t = 1 - s / steps;
    for (let i = 0; i < n; i++) {
      for (let j = i + 1; j < n; j++) {
        const p = nodes[i], q = nodes[j];
        let dx = p.x - q.x, dy = p.y - q.y;
        const d2 = Math.max(dx * dx + dy * dy, 25);
        const f = 2600 / d2;
        const d = Math.sqrt(d2);
        dx /= d; dy /= d;
        p.vx += dx * f; p.vy += dy * f; q.vx -= dx * f; q.vy -= dy * f;
      }
    }
    for (const [i, j, len] of pairs) {
      const p = nodes[i], q = nodes[j];
      const dx = q.x - p.x, dy = q.y - p.y;
      const d = Math.max(Math.sqrt(dx * dx + dy * dy), 1);
      const f = (d - len) * 0.02;
      p.vx += (dx / d) * f; p.vy += (dy / d) * f; q.vx -= (dx / d) * f; q.vy -= (dy / d) * f;
    }
    for (const p of nodes) {
      p.vx += (W / 2 - p.x) * 0.002;
      p.vy += (H / 2 - p.y) * 0.003;
      p.x += Math.max(-30, Math.min(30, p.vx)) * t;
      p.y += Math.max(-30, Math.min(30, p.vy)) * t;
      p.vx *= 0.5; p.vy *= 0.5;
    }
  }
  // Fill the frame (the same scale both ways): the labels keep their size, the nodes get the room between them.
  if (n < 2) return;
  const xs = nodes.map((p) => p.x), ys = nodes.map((p) => p.y);
  const [x0, x1, y0, y1] = [Math.min(...xs), Math.max(...xs), Math.min(...ys), Math.max(...ys)];
  const padX = 120, padY = 50;
  const k = Math.min((W - 2 * padX) / Math.max(x1 - x0, 1), (H - 2 * padY) / Math.max(y1 - y0, 1), 3);
  const ox = (W - (x1 - x0) * k) / 2, oy = (H - (y1 - y0) * k) / 2;
  for (const p of nodes) {
    p.x = ox + (p.x - x0) * k;
    p.y = oy + (p.y - y0) * k;
  }
}

/** The graph's nodes and links from the page's facts and the graph rows; memories that name no entity and have no
 *  pair are left out (counted). */
export function buildGraph(facts: Fact[], g: MemoryGraphData) {
  const live = new Map(facts.map((f) => [f.id, f]));
  const links: Link[] = [];
  const memIds = new Set<number>();
  for (const e of g.edges) links.push({ a: `e${e.src}`, b: `e${e.dst}`, kind: "edge", label: e.kind.replaceAll("_", " ") });
  for (const [f, e, role] of g.fact_links) {
    if (!live.has(f)) continue;
    memIds.add(f);
    links.push({ a: `m${f}`, b: `e${e}`, kind: "about", label: role });
  }
  for (const [a, b, v] of g.fact_pairs) {
    if (!live.has(a) || !live.has(b)) continue;
    memIds.add(a); memIds.add(b);
    links.push({ a: `m${a}`, b: `m${b}`, kind: v === "conflict" ? "conflict" : "distinct", label: v });
  }
  const types = [...new Set(g.entities.map((e) => e.kind))].sort();
  const nodes: Node[] = [
    ...g.entities.map((e) => ({ key: `e${e.id}`, kind: "entity" as const, label: e.name, group: e.kind, x: 0, y: 0, vx: 0, vy: 0, ref: e.id })),
    ...[...memIds].map((id) => {
      const f = live.get(id)!;
      return { key: `m${id}`, kind: "memory" as const, label: f.text, group: f.kind, x: 0, y: 0, vx: 0, vy: 0, ref: id };
    }),
  ];
  return { nodes, links, types, unlinked: facts.length - memIds.size };
}

const short = (s: string, n: number) => (s.length > n ? `${s.slice(0, n - 1)}…` : s);
const KIND_STROKE: Record<string, string> = { lesson: "#eda100", preference: "#e87ba4", fact: "var(--text-secondary)" };

export function MemoryGraph({ agent, facts, query }: { agent: string; facts: Fact[]; query: string }) {
  const app = useApp();
  const [g, setG] = useState<MemoryGraphData | null>(null);
  const [sel, setSel] = useState<string | null>(null);
  const [hover, setHover] = useState<string | null>(null);
  const [view, setView] = useState({ x: 0, y: 0, k: 1 });
  const drag = useRef<{ x: number; y: number; vx: number; vy: number } | null>(null);
  const svg = useRef<SVGSVGElement>(null);
  useEffect(() => {
    setG(null);
    setSel(null);
    api.memoryGraph(agent).then(setG).catch((e) => app.notify(String(e)));
  }, [agent, app.tick]);
  const built = useMemo(() => {
    if (!g) return null;
    const b = buildGraph(facts, g);
    layout(b.nodes, b.links);
    return b;
  }, [g, facts]);
  // A wheel zoom that does not scroll the page.
  useEffect(() => {
    const el = svg.current;
    if (!el) return;
    const wheel = (e: WheelEvent) => {
      e.preventDefault();
      setView((v) => ({ ...v, k: Math.max(0.3, Math.min(4, v.k * (e.deltaY < 0 ? 1.12 : 1 / 1.12))) }));
    };
    el.addEventListener("wheel", wheel, { passive: false });
    return () => el.removeEventListener("wheel", wheel);
  }, [built]);
  if (!g || !built) return <div className="text-xs text-subtle">{app.t("common.loading")}</div>;
  if (built.nodes.length === 0) return <Empty>{app.t("memory.graphEmpty")}</Empty>;

  const byKey = new Map(built.nodes.map((n) => [n.key, n]));
  const focus = sel ?? hover;
  const near = new Set<string>();
  if (focus) {
    near.add(focus);
    for (const l of built.links) if (l.a === focus || l.b === focus) (near.add(l.a), near.add(l.b));
  }
  const q = query.trim().toLowerCase();
  const hit = (n: Node) => !q || n.label.toLowerCase().includes(q);
  const color = (group: string) => {
    const i = built.types.indexOf(group);
    return i >= 0 && i < SERIES ? `var(--series-${i + 1})` : "var(--series-other)";
  };
  const dim = (key: string) => (focus ? !near.has(key) : false);
  const selected = sel ? byKey.get(sel) : undefined;
  const neighbours = selected
    ? built.links
        .filter((l) => l.a === selected.key || l.b === selected.key)
        .map((l) => ({ l, other: byKey.get(l.a === selected.key ? l.b : l.a)!, out: l.a === selected.key }))
    : [];
  const fact = selected?.kind === "memory" ? facts.find((f) => f.id === selected.ref) : undefined;
  const entity = selected?.kind === "entity" ? g.entities.find((e) => e.id === selected.ref) : undefined;

  return (
    <div className="grid gap-3 xl:grid-cols-[minmax(0,1fr)_340px]">
      <div className="relative overflow-hidden rounded-lg border border-line bg-surface">
        <svg
          ref={svg}
          role="img"
          aria-label={app.t("memory.graphAria").replace("{n}", String(built.nodes.length))}
          viewBox={`0 0 ${W} ${H}`}
          className={cn("block h-[min(72vh,900px)] w-full select-none", drag.current ? "cursor-grabbing" : "cursor-grab")}
          onPointerDown={(e) => {
            (e.target as Element).setPointerCapture?.(e.pointerId);
            drag.current = { x: e.clientX, y: e.clientY, vx: view.x, vy: view.y };
          }}
          onPointerMove={(e) => {
            const d = drag.current;
            const r = svg.current?.getBoundingClientRect();
            if (!d || !r) return;
            const s = W / r.width / view.k;
            setView((v) => ({ ...v, x: d.vx + (e.clientX - d.x) * s, y: d.vy + (e.clientY - d.y) * s }));
          }}
          onPointerUp={(e) => {
            const d = drag.current;
            drag.current = null;
            // A click (no drag) on the background clears the selection.
            if (d && Math.abs(e.clientX - d.x) + Math.abs(e.clientY - d.y) < 4 && e.target === svg.current) setSel(null);
          }}
        >
          <g transform={`translate(${W / 2} ${H / 2}) scale(${view.k}) translate(${-W / 2 + view.x} ${-H / 2 + view.y})`}>
            {built.links.map((l, i) => {
              const a = byKey.get(l.a)!, b = byKey.get(l.b)!;
              const on = focus && (l.a === focus || l.b === focus);
              const stroke = l.kind === "conflict" ? "#e5484d" : l.kind === "edge" ? "var(--border-strong)" : "var(--text-muted)";
              return (
                <g key={i} opacity={focus && !on ? 0.12 : l.kind === "about" ? 0.55 : 0.9}>
                  <line
                    x1={a.x} y1={a.y} x2={b.x} y2={b.y}
                    stroke={stroke}
                    strokeWidth={l.kind === "conflict" ? 2.4 : on ? 2 : 1.3}
                    strokeDasharray={l.kind === "about" ? "4 4" : l.kind === "distinct" ? "1.5 4" : undefined}
                  />
                  {on && l.kind !== "about" && (
                    <text x={(a.x + b.x) / 2} y={(a.y + b.y) / 2 - 4} textAnchor="middle" className="fill-[var(--text-secondary)] text-[11px]">
                      {l.label}
                    </text>
                  )}
                </g>
              );
            })}
            {built.nodes.map((n) => {
              const faded = dim(n.key) || !hit(n);
              const active = n.key === sel;
              const label = n.kind === "entity" || active || n.key === hover;
              return (
                <g
                  key={n.key}
                  transform={`translate(${n.x} ${n.y})`}
                  opacity={faded ? 0.2 : 1}
                  className="cursor-pointer"
                  onPointerDown={(e) => e.stopPropagation()}
                  onClick={() => setSel(active ? null : n.key)}
                  onPointerEnter={() => setHover(n.key)}
                  onPointerLeave={() => setHover((h) => (h === n.key ? null : h))}
                >
                  <title>{n.kind === "entity" ? `${n.label} (${n.group})` : `${n.group}: ${n.label}`}</title>
                  {n.kind === "entity" ? (
                    <circle r={active ? 11 : 8} fill={color(n.group)} stroke={active ? "var(--text-primary)" : "var(--bg-surface)"} strokeWidth={2} />
                  ) : (
                    <rect x={-6} y={-6} width={12} height={12} rx={3} fill="var(--bg-elevated)" stroke={KIND_STROKE[n.group] ?? "var(--text-secondary)"} strokeWidth={active ? 3 : 2} />
                  )}
                  {label && (
                    <text
                      y={n.kind === "entity" ? -13 : -11}
                      textAnchor="middle"
                      className={cn("text-[12px]", n.kind === "entity" ? "fill-[var(--text-primary)] font-medium" : "fill-[var(--text-secondary)]")}
                      paintOrder="stroke"
                      stroke="var(--bg-surface)"
                      strokeWidth={4}
                    >
                      {short(n.label, n.kind === "entity" ? 28 : 64)}
                    </text>
                  )}
                </g>
              );
            })}
          </g>
        </svg>
        <div className="pointer-events-none absolute bottom-2 left-2 flex max-w-[70%] flex-wrap gap-x-3 gap-y-1 rounded-md bg-surface/90 px-2 py-1.5 text-[11px] text-muted">
          {built.types.map((t) => (
            <span key={t} className="flex items-center gap-1">
              <span className="inline-block h-2.5 w-2.5 rounded-full" style={{ background: color(t) }} />
              {t}
            </span>
          ))}
          {(["fact", "lesson", "preference"] as const).map((k) => (
            <span key={k} className="flex items-center gap-1">
              <span className="inline-block h-2.5 w-2.5 rounded-[3px] border-2" style={{ borderColor: KIND_STROKE[k] }} />
              {app.t(`memory.kind.${k}`)}
            </span>
          ))}
          <span className="flex items-center gap-1"><span className="inline-block h-0.5 w-4 bg-[#e5484d]" />{app.t("memory.graphConflict")}</span>
        </div>
        <div className="absolute right-2 top-2 flex gap-1">
          <button className="btn" onClick={() => setView({ x: 0, y: 0, k: 1 })}>{app.t("memory.graphReset")}</button>
        </div>
      </div>
      <div className="space-y-2 rounded-lg border border-line bg-elevated/40 p-3 text-xs">
        <div className="flex items-center gap-1.5 font-semibold">
          <Network className="h-3.5 w-3.5 text-accent" /> {app.t("memory.graphCounts").replace("{e}", String(g.entities.length)).replace("{m}", String(built.nodes.length - g.entities.length)).replace("{l}", String(built.links.length))}
        </div>
        {(built.unlinked > 0 || g.more > 0) && (
          <div className="text-[11px] text-subtle">
            {built.unlinked > 0 && app.t("memory.graphUnlinked").replace("{n}", String(built.unlinked))} {g.more > 0 && app.t("memory.graphMore").replace("{n}", String(g.more))}
          </div>
        )}
        {!selected ? (
          <div className="text-[11px] leading-relaxed text-muted">{app.t("memory.graphHint")}</div>
        ) : (
          <div className="space-y-2">
            <div className="rounded-md border border-line bg-surface p-2">
              <div className="text-[11px] uppercase tracking-wide text-subtle">{selected.kind === "entity" ? selected.group : app.t(`memory.kind.${selected.group as Fact["kind"]}`)}</div>
              <div className="mt-0.5 text-[13px] font-medium leading-snug">{selected.label}</div>
              {entity?.description && <div className="mt-1 text-muted">{entity.description}</div>}
              {entity && <div className="mt-1 text-[11px] text-subtle">{app.t("memory.graphMentions").replace("{n}", String(entity.mentions))}</div>}
              {fact && (
                <div className="mt-1 text-[11px] text-subtle">
                  {fact.source} · {app.t("memory.confidence")} {Math.round(fact.confidence * 100)}% · {app.t("memory.weight")} {Math.round(fact.weight * 100)}%
                </div>
              )}
            </div>
            <ul className="max-h-[52vh] space-y-1 overflow-auto">
              {neighbours.map(({ l, other, out }, i) => (
                <li key={i}>
                  <button className="flex w-full items-start gap-2 rounded-md px-1.5 py-1 text-left hover:bg-surface" onClick={() => setSel(other.key)}>
                    <span className={cn("mt-0.5 shrink-0 font-mono text-[10px]", l.kind === "conflict" ? "text-red-500" : "text-subtle")}>
                      {l.kind === "about" ? (out ? app.t("memory.graphAbout") : app.t("memory.graphNamedBy")) : `${out ? "→" : "←"} ${l.label}`}
                    </span>
                    <span className="min-w-0 flex-1 leading-snug">{short(other.label, 140)}</span>
                  </button>
                </li>
              ))}
            </ul>
          </div>
        )}
      </div>
    </div>
  );
}
