// Listeners for the events the backend emits; outside Tauri they never fire.
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { inTauri } from "./api";
import type { CommandApproval, EventRecord } from "./types";

/** Committed domain events (`agenium://event`), live run output (`agenium://run`), command approvals. */
export async function onEvent(f: (e: EventRecord) => void): Promise<UnlistenFn> {
  if (!inTauri) return () => {};
  return listen<EventRecord>("agenium://event", (e) => f(e.payload));
}

export async function onRun(f: (run: string, ev: any, agent: string | null) => void): Promise<UnlistenFn> {
  if (!inTauri) return () => {};
  return listen<{ run: string; agent?: string | null; event: any }>("agenium://run", (e) =>
    f(e.payload.run, e.payload.event, e.payload.agent ?? null),
  );
}

/** The preview browser's events, per tab: its page changed, an element was picked, the page left picking (Esc). */
export async function onBrowser(f: {
  url: (tab: string, u: string) => void;
  picked: (tab: string, json: string) => void;
  pickOff: (tab: string) => void;
}): Promise<UnlistenFn> {
  if (!inTauri) return () => {};
  const offs = await Promise.all([
    listen<{ tab: string; url: string }>("agenium://browser-url", (e) => f.url(e.payload.tab, e.payload.url)),
    listen<{ tab: string; pick: string }>("agenium://browser-picked", (e) => f.picked(e.payload.tab, e.payload.pick)),
    listen<{ tab: string }>("agenium://browser-pick-off", (e) => f.pickOff(e.payload.tab)),
  ]);
  return () => offs.forEach((o) => o());
}

/** The embedding model's preparation moved on (`agenium://model`). */
export async function onModel(f: () => void): Promise<UnlistenFn> {
  if (!inTauri) return () => {};
  return listen("agenium://model", () => f());
}

/** A step of a markitdown install, or a newer version found (`agenium://markitdown`, ADR-152). */
export async function onMarkitdown(f: (e: { step?: string; update?: string }) => void): Promise<UnlistenFn> {
  if (!inTauri) return () => {};
  return listen<{ step?: string; update?: string }>("agenium://markitdown", (e) => f(e.payload));
}

/** A piece of a drafting helper's answer as its model writes it (`agenium://helper`, ADR-151). */
export async function onHelper(f: (p: import("../components/helperLive").HelperPiece) => void): Promise<UnlistenFn> {
  if (!inTauri) return () => {};
  return listen<import("../components/helperLive").HelperPiece>("agenium://helper", (e) => f(e.payload));
}

/** The vendors' usage was read again (`agenium://usage`). */
export async function onUsage(f: () => void): Promise<UnlistenFn> {
  if (!inTauri) return () => {};
  return listen("agenium://usage", () => f());
}

export async function onApproval(f: (a: CommandApproval) => void): Promise<UnlistenFn> {
  if (!inTauri) return () => {};
  return listen<CommandApproval>("agenium://approval", (e) => f(e.payload));
}
