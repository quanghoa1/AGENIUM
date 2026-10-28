import type { Activity } from "../PixelOffice";
import { Facing, R, T } from "./sprites";

/** Floor and wall tones per theme: slate in the dark theme; on the light one a pale oak floor and a soft blue-grey
 *  wall, so the room is not a dark block on a white page. The furniture keeps its colours in both. */
export const ROOM = {
  dark: { plank: ["#2e3548", "#2b3244"], top: "#323a4e", bottom: "#272d3c", joint: "#262c3a", wall: "#2b3654", wallTop: "#161c2c", wallBase: "#1f2840", rail: "#323e60" },
  light: { plank: ["#e6d6bd", "#e1d0b5"], top: "#ecdec8", bottom: "#d6c3a6", joint: "#cdb894", wall: "#c9d6e6", wallTop: "#9fb1c8", wallBase: "#aebfd3", rail: "#dbe4ef" },
};

type RoomTones = (typeof ROOM)["dark"];

export function floor(g: CanvasRenderingContext2D, x0: number, y0: number, w: number, h: number, c: RoomTones) {
  // Long planks, 4 px high, joints staggered by row, so the warm desks and shelves stand out. Low-contrast seams keep
  // it a floor, not a brick wall.
  for (let y = y0; y < y0 + h; y += 4) {
    const row = (y - y0) / 4;
    R(g, x0, y, w, 4, c.plank[row % 2]);
    R(g, x0, y, w, 1, c.top);
    R(g, x0, y + 3, w, 1, c.bottom);
    for (let x = x0 + ((row * 23) % 56); x < x0 + w; x += 56) R(g, x, y + 1, 1, 2, c.joint);
  }
}

export function wall(g: CanvasRenderingContext2D, x0: number, w: number, c: RoomTones) {
  R(g, x0, 0, w, 2 * T, c.wall);
  R(g, x0, 0, w, 3, c.wallTop);
  R(g, x0, 2 * T - 3, w, 3, c.wallBase);
  R(g, x0, T + 2, w, 1, c.rail);
}

export function bookshelf(g: CanvasRenderingContext2D, x: number, y: number) {
  R(g, x, y, 2 * T, T + 4, "#6e4024");
  R(g, x + 1, y + 1, 2 * T - 2, T + 2, "#4a2a17");
  const books = ["#c0392b", "#2e86c1", "#27ae60", "#f1c40f", "#8e44ad", "#e67e22", "#ecf0f1", "#16a085"];
  for (const sy of [y + 2, y + 11]) {
    let bx = x + 2;
    let i = Math.floor(x / 7) + sy;
    while (bx < x + 2 * T - 3) {
      const w = 2 + (i % 2);
      const h = 6 + (i % 3 === 0 ? 1 : 0);
      R(g, bx, sy + (7 - h), w, h, books[i % books.length]);
      bx += w + (i % 4 === 0 ? 1 : 0);
      i++;
    }
    R(g, x + 1, sy + 7, 2 * T - 2, 1, "#6e4024");
  }
}

export function clock(g: CanvasRenderingContext2D, x: number, y: number, t: number) {
  R(g, x + 2, y, 8, 10, "#d64545");
  R(g, x, y + 2, 12, 6, "#d64545");
  R(g, x + 3, y + 1, 6, 8, "#f5f5f0");
  R(g, x + 1, y + 3, 10, 4, "#f5f5f0");
  R(g, x + 5, y + 2, 1, 3, "#222");
  const m = Math.floor(t / 1000) % 4;
  const hand = [
    [6, 5, 3, 1],
    [5, 6, 1, 3],
    [3, 5, 3, 1],
    [5, 4, 1, 2],
  ][m];
  R(g, x + hand[0], y + hand[1], hand[2], hand[3], "#222");
}

export function picture(g: CanvasRenderingContext2D, x: number, y: number, seed: number) {
  R(g, x, y, 14, 11, "#8a5a2b");
  R(g, x + 1, y + 1, 12, 9, "#e8d9b0");
  const c = ["#3a7bd5", "#e2725b", "#4caf50", "#9b59b6"][seed % 4];
  R(g, x + 2, y + 6, 10, 3, c);
  R(g, x + 4, y + 3, 4, 3, "#f4d03f");
}

export function plant(g: CanvasRenderingContext2D, x: number, y: number, t: number, still: boolean) {
  const sway = still ? 0 : Math.floor(t / 900) % 2;
  R(g, x + 4, y + 10, 8, 6, "#b5651d");
  R(g, x + 3, y + 10, 10, 2, "#8a4b14");
  const leaf = ["#2e8b57", "#3cb371", "#246b45"];
  for (const [dx, dy, h, c] of [
    [7, 0, 10, 0],
    [5, 2, 8, 1],
    [9, 1, 9, 2],
    [3, 4, 6, 0],
    [11, 4, 6, 1],
  ] as const) {
    R(g, x + dx + (dy < 3 ? sway : 0), y + dy, 2, h, leaf[c]);
  }
}

export function hangingPlant(g: CanvasRenderingContext2D, x: number, y: number) {
  R(g, x + 3, y, 6, 4, "#7a4a2c");
  for (const [dx, h] of [
    [2, 10],
    [5, 14],
    [8, 9],
    [10, 6],
  ] as const)
    R(g, x + dx, y + 3, 2, h, dx % 2 ? "#3cb371" : "#2e8b57");
}

