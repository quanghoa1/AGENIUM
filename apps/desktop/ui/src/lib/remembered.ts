// The model the user picked last in a place (ADR-152), kept in the root: a form or a helper chat opens with it. One that
// can no longer run here (its provider removed, its CLI gone) is not used: the form opens as before, with a note.
import { api, type ModelArg, type PrefKey } from "./api";
import type { Capabilities } from "./types";

/** Whether `m` can be used with what this computer has now. */
export function usable(m: ModelArg | null | undefined, caps: Capabilities | null): boolean {
  if (!m?.provider || !caps) return false;
  if (m.backend === "cli") return caps.clis.some((c) => c.kind.toLowerCase() === m.provider && c.compatible);
  return caps.providers.some((p) => p.id === m.provider);
}

function asModel(v: unknown): ModelArg | null {
  const m = v as ModelArg | null;
  return m && typeof m.provider === "string" && typeof m.model === "string" ? { ...m, effort: m.effort || "high" } : null;
}

/** The kept model under `key`, when it can still run here; `gone` names one that cannot. */
export function rememberedModel(v: unknown, caps: Capabilities | null): { model: ModelArg | null; gone: string | null } {
  const m = asModel(v);
  if (!m) return { model: null, gone: null };
  return usable(m, caps) ? { model: m, gone: null } : { model: null, gone: `${m.provider}/${m.model}` };
}

/** Keeps `value` under `key` (best effort: a failure only means the next form opens empty). */
export function remember(key: PrefKey, value: unknown) {
  api.setPreference(key, value).catch(() => undefined);
}
