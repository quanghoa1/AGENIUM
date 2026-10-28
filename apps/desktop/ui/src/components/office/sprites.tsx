

export const T = 16; // tile, virtual pixels

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

export type Facing = "down" | "up" | "left" | "right";

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

export function look(id: string): Look {
  const h = hash(id);
  // Unsigned shifts: a signed one can go negative and index past the palette.
  return { h: HAIR[h % HAIR.length], s: SKIN[(h >>> 4) % SKIN.length], c: SHIRT[(h >>> 8) % SHIRT.length], p: PANTS[(h >>> 12) % PANTS.length], style: (h >>> 16) % 3 };
}

export const OUTLINE = "#1b1726";

/** Draws a 12x18 character map at (x, y) = top-left; mirrored for "left". `rows` limits the rows drawn (seated). */
export function sprite(g: CanvasRenderingContext2D, map: string[], x: number, y: number, l: Look, mirror: boolean, rows = 18) {
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
export function pose(facing: Facing, l: Look, frame: number, walking: boolean, typing: boolean): string[] {
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

export function R(g: CanvasRenderingContext2D, x: number, y: number, w: number, h: number, c: string) {
  g.fillStyle = c;
  g.fillRect(x, y, w, h);
}
