import { KeyRound, Laptop, Plus, Search, Trash2 } from "lucide-react";
import { useEffect, useState } from "react";
import { Mono, cn } from "../../components/ui";
import { api } from "../../lib/api";
import { friendly } from "../../lib/errors";
import type { Key } from "../../lib/i18n";
import { useApp } from "../../lib/store";
import type { LocalServer, ModelCapsRow } from "../../lib/types";

/** `note`: shown under the presets while this one is picked (a translated key), `hint`: the vendor list's own note. */
export type Preset = { id: string; label: string; kind: "anthropic" | "gemini" | "openai_compat"; base_url?: string; key: boolean; local?: boolean; note?: Key; hint?: string };

export const PRESETS: Preset[] = [
  { id: "anthropic", label: "Anthropic", kind: "anthropic", key: true },
  { id: "gemini", label: "Google Gemini", kind: "gemini", key: true },
  { id: "openai", label: "OpenAI", kind: "openai_compat", base_url: "https://api.openai.com/v1", key: true },
  { id: "openrouter", label: "OpenRouter", kind: "openai_compat", base_url: "https://openrouter.ai/api/v1", key: true },
  { id: "deepseek", label: "DeepSeek", kind: "openai_compat", base_url: "https://api.deepseek.com/v1", key: true },
  // One account key works on both: the endpoint decides the bill (a Go subscription, or Zen's credit).
  { id: "opencode-go", label: "OpenCode Go", kind: "openai_compat", base_url: "https://opencode.ai/zen/go/v1", key: true, note: "providers.note.opencodeGo" },
  { id: "opencode-zen", label: "OpenCode Zen", kind: "openai_compat", base_url: "https://opencode.ai/zen/v1", key: true, note: "providers.note.opencodeZen" },
  { id: "custom", label: "OpenAI-compatible", kind: "openai_compat", base_url: "", key: true },
  { id: "local", label: "Local model", kind: "openai_compat", base_url: "http://127.0.0.1:11434/v1", key: false, local: true },
];

export interface ProviderDraft {
  editing: boolean;
  id: string;
  kind: string;
  base_url: string;
  api_key: string;
  clear_key: boolean;
  max_concurrency: number;
  models: string;
  local: boolean;
  has_key: boolean;
  /** Extra headers; `stored` = a secret already in the vault (empty value keeps it). */
  headers: { name: string; value: string; secret: boolean; stored?: boolean }[];
  /** Per-model settings, as typed. */
  caps: CapsDraft[];
  /** Extra request body fields (`model` "" = every model) and where the effort goes ("" = Agenium's own). */
  body: { path: string; value: string; model: string }[];
  effort_field: string;
}

/** Suggested fields (nothing is sent unless the user adds it). */
const FIELD_HINTS = ["temperature", "top_p", "top_k", "seed", "presence_penalty", "frequency_penalty", "reasoning.effort", "reasoning.max_tokens", "thinking.type", "thinking.budget_tokens", "verbosity", "parallel_tool_calls"];

const EFFORT_FIELD_HINTS = ["reasoning_effort", "reasoning.effort", "thinking.effort", "effort"];

/** Agenium's own fields (`agenium_llm::custom::RESERVED_FIELDS`): the server refuses them, the form says so first. */
const RESERVED = ["model", "messages", "input", "instructions", "prompt", "tools", "tool_choice", "functions", "function_call", "stream", "stream_options", "n", "response_format", "max_tokens", "max_completion_tokens", "max_output_tokens", "prompt_cache_key"];

/** Why a field path would be refused, if it would be. */
function fieldProblem(path: string, t: (k: Key) => string): string | null {
  const p = path.trim();
  if (!p) return null;
  const segs = p.split(".");
  if (segs.length > 4 || segs.some((s) => !/^[A-Za-z0-9_-]{1,64}$/.test(s))) return t("providers.fieldBadName");
  if (RESERVED.includes(segs[0].toLowerCase())) return t("providers.fieldReserved");
  return null;
}

/** One model's settings in the form: "" = as the endpoint says. */
interface CapsDraft {
  model: string;
  tools: "" | "yes" | "no";
  vision: "" | "yes" | "no";
  context: string;
  efforts: string;
  default_effort: string;
}

