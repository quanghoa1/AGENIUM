// The Map page's canvas (ADR-166): nodes with an icon per kind, links as arrows named by their type, dashed when
// tentative or stale; a selected node dims what does not touch it, a found path is drawn on top. Scroll to zoom, drag
// to move, right-click a node for the path menu. No graph library: an SVG, laid out by `layout.ts`.
import { useEffect, useRef, useState } from "react";
import { cn } from "../../components/ui";
import type { MapLinkView } from "../../lib/types";
import { colourOf, iconOf } from "./kinds";
import { H, W, label, type Placed } from "./layout";

export function MapCanvas({
  nodes,
  links,
  selected,
  path,
  onSelect,
  onMenu,
  fitKey,
}: {
  nodes: Placed[];
  links: MapLinkView[];
  selected: string | null;
  path: Set<number>;
  onSelect: (key: string | null) => void;
  onMenu: (key: string, x: number, y: number) => void;
  /** Changes when the view is to be fitted again. */
  fitKey: string;
}) {
  const svg = useRef<SVGSVGElement>(null);
  const drag = useRef<{ x: number; y: number; vx: number; vy: number } | null>(null);
  const [view, setView] = useState({ x: 0, y: 0, k: 1 });
  const [hover, setHover] = useState<string | null>(null);
  useEffect(() => setView({ x: 0, y: 0, k: 1 }), [fitKey]);
  useEffect(() => {
    const el = svg.current;
    if (!el) return;
    const wheel = (e: WheelEvent) => {
      e.preventDefault();
      setView((v) => ({ ...v, k: Math.max(0.3, Math.min(4, v.k * (e.deltaY < 0 ? 1.12 : 1 / 1.12))) }));
    };
    el.addEventListener("wheel", wheel, { passive: false });
    return () => el.removeEventListener("wheel", wheel);
  }, []);
  const at = new Map(nodes.map((n) => [n.key, n]));
  const colour = colourOf(nodes.map((n) => n.kind));
  const focus = selected ?? hover;
  const near = new Set<string>();
  if (focus) {
    near.add(focus);
    for (const l of links) if (l.src === focus || l.dst === focus) (near.add(l.src), near.add(l.dst));
  }
  const pathKeys = new Set(links.filter((l) => path.has(l.id)).flatMap((l) => [l.src, l.dst]));
  const dimmed = (k: string) => (path.size > 0 ? !pathKeys.has(k) : focus ? !near.has(k) : false);
  const labelled = links.length <= 40;
  return (
    <svg
      ref={svg}
      role="img"
      aria-label={`Project map with ${nodes.length} nodes`}
      viewBox={`0 0 ${W} ${H}`}
      className={cn("block h-full w-full select-none", drag.current ? "cursor-grabbing" : "cursor-grab")}
      onPointerDown={(e) => {
        if (e.button !== 0) return;
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
        if (d && Math.abs(e.clientX - d.x) + Math.abs(e.clientY - d.y) < 4 && e.target === svg.current) onSelect(null);
      }}
    >
      <defs>
        <marker id="map-arrow" viewBox="0 0 10 10" refX="10" refY="5" markerWidth="7" markerHeight="7" orient="auto-start-reverse">
          <path d="M0 0L10 5L0 10z" fill="var(--text-muted)" />
        </marker>
        <marker id="map-arrow-on" viewBox="0 0 10 10" refX="10" refY="5" markerWidth="7" markerHeight="7" orient="auto-start-reverse">
          <path d="M0 0L10 5L0 10z" fill="#3B82F6" />
        </marker>
      </defs>
      <g transform={`translate(${W / 2} ${H / 2}) scale(${view.k}) translate(${-W / 2 + view.x} ${-H / 2 + view.y})`}>
        {links.map((l) => {
          const a = at.get(l.src), b = at.get(l.dst);
          if (!a || !b) return null;
          const onPath = path.has(l.id);
          const on = onPath || (path.size === 0 && !!focus && (l.src === focus || l.dst === focus));
          const dx = b.x - a.x, dy = b.y - a.y, d = Math.max(Math.hypot(dx, dy), 1);
          const r = 22;
          const [x1, y1, x2, y2] = [a.x + (dx / d) * r, a.y + (dy / d) * r, b.x - (dx / d) * r, b.y - (dy / d) * r];
          const faded = path.size > 0 ? !onPath : !!focus && !on;
          return (
            <g key={l.id} opacity={faded ? 0.12 : 1} data-link={l.kind}>
              <line
                x1={x1} y1={y1} x2={x2} y2={y2}
                stroke={on ? "#3B82F6" : l.status === "stale" ? "#d97706" : "var(--border-strong)"}
                strokeWidth={onPath ? 3 : on ? 2 : 1.3}
                strokeDasharray={l.status === "confirmed" ? undefined : "6 5"}
                markerEnd={`url(#${on ? "map-arrow-on" : "map-arrow"})`}
              />
              {(labelled || on) && (
                <text x={(x1 + x2) / 2} y={(y1 + y2) / 2 - 5} textAnchor="middle" className="fill-[var(--text-secondary)] text-[12px]" paintOrder="stroke" stroke="var(--bg-surface)" strokeWidth={4}>
                  {l.kind.replaceAll("_", " ")}
                </text>
              )}
            </g>
          );
        })}
        {nodes.map((n) => {
          const Icon = iconOf(n.kind);
          const active = n.key === selected;
          const unsure = n.status !== "confirmed";
          return (
            <g
              key={n.key}
              data-key={n.key}
              transform={`translate(${n.x} ${n.y})`}
              opacity={dimmed(n.key) ? 0.2 : 1}
              className="cursor-pointer"
              onPointerDown={(e) => e.stopPropagation()}
              onClick={() => onSelect(active ? null : n.key)}
              onContextMenu={(e) => {
                e.preventDefault();
                onMenu(n.key, e.clientX, e.clientY);
              }}
              onPointerEnter={() => setHover(n.key)}
              onPointerLeave={() => setHover((h) => (h === n.key ? null : h))}
            >
              <title>{`${n.kind}: ${n.name}${n.summary ? `\n${n.summary}` : ""}`}</title>
              <circle
                r={active ? 22 : 18}
                fill="var(--bg-surface)"
                stroke={colour(n.kind)}
                strokeWidth={active ? 3.5 : 2}
                strokeDasharray={unsure ? "4 3" : undefined}
              />
              <Icon x={-9} y={-9} width={18} height={18} color={colour(n.kind)} strokeWidth={2} />
              <text y={36} textAnchor="middle" className={cn("text-[12px]", active ? "fill-[var(--text-primary)] font-medium" : "fill-[var(--text-secondary)]")} paintOrder="stroke" stroke="var(--bg-surface)" strokeWidth={4}>
                {label(n)}
              </text>
            </g>
          );
        })}
      </g>
    </svg>
  );
}
