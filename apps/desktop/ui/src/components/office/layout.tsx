import { Facing, T } from "./sprites";

interface Spot {
  tx: number;
  ty: number;
}

interface Layout {
  cols: number;
  rows: number;
  /** tiles taken by furniture */
  blocked: Set<string>;
  /** agent id -> its desk (left tile) and chair */
  desks: Map<string, { desk: Spot; chair: Spot; wide: boolean }>;
  /** Places to sit in the lounge (sofa, armchairs) or stand on the rug, with the way the agent faces there. */
  loungeSeats: { spot: Spot; facing: Facing; seat: boolean }[];
  armchairs: { spot: Spot; facing: Facing }[];
  plants: Spot[];
  rugAt: { tx: number; ty: number; w: number; h: number };
  sofaAt: Spot;
  tableAt: Spot;
  coolerAt: Spot;
}

export const key = (x: number, y: number) => `${x},${y}`;

export function buildLayout(cols: number, ids: string[], lead: string | null | undefined, deskless: Set<string>): Layout {
  const blocked = new Set<string>();
  const desks = new Map<string, { desk: Spot; chair: Spot; wide: boolean }>();
  let y = 3;
  const seated = ids.filter((i) => !deskless.has(i));
  const workers = seated.filter((i) => i !== lead);
  if (lead && seated.includes(lead)) {
    const dx = Math.floor((cols - 3) / 2);
    desks.set(lead, { desk: { tx: dx, ty: y }, chair: { tx: dx + 1, ty: y + 1 }, wide: true });
    for (let i = 0; i < 3; i++) blocked.add(key(dx + i, y));
    y += 3;
  }
  // Worker desks two tiles wide with a walking tile between them, and a walking row under each chair row.
  const slots: number[] = [];
  for (let x = 1; x + 2 <= cols - 1; x += 3) slots.push(x);
  workers.forEach((id, i) => {
    const sx = slots[i % slots.length];
    const sy = y + Math.floor(i / slots.length) * 3;
    desks.set(id, { desk: { tx: sx, ty: sy }, chair: { tx: sx + 1, ty: sy + 1 }, wide: false });
    blocked.add(key(sx, sy));
    blocked.add(key(sx + 1, sy));
  });
  if (workers.length) y += Math.ceil(workers.length / slots.length) * 3;
  // Lounge at the bottom right: rug, sofa, table; a water cooler on the left; plants in the corners.
  const ly = y;
  const rugW = Math.min(6, cols - 3);
  const rx = cols - rugW - 1;
  const sofaAt = { tx: rx + Math.floor((rugW - 2) / 2), ty: ly };
  const tableAt = { tx: sofaAt.tx, ty: ly + 2 };
  blocked.add(key(sofaAt.tx, sofaAt.ty));
  blocked.add(key(sofaAt.tx + 1, sofaAt.ty));
  blocked.add(key(tableAt.tx, tableAt.ty));
  blocked.add(key(tableAt.tx + 1, tableAt.ty));
  const coolerAt = { tx: 1, ty: ly + 1 };
  blocked.add(key(coolerAt.tx, coolerAt.ty));
  const rows = ly + 4;
  const plants = [
    { tx: 0, ty: 2 },
    { tx: cols - 1, ty: 2 },
    { tx: 0, ty: rows - 1 },
  ];
  for (const p of plants) blocked.add(key(p.tx, p.ty));
  // Armchairs left and right of the table, turned towards it.
  const armchairs: { spot: Spot; facing: Facing }[] = [];
  for (const [ax, facing] of [
    [tableAt.tx - 1, "right"],
    [tableAt.tx + 2, "left"],
  ] as const) {
    if (ax > 0 && ax < cols - 1 && !blocked.has(key(ax, tableAt.ty))) {
      armchairs.push({ spot: { tx: ax, ty: tableAt.ty }, facing });
      blocked.add(key(ax, tableAt.ty));
    }
  }
  // Sofa and armchair seats first, then standing places on the rug.
  const loungeSeats: { spot: Spot; facing: Facing; seat: boolean }[] = [
    { spot: { tx: sofaAt.tx, ty: sofaAt.ty }, facing: "down", seat: true },
    { spot: { tx: sofaAt.tx + 1, ty: sofaAt.ty }, facing: "down", seat: true },
    ...armchairs.map((a) => ({ ...a, seat: true })),
  ];
  for (let x = rx; x < rx + rugW; x++) {
    if (!blocked.has(key(x, ly + 1))) loungeSeats.push({ spot: { tx: x, ty: ly + 1 }, facing: "down", seat: false });
  }
  // More room to stand around the lounge when many agents are free.
  for (let x = rx; x < rx + rugW; x++) {
    if (!blocked.has(key(x, ly + 2))) loungeSeats.push({ spot: { tx: x, ty: ly + 2 }, facing: "up", seat: false });
  }
  for (let x = rx - 1; x >= 2; x--) {
    if (!blocked.has(key(x, ly + 1))) loungeSeats.push({ spot: { tx: x, ty: ly + 1 }, facing: "right", seat: false });
  }
  return {
    cols,
    rows,
    blocked,
    desks,
    loungeSeats,
    armchairs,
    plants,
    rugAt: { tx: rx, ty: ly, w: rugW, h: 3 },
    sofaAt,
    tableAt,
    coolerAt,
  };
}

/** Shortest path on the tile grid (4 directions) around furniture; the goal may be a furniture tile (a sofa seat). */
export function path(l: Layout, from: Spot, to: Spot): Spot[] {
  const start = key(from.tx, from.ty);
  const goal = key(to.tx, to.ty);
  const prev = new Map<string, string>([[start, ""]]);
  const q: Spot[] = [from];
  while (q.length) {
    const c = q.shift()!;
    const k = key(c.tx, c.ty);
    if (k === goal) break;
    for (const [dx, dy] of [
      [1, 0],
      [-1, 0],
      [0, 1],
      [0, -1],
    ]) {
      const n = { tx: c.tx + dx, ty: c.ty + dy };
      const nk = key(n.tx, n.ty);
      if (n.tx < 0 || n.ty < 2 || n.tx >= l.cols || n.ty >= l.rows || prev.has(nk)) continue;
      if (l.blocked.has(nk) && nk !== goal) continue;
      prev.set(nk, k);
      q.push(n);
    }
  }
  if (!prev.has(goal)) return [];
  const out: Spot[] = [];
  for (let k = goal; k && k !== start; k = prev.get(k)!) {
    const [x, y] = k.split(",").map(Number);
    out.unshift({ tx: x, ty: y });
  }
  return out;
}

type Plan = "desk" | "sofa";

export interface Walker {
  x: number; // feet centre, virtual px
  y: number;
  route: Spot[];
  /** Exact place to end on after the route (the middle of a desk). */
  goal: { x: number; y: number } | null;
  plan: Plan;
  facing: Facing;
  sitting: "desk" | "sofa" | null;
  /** Facing once seated in the lounge. */
  seatFacing: Facing;
  /** The lounge place it is going to (changes when others come and go). */
  seat: string;
  layoutKey: string;
}

export const tileCentre = (s: Spot) => ({ x: s.tx * T + T / 2, y: s.ty * T + T - 2 });

/** Where an agent sits at its desk: the middle of the desk. */
export const deskSeat = (d: { desk: Spot; chair: Spot; wide: boolean }) => ({ x: d.desk.tx * T + (d.wide ? 1.5 * T : T), y: tileCentre(d.chair).y });
