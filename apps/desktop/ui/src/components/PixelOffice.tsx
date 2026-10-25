// The pixel office of the Live tab: one room per team (Solo agents and the pool share one), seen from above like a
// small RPG map. An agent with a task (an open ticket, a run, or for a Lead a team with open tickets) sits at its
// desk, back turned, the CRT showing what it does (green text for a shell command, blue for other tools and
// writing, violet while thinking, amber while waiting); an agent without a task walks to the lounge and sits on
// the sofa or an armchair; offline agents sleep at the desk. Every character carries its name. Bubbles: "..."
// thinking, "?" waiting, a wrench/prompt while calling a tool, Zz. Dashed lines from the Lead's desk to a worker's
// desk are delegations (labelled with the ticket); messages between them fly across as envelopes.
// The look follows pixel-agents (https://github.com/pixel-agents-hq/pixel-agents); the art and the code are our own:
// every sprite below is a hand-made character map, every piece of furniture is drawn from rectangles.
import { useEffect, useRef, useState } from "react";
import type { LiveAgent } from "../lib/types";
import { vendorOf } from "../lib/vendors";

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

const SIGN: Record<CallKind, [string, string, string]> = {
  mcp: ["#6d28d9", "#ede9fe", "MCP"],
  skill: ["#0f766e", "#ccfbf1", "Skill"],
  tool: ["#334155", "#e2e8f0", "Tool"],
  agenium: ["#1d4ed8", "#dbeafe", "Agenium"],
};

const T = 16; // tile, virtual pixels

// ---------------------------------------------------------------- sprites (12 x 18, our own)
// o outline, h hair, s skin, e eye, c shirt, p trousers, b shoes, . nothing

const FRONT = [
  "...oooooo...",
  "..ohhhhhho..",
  ".ohhhhhhhho.",
  ".ohhhhhhhho.",
  ".ohssssssho.",
  ".osssssssso.",
  ".osesssseso.",
  ".osesssseso.",
  ".osssssssso.",
  "..osssssso..",
  "..occcccco..",
  ".occcccccco.",
  "osccccccccso",
  "osccccccccso",
  ".occcccccco.",
  "..oppppppo..",
  "..opp..ppo..",
  "..obb..bbo..",
];

const BACK = [
  "...oooooo...",
  "..ohhhhhho..",
  ".ohhhhhhhho.",
  ".ohhhhhhhho.",
  ".ohhhhhhhho.",
  ".ohhhhhhhho.",
  ".ohhhhhhhho.",
  ".ohhhhhhhho.",
  ".ohhhhhhhho.",
  "..ohhhhhho..",
  "..occcccco..",
  ".occcccccco.",
  "osccccccccso",
  "osccccccccso",
  ".occcccccco.",
  "..oppppppo..",
  "..opp..ppo..",
  "..obb..bbo..",
];

const SIDE = [
  "...oooooo...",
  "..ohhhhhho..",
  ".ohhhhhhhho.",
  ".ohhhhhhhho.",
  ".ohhhssssso.",
  ".ohhsssssso.",
  ".ohhsssseso.",
  ".ohhsssseso.",
  ".ohssssssso.",
  "..osssssso..",
  "..occcccco..",
  "..occcccco..",
  "..occcscco..",
  "..occcscco..",
  "..occcccco..",
  "..oppppppo..",
  "...oppppo...",
  "...obbbbo...",
];

// Leg frames while walking (rows 15-17).
const WALK_FRONT = [
  ["..oppppppo..", "..opp..ppo..", "..obb...bo.."],
  ["..oppppppo..", "..opp..ppo..", "..ob...bbo.."],
];
const WALK_SIDE = [
  ["..oppppppo..", "..opp..ppo..", "..obb..bbo.."],
  ["...oppppo...", "...oppppo...", "...obbbbo..."],
];

type Facing = "down" | "up" | "left" | "right";