const SCREEN: Record<Activity, string> = {
  idle: "#1d2433",
  off: "#0d1018",
  thinking: "#6d4fd6",
  tool: "#2b6fd6",
  text: "#2b6fd6",
  shell: "#12351f",
  waiting: "#c98a10",
};

export function desk(g: CanvasRenderingContext2D, x: number, y: number, act: Activity, t: number, still: boolean, wide: boolean) {
  const w = wide ? 3 * T : 2 * T;
  R(g, x, y + 2, w, 11, "#c58f58");
  R(g, x, y + 2, w, 2, "#dcae78");
  R(g, x, y + 13, w, 3, "#9b6a3c");
  R(g, x + 1, y + 16, 2, 3, "#5e3f24");
  R(g, x + w - 3, y + 16, 2, 3, "#5e3f24");
  // CRT computer, screen to the viewer
  const cx = x + Math.floor(w / 2) - 7;
  R(g, cx, y - 6, 14, 12, "#cfc9b8");
  R(g, cx, y - 6, 14, 1, "#e6e1d2");
  R(g, cx + 1, y - 4, 12, 8, "#55524a");
  R(g, cx + 2, y - 3, 10, 6, SCREEN[act]);
  if (act === "shell" || act === "tool" || act === "text") {
    const k = still ? 0 : Math.floor(t / 160) % 5;
    const ink = act === "shell" ? "#5dff8f" : "#cfe4ff";
    for (let i = 0; i < 3; i++) R(g, cx + 3, y - 2 + i * 2, ((i * 2 + k) % 5) + 2, 1, ink);
  } else if (act === "thinking") {
    const k = still ? 0 : Math.floor(t / 300) % 3;
    for (let i = 0; i < 3; i++) R(g, cx + 4 + i * 2, y, 1, 1, i <= k ? "#e6dcff" : "#8f78e6");
  } else if (act === "waiting") {
    R(g, cx + 6, y - 2, 2, 3, "#2a1d00");
    R(g, cx + 6, y + 2, 2, 1, "#2a1d00");
  }
  R(g, cx + 5, y + 6, 4, 2, "#a9a393");
  // keyboard, mug, papers
  R(g, cx + 1, y + 9, 12, 2, "#ecebe6");
  R(g, x + w - 7, y + 5, 4, 4, "#f2f2f2");
  R(g, x + w - 3, y + 6, 1, 2, "#f2f2f2");
  R(g, x + 3, y + 6, 5, 4, "#f7f3e8");
}

export function chair(g: CanvasRenderingContext2D, x: number, y: number, c: string) {
  R(g, x + 3, y + 4, 10, 7, c);
  R(g, x + 3, y + 4, 10, 2, "#00000033");
  R(g, x + 5, y + 11, 2, 3, "#3a3a44");
  R(g, x + 9, y + 11, 2, 3, "#3a3a44");
}

export function rug(g: CanvasRenderingContext2D, x: number, y: number, w: number, h: number) {
  R(g, x, y, w, h, "#3d6aa6");
  R(g, x + 2, y + 2, w - 4, h - 4, "#4f7fbf");
  for (let i = x + 4; i < x + w - 4; i += 6) R(g, i, y + 4, 2, h - 8, "#5a8bc9");
}

export function sofa(g: CanvasRenderingContext2D, x: number, y: number) {
  R(g, x, y + 2, 2 * T, 12, "#a5354a");
  R(g, x, y + 2, 2 * T, 4, "#c04760");
  R(g, x + 2, y + 6, 13, 7, "#b93e56");
  R(g, x + 17, y + 6, 13, 7, "#b93e56");
  R(g, x, y + 14, 2 * T, 2, "#7a2536");
}

export function armchair(g: CanvasRenderingContext2D, x: number, y: number, facing: Facing) {
  const back = facing === "right" ? x + 2 : x + 11;
  R(g, x + 3, y + 4, 10, 10, "#a5354a");
  R(g, back, y + 2, 3, 13, "#7a2536");
  R(g, x + 4, y + 6, 8, 6, "#c04760");
  R(g, x + 3, y + 14, 10, 2, "#7a2536");
}

export function coffeeTable(g: CanvasRenderingContext2D, x: number, y: number) {
  R(g, x + 1, y + 2, 2 * T - 2, 10, "#b9844f");
  R(g, x + 1, y + 2, 2 * T - 2, 2, "#d6a26a");
  R(g, x + 1, y + 12, 2 * T - 2, 2, "#8a5c33");
  R(g, x + 12, y + 4, 4, 4, "#f2f2f2");
}

export function cooler(g: CanvasRenderingContext2D, x: number, y: number) {
  R(g, x + 4, y - 4, 8, 8, "#7fc8f0");
  R(g, x + 5, y - 3, 6, 6, "#a8dcf7");
  R(g, x + 3, y + 4, 10, 12, "#e6e6ea");
  R(g, x + 6, y + 7, 4, 2, "#3c78d8");
}

export function whiteboard(g: CanvasRenderingContext2D, x: number, y: number, lines: string[]) {
  R(g, x, y, 46, 22, "#9aa3b2");
  R(g, x + 1, y + 1, 44, 20, "#f8fafc");
  R(g, x + 3, y + 22, 2, 6, "#6b7280");
  R(g, x + 41, y + 22, 2, 6, "#6b7280");
  g.font = "4px ui-monospace, monospace";
  g.textBaseline = "top";
  lines.slice(0, 3).forEach((l, i) => {
    g.fillStyle = i === 0 ? "#1d4ed8" : "#334155";
    g.fillText(l.slice(0, 16), x + 3, y + 2 + i * 6);
  });
}
