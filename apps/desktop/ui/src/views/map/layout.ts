// Where the Map page puts its nodes (ADR-166). Sequential (the default, as BloodHound's): columns along the links'
// direction, what a node leads to on its right and what leads to it on its left, each column ordered by where its
// neighbours sit. Free: a small force layout (springs along links, every pair pushing apart, a pull to the middle).
import type { MapLinkView, MapNodeView } from "../../lib/types";

export const W = 1600;
export const H = 900;

export type Placed = MapNodeView & { x: number; y: number };

/** Columns along the links from `root` (or from the nodes nothing leads to), ordered to cross few links. */
export function sequential(nodes: MapNodeView[], links: MapLinkView[], root?: string | null): Placed[] {
  const keys = new Set(nodes.map((n) => n.key));
  const ls = links.filter((l) => keys.has(l.src) && keys.has(l.dst));
  const out = new Map<string, string[]>(), into = new Map<string, string[]>();
  for (const l of ls) {
    out.set(l.src, [...(out.get(l.src) ?? []), l.dst]);
    into.set(l.dst, [...(into.get(l.dst) ?? []), l.src]);
  }
  const layer = new Map<string, number>();
  const walk = (start: string[], step: (k: string) => string[], dir: number) => {
    const q = [...start];
    while (q.length) {
      const k = q.shift()!;
      for (const n of step(k)) {
        if (!layer.has(n)) {
          layer.set(n, layer.get(k)! + dir);
          q.push(n);
        }
      }
    }
  };
  const starts = root && keys.has(root) ? [root] : nodes.filter((n) => !into.get(n.key)?.length).map((n) => n.key);
  for (const s of starts.length ? starts : nodes.slice(0, 1).map((n) => n.key)) if (!layer.has(s)) layer.set(s, 0);
  walk([...layer.keys()], (k) => out.get(k) ?? [], 1);
  walk([...layer.keys()], (k) => into.get(k) ?? [], -1);
  // A part the links do not reach from there (another component, a cycle): its own columns from its first node.
  for (const n of nodes) {
    if (layer.has(n.key)) continue;
    layer.set(n.key, 0);
    walk([n.key], (k) => out.get(k) ?? [], 1);
    walk([n.key], (k) => into.get(k) ?? [], -1);
  }
  const lo = Math.min(...layer.values()), hi = Math.max(...layer.values());
  const cols: string[][] = Array.from({ length: hi - lo + 1 }, () => []);
  for (const n of nodes) cols[layer.get(n.key)! - lo].push(n.key);
  // Each column after the first is ordered by the mean row of its neighbours in the column before (one sweep).
  const row = new Map<string, number>();
  cols.forEach((c, ci) => {
    if (ci > 0) {
      const mean = (k: string) => {
        const ns = [...(into.get(k) ?? []), ...(out.get(k) ?? [])].filter((n) => layer.get(n)! - lo === ci - 1);
        return ns.length ? ns.reduce((s, n) => s + (row.get(n) ?? 0), 0) / ns.length : Number.MAX_SAFE_INTEGER;
      };
      c.sort((a, b) => mean(a) - mean(b));
    }
    c.forEach((k, i) => row.set(k, i));
  });
  const byKey = new Map(nodes.map((n) => [n.key, n]));
  const gapX = cols.length > 1 ? Math.min(320, (W - 240) / (cols.length - 1)) : 0;
  const x0 = (W - gapX * (cols.length - 1)) / 2;
  const placed: Placed[] = [];
  cols.forEach((c, ci) => {
    const gapY = Math.min(110, (H - 120) / Math.max(c.length, 1));
    const y0 = (H - gapY * (c.length - 1)) / 2;
    c.forEach((k, i) => placed.push({ ...byKey.get(k)!, x: x0 + ci * gapX, y: y0 + i * gapY }));
  });
  return placed;
}

/** Springs along the links, repulsion between all, a pull to the middle; deterministic. */
export function force(nodes: MapNodeView[], links: MapLinkView[]): Placed[] {
  const n = nodes.length;
  const p = nodes.map((node, i) => {
    const a = (i / Math.max(1, n)) * Math.PI * 2 * 7.3;
    const r = 80 + 340 * Math.sqrt(i / Math.max(1, n));
    return { ...node, x: W / 2 + Math.cos(a) * r, y: H / 2 + Math.sin(a) * r * 0.6, vx: 0, vy: 0 };
  });
  const at = new Map(p.map((q, i) => [q.key, i]));
  const pairs = links.filter((l) => at.has(l.src) && at.has(l.dst)).map((l) => [at.get(l.src)!, at.get(l.dst)!] as const);
  const steps = n > 150 ? 140 : 260;
  for (let s = 0; s < steps; s++) {
    const t = 1 - s / steps;
    for (let i = 0; i < n; i++) {
      for (let j = i + 1; j < n; j++) {
        const a = p[i], b = p[j];
        let dx = a.x - b.x, dy = a.y - b.y;
        const d2 = Math.max(dx * dx + dy * dy, 25);
        const f = 3600 / d2, d = Math.sqrt(d2);
        dx /= d; dy /= d;
        a.vx += dx * f; a.vy += dy * f; b.vx -= dx * f; b.vy -= dy * f;
      }
    }
    for (const [i, j] of pairs) {
      const a = p[i], b = p[j];
      const dx = b.x - a.x, dy = b.y - a.y;
      const d = Math.max(Math.sqrt(dx * dx + dy * dy), 1);
      const f = (d - 160) * 0.02;
      a.vx += (dx / d) * f; a.vy += (dy / d) * f; b.vx -= (dx / d) * f; b.vy -= (dy / d) * f;
    }
    for (const q of p) {
      q.vx += (W / 2 - q.x) * 0.002;
      q.vy += (H / 2 - q.y) * 0.003;
      q.x += Math.max(-30, Math.min(30, q.vx)) * t;
      q.y += Math.max(-30, Math.min(30, q.vy)) * t;
      q.vx *= 0.5; q.vy *= 0.5;
    }
  }
  if (n > 1) {
    const xs = p.map((q) => q.x), ys = p.map((q) => q.y);
    const [x0, x1, y0, y1] = [Math.min(...xs), Math.max(...xs), Math.min(...ys), Math.max(...ys)];
    const k = Math.min((W - 240) / Math.max(x1 - x0, 1), (H - 120) / Math.max(y1 - y0, 1), 2.5);
    const ox = (W - (x1 - x0) * k) / 2, oy = (H - (y1 - y0) * k) / 2;
    for (const q of p) { q.x = ox + (q.x - x0) * k; q.y = oy + (q.y - y0) * k; }
  }
  return p.map(({ vx: _vx, vy: _vy, ...q }) => q);
}

/** A short label for a node: a symbol's own name, a file's name, anything else its name. */
export function label(n: MapNodeView): string {
  const s = n.kind === "symbol" ? n.name.split("#").pop()! : n.kind === "file" ? n.name.split("/").pop()! : n.name;
  return s.length > 28 ? `${s.slice(0, 27)}…` : s;
}
