import { Search } from "lucide-react";
import { useEffect, useState } from "react";
import { api } from "../../lib/api";
import { useApp } from "../../lib/store";
import type { ToolSearchSettings } from "../../lib/types";

const MODES = ["auto", "always", "never"] as const;
const JUDGE = ["unsure", "always", "off"] as const;

/** `[tool_search]` (ADR-139) on the MCP page it acts on: when an agent finds its MCP tools instead of getting them all
 *  listed, how many it keeps listed, and when the judge reads a search's candidates. */
export function ToolSearchCard() {
  const app = useApp();
  const [form, setForm] = useState<ToolSearchSettings | null>(null);
  const [saved, setSaved] = useState<ToolSearchSettings | null>(null);
  useEffect(() => {
    api
      .toolSearch()
      .then((s) => (setForm(s), setSaved(s)))
      .catch(() => undefined);
  }, []);
  if (!form || !saved) return null;
  const set = (p: Partial<ToolSearchSettings>) => setForm({ ...form, ...p });
  const num = (v: string, min: number) => Math.max(min, Math.floor(Number(v) || 0));
  const changed = JSON.stringify(form) !== JSON.stringify(saved);
  const save = () =>
    api
      .setToolSearch(form)
      .then((s) => (setForm(s), setSaved(s), app.notify(app.t("library.toolSearch.saved"))))
      .catch((e) => app.notify(String(e)));
  const deferring = form.mode === "auto";
  return (
    <div className="card space-y-3 p-4">
      <div className="flex items-center gap-2 text-xs font-semibold">
        <Search className="h-3.5 w-3.5" /> {app.t("library.toolSearch")}
      </div>
      <div className="text-[11px] text-subtle">{app.t("library.toolSearch.intro")}</div>
      <div className="grid gap-2 md:grid-cols-2">
        <label className="block text-xs">
          <span className="label">{app.t("library.toolSearch.mode")}</span>
          <select className="input mt-1" value={form.mode} onChange={(e) => set({ mode: e.target.value as ToolSearchSettings["mode"] })}>
            {MODES.map((m) => (
              <option key={m} value={m}>
                {app.t(`library.toolSearch.mode.${m}`)}
              </option>
            ))}
          </select>
        </label>
        <label className="block text-xs">
          <span className="label">{app.t("library.toolSearch.keepUsed")}</span>
          <input className="input mt-1" type="number" min={0} disabled={form.mode === "never"} value={form.keep_used} onChange={(e) => set({ keep_used: num(e.target.value, 0) })} />
        </label>
        <label className="block text-xs">
          <span className="label">{app.t("library.toolSearch.overTools")}</span>
          <input className="input mt-1" type="number" min={1} disabled={!deferring} value={form.over_tools} onChange={(e) => set({ over_tools: num(e.target.value, 1) })} />
        </label>
        <label className="block text-xs">
          <span className="label">{app.t("library.toolSearch.overTokens")}</span>
          <input className="input mt-1" type="number" min={1} step={1000} disabled={!deferring} value={form.over_tokens} onChange={(e) => set({ over_tokens: num(e.target.value, 1) })} />
        </label>
        <label className="block text-xs md:col-span-2">
          <span className="label">{app.t("library.toolSearch.judge")}</span>
          <select className="input mt-1" value={form.judge} onChange={(e) => set({ judge: e.target.value as ToolSearchSettings["judge"] })}>
            {JUDGE.map((j) => (
              <option key={j} value={j}>
                {app.t(`library.toolSearch.judge.${j}`)}
              </option>
            ))}
          </select>
          <span className="mt-1 block text-[11px] text-subtle">{app.t("library.toolSearch.judgeNote")}</span>
        </label>
      </div>
      <div className="flex gap-2">
        <button className="btn-primary" disabled={!changed} onClick={save}>
          {app.t("common.save")}
        </button>
        {changed && (
          <button className="btn" onClick={() => setForm(saved)}>
            {app.t("common.cancel")}
          </button>
        )}
      </div>
    </div>
  );
}