const HAIR = ["#3b2416", "#6b3f22", "#c9893b", "#e9c46a", "#1d1d2b", "#8c2f2f", "#d9d9d9", "#4b3b78"];
const SKIN = ["#f4cfae", "#e2ad84", "#b9805a", "#8a5a3b", "#f8dcc6"];
const SHIRT = ["#2f6fd6", "#2fa36b", "#e07b2e", "#d0424a", "#8152d6", "#1fa5b8", "#d64f9a", "#f2f2f2", "#2c2c3a"];
const PANTS = ["#26304a", "#3a2e28", "#2b3d2f", "#40404a"];

function hash(s: string): number {
  let h = 2166136261;
  for (const c of s) h = Math.imul(h ^ c.charCodeAt(0), 16777619);
  return h >>> 0;
}

interface Look {
  h: string;
  s: string;
  c: string;
  p: string;
  style: number;
}

function look(id: string): Look {
  const h = hash(id);
  // Unsigned shifts: a signed one can go negative and index past the palette.
  return { h: HAIR[h % HAIR.length], s: SKIN[(h >>> 4) % SKIN.length], c: SHIRT[(h >>> 8) % SHIRT.length], p: PANTS[(h >>> 12) % PANTS.length], style: (h >>> 16) % 3 };
}

const OUTLINE = "#1b1726";

/** Draws a 12x18 character map at (x, y) = top-left; mirrored for "left". `rows` limits the rows drawn (seated). */
function sprite(g: CanvasRenderingContext2D, map: string[], x: number, y: number, l: Look, mirror: boolean, rows = 18) {
  const col: Record<string, string> = { o: OUTLINE, h: l.h, s: l.s, e: "#1b1726", c: l.c, p: l.p, b: "#2a2222" };
  for (let r = 0; r < Math.min(rows, map.length); r++) {
    const line = map[r];
    for (let i = 0; i < 12; i++) {
      const ch = line[mirror ? 11 - i : i];
      if (ch === ".") continue;
      g.fillStyle = col[ch];
      g.fillRect(x + i, y + r, 1, 1);
    }
  }
}

/** The map of one pose, with the hair style and the walking/typing frame applied. */
function pose(facing: Facing, l: Look, frame: number, walking: boolean, typing: boolean): string[] {
  const base = facing === "up" ? BACK : facing === "down" ? FRONT : SIDE;
  const m = base.map((r) => r.split(""));
  // Hair styles: 0 short, 1 long (strands down the sides / the back), 2 spiky.
  if (l.style === 1) {
    for (let y = 4; y <= 11; y++) {
      if (facing === "up") {
        if (y >= 10) for (let x = 3; x <= 8; x++) m[y][x] = "h";
      } else if (facing === "down") {
        if (y <= 10) {
          m[y][1] = "h";
          m[y][10] = "h";
          m[y][0] = "o";
          m[y][11] = "o";
        }
      } else if (y <= 10) {
        m[y][2] = "h";
        m[y][1] = "o";
      }
    }
  } else if (l.style === 2) {
    m[0] = ".o.o.oo.o.o.".split("");
    m[1] = ".ohohhhhoho.".split("");
  }
  if (walking) {
    const legs = (facing === "left" || facing === "right" ? WALK_SIDE : WALK_FRONT)[frame % 2];
    for (let i = 0; i < 3; i++) m[15 + i] = legs[i].split("");
  }
  if (typing) {
    // Hands forward, alternating.
    const k = frame % 2;
    m[12][0] = k ? "." : "o";
    m[12][11] = k ? "o" : ".";
    m[11][0] = k ? "s" : ".";
    m[11][11] = k ? "." : "s";
  }
  return m.map((r) => r.join(""));
}

// ---------------------------------------------------------------- furniture (rectangles)

function R(g: CanvasRenderingContext2D, x: number, y: number, w: number, h: number, c: string) {
  g.fillStyle = c;
  g.fillRect(x, y, w, h);
}

function floor(g: CanvasRenderingContext2D, x0: number, y0: number, w: number, h: number) {
  // Wooden planks, 4 px high, seams staggered by row.
  for (let y = y0; y < y0 + h; y += 4) {
    const row = (y - y0) / 4;
    R(g, x0, y, w, 4, row % 2 ? "#8f5734" : "#9a5f39");
    R(g, x0, y + 3, w, 1, "#7a4a2c");
    for (let x = x0 + ((row * 11) % 24); x < x0 + w; x += 24) R(g, x, y, 1, 3, "#7a4a2c");
  }
}

