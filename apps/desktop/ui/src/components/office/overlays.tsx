import { vendorOf } from "../../lib/vendors";
import type { CallKind } from "../PixelOffice";
import { OUTLINE, R } from "./sprites";

const SIGN: Record<CallKind, [string, string, string]> = {
  mcp: ["#6d28d9", "#ede9fe", "MCP"],
  skill: ["#0f766e", "#ccfbf1", "Skill"],
  tool: ["#334155", "#e2e8f0", "Tool"],
  agenium: ["#1d4ed8", "#dbeafe", "Agenium"],
};

/** A label over a desk: what the agent calls now (MCP / Skill / Tool). */
export function sign(g: CanvasRenderingContext2D, cx: number, y: number, kind: CallKind, text: string) {
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
export function labelWidth(g: CanvasRenderingContext2D, text: string) {
  g.font = "bold 4px ui-sans-serif, system-ui";
  return Math.ceil(g.measureText(labelText(text)).width) + 4;
}

// Long names keep both ends: "very-long…archer-1".
const labelText = (text: string) => (text.length > 18 ? `${text.slice(0, 8)}…${text.slice(-9)}` : text);

/** A tiny vendor chip (brand colour + initial) drawn just left of a name, so the office shows which model runs an
 * agent (ADR-89). Returns its width so the name can sit beside it. */
export function vendorChip(g: CanvasRenderingContext2D, rightX: number, cy: number, provider: string, model: string): number {
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
  // Back to the canvas default for whatever is drawn next (the whiteboard in the next frame).
  g.textAlign = "left";
  return s + 1;
}

/** A small dark pill with text, centred on cx (names over heads, tickets on delegation lines). */
export function label(g: CanvasRenderingContext2D, cx: number, y: number, text: string, bg: string, fg: string) {
  const t = labelText(text);
  const w = labelWidth(g, text);
  const x = Math.round(cx - w / 2);
  R(g, x, y, w, 6, bg);
  g.fillStyle = fg;
  // Left-aligned: a vendor chip drawn before leaves the canvas centred, which pushed names out of their pill.
  g.textAlign = "left";
  g.textBaseline = "top";
  g.fillText(t, x + 2, y + 1);
}

export function crown(g: CanvasRenderingContext2D, x: number, y: number) {
  R(g, x, y + 2, 7, 3, "#f5c518");
  R(g, x, y, 1, 2, "#f5c518");
  R(g, x + 3, y - 1, 1, 3, "#f5c518");
  R(g, x + 6, y, 1, 2, "#f5c518");
  R(g, x + 3, y + 3, 1, 1, "#d63031");
  R(g, x, y + 5, 7, 1, "#b8860b");
}

/** Pixel speech bubble above a head with a small icon. */
export function bubble(g: CanvasRenderingContext2D, x: number, y: number, kind: "dots" | "q" | "tool" | "shell" | "z" | "pen", frame: number) {
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

export function envelope(g: CanvasRenderingContext2D, x: number, y: number) {
  R(g, x - 5, y - 4, 10, 8, OUTLINE);
  R(g, x - 4, y - 3, 8, 6, "#fbfbf6");
  R(g, x - 3, y - 2, 1, 1, "#9aa0ad");
  R(g, x - 2, y - 1, 1, 1, "#9aa0ad");
  R(g, x - 1, y, 2, 1, "#9aa0ad");
  R(g, x + 1, y - 1, 1, 1, "#9aa0ad");
  R(g, x + 2, y - 2, 1, 1, "#9aa0ad");
}