const yesNo = (v?: boolean | null): "" | "yes" | "no" => (v === true ? "yes" : v === false ? "no" : "");

const fromYesNo = (v: "" | "yes" | "no") => (v === "" ? null : v === "yes");

/** The provider's stored settings as form rows. */
export function capsDraft(rows: ModelCapsRow[] | undefined): CapsDraft[] {
  return (rows ?? []).map((c) => ({
    model: c.model,
    tools: yesNo(c.tools),
    vision: yesNo(c.vision),
    context: c.context_window ? String(c.context_window) : "",
    efforts: (c.efforts ?? []).join(", "),
    default_effort: c.default_effort ?? "",
  }));
}

/** Extra headers and per-model settings of a custom endpoint (its gateway's headers, the effort levels it takes). */
function Advanced({ d, set }: { d: ProviderDraft; set: (p: Partial<ProviderDraft>) => void }) {
  const app = useApp();
  const [open, setOpen] = useState(d.headers.length > 0 || d.caps.length > 0 || d.body.length > 0 || !!d.effort_field);
  const hs = d.headers;
  const cs = d.caps;
  const setH = (i: number, p: Partial<ProviderDraft["headers"][number]>) => set({ headers: hs.map((h, j) => (j === i ? { ...h, ...p } : h)) });
  const setC = (i: number, p: Partial<CapsDraft>) => set({ caps: cs.map((c, j) => (j === i ? { ...c, ...p } : c)) });
  const listed = d.models
    .split(/[,\n]/)
    .map((m) => m.trim())
    .filter(Boolean);
  return (
    <div className="rounded-lg border border-line">
      <button className="flex w-full items-center gap-2 px-3 py-2 text-left text-xs font-medium" onClick={() => setOpen(!open)}>
        <span className="text-subtle">{open ? "▾" : "▸"}</span> {app.t("providers.advanced")}
        <span className="text-[11px] font-normal text-subtle">{app.t("providers.advancedHint")}</span>
      </button>
      {open && (
        <div className="space-y-4 border-t border-line p-3">
          <div>
            <div className="label mb-1">{app.t("providers.headers")}</div>
            <div className="mb-2 text-[11px] text-subtle">{app.t("providers.headersNote")}</div>
            <div className="space-y-1.5">
              {hs.map((h, i) => (
                <div key={i} className="flex items-center gap-2">
                  <input className="input w-56 font-mono" placeholder="X-Tenant-Id" value={h.name} onChange={(e) => setH(i, { name: e.target.value })} />
                  <input
                    className="input flex-1 font-mono"
                    type={h.secret ? "password" : "text"}
                    autoComplete="off"
                    placeholder={h.secret && h.stored ? app.t("providers.keyKept") : app.t("providers.headerValue")}
                    value={h.value}
                    onChange={(e) => setH(i, { value: e.target.value })}
                  />
                  <label className="flex items-center gap-1 text-[11px] text-muted" title={app.t("providers.headerSecretHint")}>
                    <input type="checkbox" checked={h.secret} onChange={(e) => setH(i, { secret: e.target.checked })} /> {app.t("providers.headerSecret")}
                  </label>
                  <button className="btn px-2" title={app.t("common.delete")} onClick={() => set({ headers: hs.filter((_, j) => j !== i) })}>
                    <Trash2 className="h-3.5 w-3.5" />
                  </button>
                </div>
              ))}
              <button className="btn" onClick={() => set({ headers: [...hs, { name: "", value: "", secret: true }] })}>
                <Plus className="h-3.5 w-3.5" /> {app.t("providers.addHeader")}
              </button>
            </div>
          </div>
          <div>
            <div className="label mb-1">{app.t("providers.perModel")}</div>
            <div className="mb-2 text-[11px] text-subtle">{app.t("providers.perModelNote")}</div>
            {cs.length > 0 && (
              <table className="w-full text-xs">
                <thead>
                  <tr className="label text-left">
                    <th className="py-1">model</th>
                    <th>{app.t("providers.capTools")}</th>
                    <th>{app.t("providers.capVision")}</th>
                    <th>{app.t("providers.capContext")}</th>
                    <th>{app.t("providers.capEfforts")}</th>
                    <th>{app.t("providers.capDefault")}</th>
                    <th />
                  </tr>
                </thead>
                <tbody>
                  {cs.map((c, i) => {
                    const levels = c.efforts
                      .split(",")
                      .map((x) => x.trim())
                      .filter(Boolean);
                    const tri = (v: "" | "yes" | "no", on: (v: "" | "yes" | "no") => void) => (
                      <select className="input py-1" value={v} onChange={(e) => on(e.target.value as "" | "yes" | "no")}>
                        <option value="">{app.t("providers.capAuto")}</option>
                        <option value="yes">{app.t("providers.capYes")}</option>
                        <option value="no">{app.t("providers.capNo")}</option>
                      </select>
                    );
                    return (
                      <tr key={i}>
                        <td className="py-1 pr-2">
                          <input className="input py-1 font-mono" list="caps-models" value={c.model} onChange={(e) => setC(i, { model: e.target.value })} />
                        </td>
                        <td className="pr-2">{tri(c.tools, (v) => setC(i, { tools: v }))}</td>
                        <td className="pr-2">{tri(c.vision, (v) => setC(i, { vision: v }))}</td>
                        <td className="pr-2">
                          <input className="input w-28 py-1 font-mono" type="number" min={1} placeholder="128000" value={c.context} onChange={(e) => setC(i, { context: e.target.value })} />
                        </td>
                        <td className="pr-2">
                          <input className="input py-1 font-mono" placeholder="low, medium, high" value={c.efforts} onChange={(e) => setC(i, { efforts: e.target.value })} />
                        </td>
                        <td className="pr-2">
                          <select className="input py-1" value={c.default_effort} disabled={levels.length === 0} onChange={(e) => setC(i, { default_effort: e.target.value })}>
                            <option value="">–</option>
                            {levels.map((l) => (
                              <option key={l} value={l}>
                                {l}
                              </option>
                            ))}
                          </select>
                        </td>
                        <td>
                          <button className="btn px-2" title={app.t("common.delete")} onClick={() => set({ caps: cs.filter((_, j) => j !== i) })}>
                            <Trash2 className="h-3.5 w-3.5" />
                          </button>
                        </td>
                      </tr>
                    );
                  })}
                </tbody>
              </table>
            )}
            <datalist id="caps-models">
              {listed.map((m) => (
                <option key={m} value={m} />
              ))}
            </datalist>
            <button
              className="btn mt-1.5"
              onClick={() => set({ caps: [...cs, { model: listed.find((m) => !cs.some((c) => c.model === m)) ?? "", tools: "", vision: "", context: "", efforts: "", default_effort: "" }] })}
            >
              <Plus className="h-3.5 w-3.5" /> {app.t("providers.addModelSettings")}
            </button>
          </div>
          <div>
            <div className="label mb-1">{app.t("providers.requestFields")}</div>
            <div className="mb-2 text-[11px] text-subtle">{app.t("providers.requestFieldsNote")}</div>
            <label className="mb-2 flex items-center gap-2 text-xs">
              <span className="w-40 text-muted">{app.t("providers.effortField")}</span>
              <input
                className="input w-64 font-mono"
                list="effort-field-hints"
                placeholder={`reasoning_effort (${app.t("model.default")})`}
                value={d.effort_field}
                onChange={(e) => set({ effort_field: e.target.value })}
              />
              {fieldProblem(d.effort_field, app.t) && <span className="text-[11px] text-rose-600 dark:text-rose-400">{fieldProblem(d.effort_field, app.t)}</span>}
            </label>
            <datalist id="effort-field-hints">
              {EFFORT_FIELD_HINTS.map((h) => (
                <option key={h} value={h} />
              ))}
            </datalist>
            <div className="space-y-1.5">
              {d.body.map((f, i) => {
                const problem = fieldProblem(f.path, app.t);
                const setF = (p: Partial<ProviderDraft["body"][number]>) => set({ body: d.body.map((x, j) => (j === i ? { ...x, ...p } : x)) });
                return (
                  <div key={i} className="flex flex-wrap items-center gap-2">
                    <input className={cn("input w-56 font-mono", problem && "border-rose-500")} list="field-hints" placeholder="temperature" value={f.path} onChange={(e) => setF({ path: e.target.value })} />
                    <input className="input w-56 font-mono" placeholder={app.t("providers.fieldValue")} value={f.value} onChange={(e) => setF({ value: e.target.value })} />
                    <select className="input w-48" value={f.model} onChange={(e) => setF({ model: e.target.value })}>
                      <option value="">{app.t("providers.fieldAllModels")}</option>
                      {[...new Set([...listed, ...cs.map((c) => c.model).filter(Boolean), ...(f.model ? [f.model] : [])])].map((m) => (
                        <option key={m} value={m}>
                          {m}
                        </option>
                      ))}
                    </select>
                    <button className="btn px-2" title={app.t("common.delete")} onClick={() => set({ body: d.body.filter((_, j) => j !== i) })}>
                      <Trash2 className="h-3.5 w-3.5" />
                    </button>
                    {problem && <span className="text-[11px] text-rose-600 dark:text-rose-400">{problem}</span>}
                  </div>
                );
              })}
              <datalist id="field-hints">
                {FIELD_HINTS.map((h) => (
                  <option key={h} value={h} />
                ))}
              </datalist>
              <button className="btn" onClick={() => set({ body: [...d.body, { path: "", value: "", model: "" }] })}>
                <Plus className="h-3.5 w-3.5" /> {app.t("providers.addField")}
              </button>
            </div>
          </div>
        </div>
      )}
    </div>
  );
}