function wall(g: CanvasRenderingContext2D, x0: number, w: number) {
  R(g, x0, 0, w, 2 * T, "#2b3654");
  R(g, x0, 0, w, 3, "#161c2c");
  R(g, x0, 2 * T - 3, w, 3, "#1f2840");
  R(g, x0, T + 2, w, 1, "#323e60");
}

function bookshelf(g: CanvasRenderingContext2D, x: number, y: number) {
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

function clock(g: CanvasRenderingContext2D, x: number, y: number, t: number) {
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

function picture(g: CanvasRenderingContext2D, x: number, y: number, seed: number) {
  R(g, x, y, 14, 11, "#8a5a2b");
  R(g, x + 1, y + 1, 12, 9, "#e8d9b0");
  const c = ["#3a7bd5", "#e2725b", "#4caf50", "#9b59b6"][seed % 4];
  R(g, x + 2, y + 6, 10, 3, c);
  R(g, x + 4, y + 3, 4, 3, "#f4d03f");
}

function plant(g: CanvasRenderingContext2D, x: number, y: number, t: number, still: boolean) {
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

function hangingPlant(g: CanvasRenderingContext2D, x: number, y: number) {
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

function desk(g: CanvasRenderingContext2D, x: number, y: number, act: Activity, t: number, still: boolean, wide: boolean) {
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

function chair(g: CanvasRenderingContext2D, x: number, y: number, c: string) {
  R(g, x + 3, y + 4, 10, 7, c);
  R(g, x + 3, y + 4, 10, 2, "#00000033");
  R(g, x + 5, y + 11, 2, 3, "#3a3a44");
  R(g, x + 9, y + 11, 2, 3, "#3a3a44");
}

function rug(g: CanvasRenderingContext2D, x: number, y: number, w: number, h: number) {
  R(g, x, y, w, h, "#3d6aa6");
  R(g, x + 2, y + 2, w - 4, h - 4, "#4f7fbf");
  for (let i = x + 4; i < x + w - 4; i += 6) R(g, i, y + 4, 2, h - 8, "#5a8bc9");
}

function sofa(g: CanvasRenderingContext2D, x: number, y: number) {
  R(g, x, y + 2, 2 * T, 12, "#a5354a");
  R(g, x, y + 2, 2 * T, 4, "#c04760");
  R(g, x + 2, y + 6, 13, 7, "#b93e56");
  R(g, x + 17, y + 6, 13, 7, "#b93e56");
  R(g, x, y + 14, 2 * T, 2, "#7a2536");
}

function armchair(g: CanvasRenderingContext2D, x: number, y: number, facing: Facing) {
  const back = facing === "right" ? x + 2 : x + 11;
  R(g, x + 3, y + 4, 10, 10, "#a5354a");
  R(g, back, y + 2, 3, 13, "#7a2536");
  R(g, x + 4, y + 6, 8, 6, "#c04760");
  R(g, x + 3, y + 14, 10, 2, "#7a2536");
}

function coffeeTable(g: CanvasRenderingContext2D, x: number, y: number) {
  R(g, x + 1, y + 2, 2 * T - 2, 10, "#b9844f");
  R(g, x + 1, y + 2, 2 * T - 2, 2, "#d6a26a");
  R(g, x + 1, y + 12, 2 * T - 2, 2, "#8a5c33");
  R(g, x + 12, y + 4, 4, 4, "#f2f2f2");
}

function cooler(g: CanvasRenderingContext2D, x: number, y: number) {
  R(g, x + 4, y - 4, 8, 8, "#7fc8f0");
  R(g, x + 5, y - 3, 6, 6, "#a8dcf7");
  R(g, x + 3, y + 4, 10, 12, "#e6e6ea");
  R(g, x + 6, y + 7, 4, 2, "#3c78d8");
}

function whiteboard(g: CanvasRenderingContext2D, x: number, y: number, lines: string[]) {
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

/** A label over a desk: what the agent calls now (MCP / Skill / Tool). */
function sign(g: CanvasRenderingContext2D, cx: number, y: number, kind: CallKind, text: string) {
  const [bg, fg, tag] = SIGN[kind];
  g.font = "bold 4px ui-sans-serif, system-ui";
  const tw = Math.ceil(g.measureText(tag).width);
  g.font = "4px ui-monospace, monospace";
  const label = text.length > 22 ? text.slice(0, 21) + "…" : text;
  const lw = Math.ceil(g.measureText(label).width);
  const w = tw + lw + 7;
  const x = Math.round(cx - w / 2);
  R(g, x, y, w, 7, OUTLINE);
  R(g, x + 1, y + 1, tw + 3, 5, bg);
  R(g, x + tw + 4, y + 1, lw + 2, 5, fg);
  g.textBaseline = "top";
  g.font = "bold 4px ui-sans-serif, system-ui";
  g.fillStyle = "#ffffff";
  g.fillText(tag, x + 2, y + 1.5);
  g.font = "4px ui-monospace, monospace";
  g.fillStyle = "#111827";
  g.fillText(label, x + tw + 5, y + 1.5);
}

/** Width of a `label` pill. */
function labelWidth(g: CanvasRenderingContext2D, text: string) {
  g.font = "bold 4px ui-sans-serif, system-ui";
  return Math.ceil(g.measureText(labelText(text)).width) + 4;
}

// Long names keep both ends: "very-long…archer-1".
const labelText = (text: string) => (text.length > 18 ? `${text.slice(0, 8)}…${text.slice(-9)}` : text);

/** A tiny vendor chip (brand colour + initial) drawn just left of a name, so the office shows which model runs an
 * agent (ADR-89). Returns its width so the name can sit beside it. */
function vendorChip(g: CanvasRenderingContext2D, rightX: number, cy: number, provider: string, model: string): number {
  const v = vendorOf(provider, model);
  const s = 7;
  const x = rightX - s;
  const y = cy - Math.ceil(s / 2);
  g.fillStyle = v.color;
  g.fillRect(x, y, s, s);
  g.fillStyle = "#ffffff";
  g.font = "5px ui-monospace, monospace";
  g.textAlign = "center";
  g.textBaseline = "middle";
  g.fillText(v.initial[0], x + s / 2, y + s / 2 + 0.5);
  return s + 1;
}

/** A small dark pill with text, centred on cx (names over heads, tickets on delegation lines). */
function label(g: CanvasRenderingContext2D, cx: number, y: number, text: string, bg: string, fg: string) {
  const t = labelText(text);
  const w = labelWidth(g, text);
  const x = Math.round(cx - w / 2);
  R(g, x, y, w, 6, bg);
  g.fillStyle = fg;
  g.textBaseline = "top";
  g.fillText(t, x + 2, y + 1);
}

function crown(g: CanvasRenderingContext2D, x: number, y: number) {
  R(g, x, y + 2, 7, 3, "#f5c518");
  R(g, x, y, 1, 2, "#f5c518");
  R(g, x + 3, y - 1, 1, 3, "#f5c518");
  R(g, x + 6, y, 1, 2, "#f5c518");
  R(g, x + 3, y + 3, 1, 1, "#d63031");
  R(g, x, y + 5, 7, 1, "#b8860b");
}

/** Pixel speech bubble above a head with a small icon. */
function bubble(g: CanvasRenderingContext2D, x: number, y: number, kind: "dots" | "q" | "tool" | "shell" | "z" | "pen", frame: number) {
  const w = 13;
  const h = 9;
  const bg = kind === "q" ? "#ffd166" : "#ffffff";
  R(g, x + 1, y, w - 2, h, OUTLINE);
  R(g, x, y + 1, w, h - 2, OUTLINE);
  R(g, x + 1, y + 1, w - 2, h - 2, bg);
  R(g, x + 3, y + h, 3, 1, OUTLINE);
  R(g, x + 4, y + h - 1, 2, 1, bg);
  R(g, x + 4, y + h + 1, 1, 1, OUTLINE);
  const ink = "#1b1726";
  if (kind === "dots") for (let i = 0; i < 3; i++) R(g, x + 3 + i * 3, y + 4, 2, 2, i <= frame % 3 ? ink : "#c7c3d6");
  else if (kind === "q") {
    R(g, x + 5, y + 2, 3, 1, ink);
    R(g, x + 7, y + 3, 1, 2, ink);
    R(g, x + 6, y + 4, 1, 1, ink);
    R(g, x + 6, y + 6, 1, 1, ink);
  } else if (kind === "tool") {
    R(g, x + 4, y + 2, 3, 3, "#6c6c78");
    R(g, x + 5, y + 3, 1, 1, "#ffffff");
    R(g, x + 6, y + 5, 3, 2, "#6c6c78");
  } else if (kind === "shell") {
    R(g, x + 3, y + 3, 1, 1, "#16a34a");
    R(g, x + 4, y + 4, 1, 1, "#16a34a");
    R(g, x + 3, y + 5, 1, 1, "#16a34a");
    if (frame % 2) R(g, x + 6, y + 5, 3, 1, "#16a34a");
  } else if (kind === "pen") {
    for (let i = 0; i < 4; i++) R(g, x + 4 + i, y + 5 - i, 1, 1, "#2b6fd6");
    R(g, x + 3, y + 6, 1, 1, ink);
  } else {
    R(g, x + 4, y + 2, 4, 1, "#6c6c78");
    R(g, x + 6, y + 3, 1, 1, "#6c6c78");
    R(g, x + 5, y + 4, 1, 1, "#6c6c78");
    R(g, x + 4, y + 5, 4, 1, "#6c6c78");
  }
}

function envelope(g: CanvasRenderingContext2D, x: number, y: number) {
  R(g, x - 5, y - 4, 10, 8, OUTLINE);
  R(g, x - 4, y - 3, 8, 6, "#fbfbf6");
  R(g, x - 3, y - 2, 1, 1, "#9aa0ad");
  R(g, x - 2, y - 1, 1, 1, "#9aa0ad");
  R(g, x - 1, y, 2, 1, "#9aa0ad");
  R(g, x + 1, y - 1, 1, 1, "#9aa0ad");
  R(g, x + 2, y - 2, 1, 1, "#9aa0ad");
}

// ---------------------------------------------------------------- layout

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

const key = (x: number, y: number) => `${x},${y}`;

function buildLayout(cols: number, ids: string[], lead: string | null | undefined, deskless: Set<string>): Layout {
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
function path(l: Layout, from: Spot, to: Spot): Spot[] {
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

// ---------------------------------------------------------------- walkers

type Plan = "desk" | "sofa";

interface Walker {
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

const tileCentre = (s: Spot) => ({ x: s.tx * T + T / 2, y: s.ty * T + T - 2 });
/** Where an agent sits at its desk: the middle of the desk. */
const deskSeat = (d: { desk: Spot; chair: Spot; wide: boolean }) => ({ x: d.desk.tx * T + (d.wide ? 1.5 * T : T), y: tileCentre(d.chair).y });

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
      floor(g, 0, 2 * T, W, H - 2 * T);
      wall(g, 0, W);
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
      g.strokeStyle = "#11151f";
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
        let ly = top - 7;
        while (placed.some((p) => lx < p.x + p.w + 1 && p.x < lx + lw + 1 && Math.abs(p.y - ly) < 7)) ly -= 7;
        placed.push({ x: lx, y: ly, w: lw });
        label(g, Math.round(w.x), ly, oa.agent.agent.name, id === selected ? "#2563eb" : "#111827e6", "#ffffff");
        // The vendor behind this agent's model, as a small chip left of the name.
        vendorChip(g, lx - 1, ly, oa.agent.agent.model.provider, oa.agent.agent.model.model);
      }
      // envelopes between a Lead and a worker
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
          <div className="truncate font-mono text-[10px] text-muted">{hovered.ticker}</div>
        </div>
      )}
    </div>
  );
}