export function ProviderForm({ draft, onDone }: { draft: ProviderDraft; onDone: () => void }) {
  const app = useApp();
  const [d, setD] = useState<ProviderDraft>(draft);
  const [found, setFound] = useState<LocalServer[] | null>(null);
  const [looking, setLooking] = useState(false);
  const set = (p: Partial<ProviderDraft>) => setD((x) => ({ ...x, ...p }));
  const scan = () => {
    setLooking(true);
    api
      .localServers()
      .then(setFound)
      .catch((e) => app.notify(String(e)))
      .finally(() => setLooking(false));
  };
  useEffect(() => {
    if (d.local && !d.editing) scan();
  }, []);
  const save = () =>
    api
      .putProvider({
        id: d.id.trim(),
        kind: d.kind,
        base_url: d.base_url.trim() || null,
        api_key: d.api_key.trim() || null,
        clear_key: d.clear_key,
        max_concurrency: d.max_concurrency,
        models: d.models
          .split(/[,\n]/)
          .map((m) => m.trim())
          .filter(Boolean),
        headers: d.headers.filter((h) => h.name.trim()).map((h) => ({ name: h.name.trim(), value: h.value.trim() || null, secret: h.secret })),
        model_caps: d.caps
          .filter((c) => c.model.trim())
          .map((c) => {
            const efforts = c.efforts
              .split(",")
              .map((x) => x.trim().toLowerCase())
              .filter(Boolean);
            return {
              model: c.model.trim(),
              tools: fromYesNo(c.tools),
              vision: fromYesNo(c.vision),
              context_window: Number(c.context) > 0 ? Number(c.context) : null,
              efforts: efforts.length ? efforts : null,
              default_effort: efforts.includes(c.default_effort) ? c.default_effort : null,
            };
          }),
        body: d.body.filter((f) => f.path.trim()).map((f) => ({ path: f.path.trim(), value: f.value, model: f.model || null })),
        effort_field: d.effort_field.trim(),
      })
      .then(() => (app.notify(app.t("common.done")), onDone()))
      .catch((e) => app.notify(friendly(e, app.t)));
  // What Save still needs: a name, an address for an OpenAI-compatible endpoint, a key for a cloud provider (kept
  // when editing one that has it).
  const missing: Key[] = [
    ...(!d.id.trim() ? (["providers.needId"] as Key[]) : []),
    ...(d.kind === "openai_compat" && !d.base_url.trim() ? (["providers.needUrl"] as Key[]) : []),
    ...(!d.local && !d.api_key.trim() && !(d.editing && d.has_key && !d.clear_key) ? (["providers.needKey"] as Key[]) : []),
    ...([d.effort_field, ...d.body.map((f) => f.path)].some((p) => fieldProblem(p, app.t)) ? (["providers.needFields"] as Key[]) : []),
  ];
  return (
    <div className="max-w-4xl space-y-3 rounded-lg border border-line bg-elevated/40 p-4">
      {d.local && (
        <div className="space-y-2">
          <div className="flex items-center gap-2">
            <button className="btn" onClick={scan} disabled={looking}>
              <Search className="h-3.5 w-3.5" /> {looking ? app.t("providers.looking") : app.t("providers.scan")}
            </button>
            <span className="text-[11px] text-subtle">{app.t("providers.scanNote")}</span>
          </div>
          {found && found.length === 0 && <div className="text-[11px] text-amber-600 dark:text-amber-400">{app.t("providers.noneFound")}</div>}
          {found?.map((s) => (
            <div key={s.base_url} className="flex flex-wrap items-center gap-2 rounded-lg border border-emerald-500/30 bg-emerald-500/5 p-2.5 text-xs">
              <Laptop className="h-4 w-4 text-emerald-500" />
              <span className="font-semibold">{s.kind === "ollama" ? "Ollama" : "LM Studio"}</span>
              <Mono className="text-subtle">{s.base_url}</Mono>
              <span className="text-subtle">· {s.models.length} model</span>
              <button className="btn ml-auto" onClick={() => set({ id: d.editing ? d.id : s.kind, base_url: s.base_url, models: s.models.join(", ") })}>
                {app.t("providers.use")}
              </button>
            </div>
          ))}
        </div>
      )}
      <div className="grid gap-3 md:grid-cols-2">
        <label>
          <span className="label">{app.t("providers.id")}</span>
          <input className="input mt-1" value={d.id} disabled={d.editing} placeholder="my-provider" onChange={(e) => set({ id: e.target.value.toLowerCase() })} />
        </label>
        <label>
          <span className="label">base URL</span>
          <input className="input mt-1 font-mono" value={d.base_url} placeholder={d.kind === "openai_compat" ? "https://…/v1" : app.t("providers.defaultUrl")} onChange={(e) => set({ base_url: e.target.value })} />
          {/<[^>]+>/.test(d.base_url) && <div className="mt-1 text-[11px] text-amber-600 dark:text-amber-400">{app.t("providers.placeholder")}</div>}
        </label>
        {!d.local && (
          <label className="md:col-span-2">
            <span className="label flex items-center gap-1">
              <KeyRound className="h-3 w-3" /> API key
            </span>
            <input
              className="input mt-1 font-mono"
              type="password"
              autoComplete="off"
              value={d.api_key}
              placeholder={d.has_key ? app.t("providers.keyKept") : "sk-…"}
              onChange={(e) => set({ api_key: e.target.value, clear_key: false })}
            />
            <span className="mt-1 block text-[11px] text-subtle">{app.t("providers.keyNote")}</span>
          </label>
        )}
        <label>
          <span className="label">{app.t("providers.concurrency")}</span>
          <input className="input mt-1" type="number" min={1} max={64} value={d.max_concurrency} onChange={(e) => set({ max_concurrency: Number(e.target.value) || 1 })} />
        </label>
        <label>
          <span className="label">{app.t("providers.models")}</span>
          <input className="input mt-1 font-mono" value={d.models} placeholder={app.t("providers.modelsHint")} onChange={(e) => set({ models: e.target.value })} />
        </label>
      </div>
      {d.kind === "openai_compat" && <Advanced d={d} set={set} />}
      {d.has_key && d.editing && (
        <label className="flex items-center gap-2 text-xs text-muted">
          <input type="checkbox" checked={d.clear_key} onChange={(e) => set({ clear_key: e.target.checked, api_key: "" })} /> {app.t("providers.clearKey")}
        </label>
      )}
      <div className="flex flex-wrap items-center gap-2">
        <button className="btn-primary" disabled={missing.length > 0} onClick={save}>
          {app.t("common.save")}
        </button>
        <button className="btn" onClick={onDone}>
          {app.t("common.cancel")}
        </button>
        {missing.length > 0 && <span className="text-[11px] text-amber-600 dark:text-amber-400">{missing.map((k) => app.t(k)).join(" · ")}</span>}
      </div>
    </div>
  );
}
