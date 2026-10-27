// Capabilities the user manages (ADR-61): model providers (API key or local model; agent CLIs are detected), and the
// system's skill library and MCP servers that each agent switches on or off.
import { AlertTriangle, CheckCircle2, Eye, FileUp, KeyRound, Laptop, ListTree, Pencil, Plug, Plus, RefreshCw, Search, Trash2, Wrench, XCircle } from "lucide-react";
import { Fragment, useEffect, useState } from "react";
import { Markdown } from "../components/Markdown";
import { ConfirmModal, Modal, PanelSwap, Segmented } from "../components/motion";
import { Empty, Mono, PageHeader, Section, cn } from "../components/ui";
import { api, inTauri } from "../lib/api";
import { friendly } from "../lib/errors";
import type { Key } from "../lib/i18n";
import { useApp } from "../lib/store";
import type { Capabilities, LibrarySkill, LocalServer, McpServerConfig, McpServerRow, ModelCapsRow, ModelInfo } from "../lib/types";

// ---------------------------------------------------------------- providers

type Preset = { id: string; label: string; kind: "anthropic" | "gemini" | "openai_compat"; base_url?: string; key: boolean; local?: boolean };

const PRESETS: Preset[] = [
  { id: "anthropic", label: "Anthropic", kind: "anthropic", key: true },
  { id: "gemini", label: "Google Gemini", kind: "gemini", key: true },
  { id: "openai", label: "OpenAI", kind: "openai_compat", base_url: "https://api.openai.com/v1", key: true },
  { id: "openrouter", label: "OpenRouter", kind: "openai_compat", base_url: "https://openrouter.ai/api/v1", key: true },
  { id: "deepseek", label: "DeepSeek", kind: "openai_compat", base_url: "https://api.deepseek.com/v1", key: true },
  { id: "opencode-go", label: "OpenCode", kind: "openai_compat", base_url: "https://opencode.ai/zen/go/v1", key: true },
  { id: "custom", label: "OpenAI-compatible", kind: "openai_compat", base_url: "", key: true },
  { id: "local", label: "Local model", kind: "openai_compat", base_url: "http://127.0.0.1:11434/v1", key: false, local: true },
];

interface ProviderDraft {
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

function ProviderForm({ draft, onDone }: { draft: ProviderDraft; onDone: () => void }) {
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

function ModelsList({ id }: { id: string }) {
  const app = useApp();
  const [ms, setMs] = useState<ModelInfo[] | null>(null);
  const [error, setError] = useState<string | null>(null);
  useEffect(() => {
    api.providerModels(id).then(setMs).catch((e) => setError(String(e)));
  }, [id]);
  if (error) return <div className="text-[11px] text-rose-600 dark:text-rose-400">{error}</div>;
  if (!ms) return <div className="text-[11px] text-subtle">{app.t("common.loading")}</div>;
  const flag = (b?: boolean | null) => (b === true ? <CheckCircle2 className="h-3.5 w-3.5 text-emerald-500" /> : b === false ? <XCircle className="h-3.5 w-3.5 text-rose-500" /> : <span className="text-subtle">?</span>);
  return (
    <div className="max-h-64 overflow-y-auto">
      <table className="w-full text-[11px]">
        <thead>
          <tr className="label text-left">
            <th className="py-1">model</th>
            <th>tools</th>
            <th>vision</th>
            <th>context</th>
          </tr>
        </thead>
        <tbody className="divide-y divide-line">
          {ms.map((m) => (
            <tr key={m.id}>
              <td className="py-1 font-mono">{m.id}</td>
              <td>{flag(m.supports_tools)}</td>
              <td>{flag(m.supports_vision)}</td>
              <td className="font-mono text-subtle">{m.context_window ? `${Math.round(m.context_window / 1000)}k` : "–"}</td>
            </tr>
          ))}
        </tbody>
      </table>
      {ms.length === 0 && <Empty>{app.t("common.none")}</Empty>}
    </div>
  );
}

/** The detector's note on a CLI in the interface language (its English sentences are for the logs). */
function cliNote(note: string | null | undefined, t: (k: Key) => string): string {
  if (!note) return "";
  if (note.startsWith("Agenium does not drive")) return t("providers.noteNotDriven");
  if (note.startsWith("cannot read its version")) return t("providers.noteNoVersion");
  const old = note.match(/older than the checked minimum (\S+)/);
  return old ? t("providers.noteOld").replace("{x}", old[1]) : note;
}

/** A CLI's login in words: signed in, not signed in, or not known (the CLI gives no way to ask). */
function LoginState({ login, installed }: { login: unknown; installed: boolean }) {
  const app = useApp();
  if (!installed) return <span className="text-subtle">–</span>;
  const [key, tone] =
    login === "logged_in"
      ? (["providers.loggedIn", "text-emerald-600 dark:text-emerald-300"] as const)
      : login === "not_logged_in"
        ? (["providers.notLoggedIn", "text-rose-600 dark:text-rose-300"] as const)
        : (["providers.loginUnknown", "text-amber-600 dark:text-amber-300"] as const);
  return <span className={tone}>{app.t(key)}</span>;
}

export function ProvidersView() {
  const app = useApp();
  const [c, setC] = useState<Capabilities | null>(null);
  const [draft, setDraft] = useState<ProviderDraft | null>(null);
  const [models, setModels] = useState<string | null>(null);
  const [del, setDel] = useState<string | null>(null);
  const [scanning, setScanning] = useState(false);
  const [vendors, setVendors] = useState<Preset[]>([]);
  const load = (refresh = false) => api.capabilities(refresh).then(setC);
  useEffect(() => {
    load();
  }, [app.tick]);
  useEffect(() => {
    api
      .providerPresets()
      .then((v) => setVendors((v ?? []).map((p) => ({ id: p.id, label: p.name, kind: p.kind, base_url: p.base_url ?? undefined, key: p.key_env !== "", local: p.key_env === "" }))))
      .catch(() => setVendors([]));
  }, []);
  if (!c)
    return (
      <div className="space-y-4">
        <PageHeader title={app.t("nav.providers")} sub={app.t("providers.sub")} />
        <div className="card flex items-center gap-2 p-4 text-xs text-muted">
          <RefreshCw className="h-3.5 w-3.5 animate-spin" /> {app.t("providers.detecting")}
        </div>
      </div>
    );
  const fromPreset = (p: Preset): ProviderDraft => ({
    editing: false,
    id: c.providers.some((x) => x.id === p.id) ? "" : p.id,
    kind: p.kind,
    base_url: p.base_url ?? "",
    api_key: "",
    clear_key: false,
    max_concurrency: p.local ? 1 : 4,
    models: "",
    local: !!p.local,
    has_key: false,
    headers: [],
    caps: [],
    body: [],
    effort_field: "",
  });
  return (
    <div className="space-y-4">
      <PageHeader title={app.t("nav.providers")} sub={app.t("providers.sub")} />
      <Section
        title={app.t("providers.api")}
        right={
          !draft && (
            <button className="btn" onClick={() => setDraft(fromPreset(PRESETS[6]))}>
              <Plus className="h-3.5 w-3.5" /> {app.t("providers.add")}
            </button>
          )
        }
      >
        {draft && (
          <div className="mb-4 space-y-3">
            {!draft.editing && (
              <div className="flex flex-wrap gap-2">
                {vendors.length > 0 && (
                  <select
                    className="input w-auto text-xs"
                    value=""
                    onChange={(e) => {
                      const p = vendors.find((v) => v.id === e.target.value);
                      if (p) setDraft(fromPreset(p));
                    }}
                  >
                    <option value="">{app.t("providers.more")} ({vendors.length})</option>
                    {vendors.map((v) => (
                      <option key={v.id} value={v.id}>
                        {v.label}
                      </option>
                    ))}
                  </select>
                )}
                {PRESETS.map((p) => (
                  <button
                    key={p.id}
                    onClick={() => setDraft(fromPreset(p))}
                    className={cn(
                      "press rounded-lg border px-3 py-2 text-left text-xs",
                      (draft.local ? p.local : !p.local && draft.base_url === (p.base_url ?? "") && draft.kind === p.kind) ? "border-accent bg-blue-600/10" : "border-line hover:border-line-strong",
                    )}
                  >
                    <div className="flex items-center gap-1.5 font-semibold">
                      {p.local ? <Laptop className="h-3.5 w-3.5" /> : <KeyRound className="h-3.5 w-3.5" />}
                      {p.local ? app.t("providers.local") : p.label}
                    </div>
                  </button>
                ))}
              </div>
            )}
            <ProviderForm key={`${draft.id}-${draft.kind}-${draft.base_url}-${draft.local}`} draft={draft} onDone={() => (setDraft(null), load())} />
          </div>
        )}
        <table className="w-full text-xs">
          <thead>
            <tr className="label border-b border-line text-left">
              <th className="py-2">id</th>
              <th>kind</th>
              <th>base URL</th>
              <th>key</th>
              <th>{app.t("providers.concurrency")}</th>
              <th />
            </tr>
          </thead>
          <tbody className="divide-y divide-line">
            {c.providers.map((p) => (
              <Fragment key={p.id}>
                <tr>
                  <td className="py-2 font-medium">{p.id}</td>
                  <td className="text-muted">{p.kind}</td>
                  <td className="font-mono text-[11px] text-subtle">{p.base_url ?? "default"}</td>
                  <td>{p.has_key ? <CheckCircle2 className="h-4 w-4 text-emerald-500" /> : <span className="text-subtle">–</span>}</td>
                  <td className="font-mono">{p.max_concurrency}</td>
                  <td className="py-1.5">
                    <div className="flex justify-end gap-1.5">
                      <button className="btn" onClick={() => setModels(models === p.id ? null : p.id)}>
                        <ListTree className="h-3.5 w-3.5" /> {app.t("providers.models")}
                      </button>
                      <button
                        className="btn"
                        title={app.t("common.edit")}
                        aria-label={app.t("common.edit")}
                        onClick={() =>
                          setDraft({
                            editing: true,
                            id: p.id,
                            kind: p.kind,
                            base_url: p.base_url ?? "",
                            api_key: "",
                            clear_key: false,
                            max_concurrency: p.max_concurrency,
                            models: p.models.join(", "),
                            local: !p.has_key && !!p.base_url && /127\.0\.0\.1|localhost/.test(p.base_url),
                            has_key: p.has_key,
                            headers: (p.headers ?? []).map((h) => ({ name: h.name, value: h.value, secret: h.secret, stored: h.secret })),
                            caps: capsDraft(p.model_caps),
                            body: (p.body ?? []).map((f) => ({ path: f.path, value: f.value, model: f.model ?? "" })),
                            effort_field: p.effort_field ?? "",
                          })
                        }
                      >
                        <Pencil className="h-3.5 w-3.5" />
                      </button>
                      <button className="btn" title={app.t("providers.remove")} aria-label={app.t("providers.remove")} onClick={() => setDel(p.id)}>
                        <Trash2 className="h-3.5 w-3.5" />
                      </button>
                    </div>
                  </td>
                </tr>
                {models === p.id && (
                  <tr key={p.id + "-models"}>
                    <td colSpan={6} className="pb-3">
                      <ModelsList id={p.id} />
                    </td>
                  </tr>
                )}
              </Fragment>
            ))}
          </tbody>
        </table>
        {c.providers.length === 0 && !draft && <Empty>{app.t("providers.empty")}</Empty>}
      </Section>
      <Section
        title={app.t("providers.cli")}
        right={
          <button
            className="btn"
            disabled={scanning}
            onClick={() => {
              setScanning(true);
              load(true).finally(() => setScanning(false));
            }}
          >
            <RefreshCw className={cn("h-3.5 w-3.5", scanning && "animate-spin")} /> {app.t("providers.detect")}
          </button>
        }
      >
        <div className="mb-3 text-[11px] text-subtle">{app.t("providers.cliNote")}</div>
        <table className="w-full text-xs">
          <thead>
            <tr className="label border-b border-line text-left">
              <th className="py-2">CLI</th>
              <th>{app.t("providers.version")}</th>
              <th>{app.t("providers.login")}</th>
              <th>{app.t("providers.usable")}</th>
              <th>{app.t("providers.note")}</th>
            </tr>
          </thead>
          <tbody className="divide-y divide-line">
            {c.clis.map((x) => (
              <tr key={x.kind}>
                <td className="py-2 font-medium">{x.kind.toLowerCase()}</td>
                <td className="font-mono">{x.version ?? "–"}</td>
                <td>
                  <LoginState login={x.login} installed={!!x.path} />
                </td>
                <td>
                  {/* Usable, but a login not confirmed is a warning, not a tick. */}
                  {!x.compatible ? (
                    <XCircle className="h-4 w-4 text-subtle" />
                  ) : x.login === "logged_in" ? (
                    <CheckCircle2 className="h-4 w-4 text-emerald-500" />
                  ) : (
                    <span title={app.t("providers.loginUnsure")}>
                      <AlertTriangle className="h-4 w-4 text-amber-500" />
                    </span>
                  )}
                </td>
                <td className="text-[11px] text-subtle">{x.path ? cliNote(x.note, app.t) : app.t("providers.notInstalled")}</td>
              </tr>
            ))}
          </tbody>
        </table>
        <div className="mt-3 text-[11px] text-subtle">
          {app.t("providers.fallback")}: {c.cli_fallback.provider ? <Mono>{c.cli_fallback.provider}/{c.cli_fallback.model}</Mono> : app.t("providers.notSet")}
        </div>
      </Section>
      <ConfirmModal
        open={!!del}
        title={`${app.t("providers.remove")} ${del ?? ""}`}
        note={app.t("providers.removeNote")}
        confirm={app.t("common.delete")}
        onClose={() => setDel(null)}
        onConfirm={() =>
          api
            .removeProvider(del!)
            .then(() => (setDel(null), load(), app.notify(app.t("common.done"))))
            .catch((e) => (setDel(null), app.notify(String(e))))
        }
      />
    </div>
  );
}

// ---------------------------------------------------------------- skill & MCP library

function SkillsPanel() {
  const app = useApp();
  const [lib, setLib] = useState<{ skills: LibrarySkill[]; problems: { dir: string; error: string }[] } | null>(null);
  const [path, setPath] = useState("");
  const [replace, setReplace] = useState(false);
  const [view, setView] = useState<{ name: string; body: string; files: string[] } | null>(null);
  const [del, setDel] = useState<LibrarySkill | null>(null);
  const [drop, setDrop] = useState(false);
  const load = () => api.skillLibrary().then(setLib);
  useEffect(() => {
    load();
  }, [app.tick]);
  const doImport = (p: string) =>
    api
      .importSkill(p, replace)
      .then((m) => {
        app.notify(`${app.t("library.imported")} ${m.name}`);
        // ADR-88: surface the skill-shape advice (one job, when-to-use, steps, tools, proof) so the user can improve it.
        if (m.shape_notes?.length) {
          app.notify(`${app.t("library.shapeNotes")} ${m.name}: ${m.shape_notes.join("; ")}`);
        }
        // ADR-90: the judge's calibrated read of the skill against the methodology principles (only when it is on).
        if (m.quality?.length) {
          app.notify(`${app.t("library.quality")} ${m.name}: ${m.quality.join("; ")}`);
        }
        setPath("");
        load();
      })
      .catch((e) => app.notify(String(e)));
  // Drop a skill folder or .zip on the window (Tauri gives the paths).
  useEffect(() => {
    if (!inTauri) return;
    // The listener arrives asynchronously: one that lands after cleanup is removed at once (no stacked listeners).
    let un: (() => void) | undefined;
    let gone = false;
    import("@tauri-apps/api/webview").then(({ getCurrentWebview }) =>
      getCurrentWebview()
        .onDragDropEvent((e) => {
          const p = e.payload;
          if (p.type === "enter" || p.type === "over") setDrop(true);
          else if (p.type === "leave") setDrop(false);
          else if (p.type === "drop") {
            setDrop(false);
            if (p.paths[0]) doImport(p.paths[0]);
          }
        })
        .then((f) => (gone ? f() : (un = f))),
    );
    return () => {
      gone = true;
      un?.();
    };
  }, [replace]);
  return (
    <div className="space-y-4">
      <div className={cn("card space-y-3 border-dashed p-4 transition-colors", drop && "border-accent bg-blue-600/10")}>
        <div className="flex items-center gap-2 text-xs font-semibold">
          <FileUp className="h-4 w-4 text-accent" /> {app.t("library.importTitle")}
        </div>
        <div className="text-[11px] text-subtle">{app.t("library.importNote")}</div>
        <div className="flex flex-wrap gap-2">
          <input className="input min-w-[280px] flex-1 font-mono" value={path} placeholder="C:\skills\my-skill  ·  my-skill.zip" onChange={(e) => setPath(e.target.value)} />
          <label className="flex items-center gap-1.5 text-xs text-muted">
            <input type="checkbox" checked={replace} onChange={(e) => setReplace(e.target.checked)} /> {app.t("library.replace")}
          </label>
          <button className="btn-primary" disabled={!path.trim()} onClick={() => doImport(path.trim())}>
            <Plus className="h-3.5 w-3.5" /> {app.t("library.import")}
          </button>
        </div>
      </div>
      {!lib && <div className="text-xs text-subtle">{app.t("common.loading")}</div>}
      <div className="grid gap-3 lg:grid-cols-2 2xl:grid-cols-3">
        {lib?.skills.map((s) => (
          <div key={s.name} className="card flex flex-col p-4">
            <div className="flex items-center gap-2">
              <Wrench className="h-4 w-4 text-cyan-500" />
              <span className="text-sm font-semibold">{s.name}</span>
              {s.version && <Mono className="text-subtle">v{s.version}</Mono>}
              <Mono className="ml-auto text-subtle">
                {s.files} {app.t(s.files === 1 ? "library.file" : "library.files")}
              </Mono>
            </div>
            <div className="mt-1.5 line-clamp-3 flex-1 text-xs text-muted">{s.description}</div>
            <div className="mt-3 flex flex-wrap items-center gap-1">
              <span className="label mr-1">{app.t("library.enabledBy")}</span>
              {s.users.length ? (
                s.users.map((u) => (
                  <span key={u} className="chip border-line-strong bg-elevated text-muted">
                    {u}
                  </span>
                ))
              ) : (
                <span className="text-[11px] text-subtle">–</span>
              )}
            </div>
            <div className="mt-3 flex gap-2">
              <button className="btn" onClick={() => api.skillText(s.name).then(setView)}>
                <Eye className="h-3.5 w-3.5" /> {app.t("library.read")}
              </button>
              <button className="btn" onClick={() => setDel(s)}>
                <Trash2 className="h-3.5 w-3.5" /> {app.t("common.delete")}
              </button>
            </div>
          </div>
        ))}
      </div>
      {lib && lib.skills.length === 0 && <Empty>{app.t("library.noSkills")}</Empty>}
      {lib && lib.problems.length > 0 && (
        <Section title={app.t("library.invalid")}>
          {lib.problems.map((p) => (
            <div key={p.dir} className="text-[11px]">
              <Mono className="text-rose-600 dark:text-rose-400">{p.dir}</Mono> — {p.error}
            </div>
          ))}
        </Section>
      )}
      <Modal open={!!view} title={view?.name ?? ""} onClose={() => setView(null)}>
        <div className="max-h-[60vh] overflow-y-auto text-xs">{view && <Markdown text={view.body} />}</div>
        {view && view.files.length > 0 && <div className="mt-3 font-mono text-[11px] text-subtle">{view.files.join(" · ")}</div>}
      </Modal>
      <ConfirmModal
        open={!!del}
        title={`${app.t("common.delete")} ${del?.name ?? ""}`}
        note={del?.users.length ? `${app.t("library.inUse")} ${del.users.join(", ")}.` : app.t("library.deleteNote")}
        confirm={del?.users.length ? app.t("library.deleteForce") : app.t("common.delete")}
        onClose={() => setDel(null)}
        onConfirm={() =>
          api
            .removeSkill(del!.name, del!.users.length > 0)
            .then(() => (setDel(null), load()))
            .catch((e) => (setDel(null), app.notify(String(e))))
        }
      />
    </div>
  );
}

const lines = (s: string) =>
  s
    .split("\n")
    .map((x) => x.trim())
    .filter(Boolean);

function pairs(s: string, sep: string): Record<string, string> {
  const out: Record<string, string> = {};
  for (const l of lines(s)) {
    const i = l.indexOf(sep);
    if (i > 0) out[l.slice(0, i).trim()] = l.slice(i + 1).trim();
  }
  return out;
}

function McpPanel() {
  const app = useApp();
  const [rows, setRows] = useState<McpServerRow[] | null>(null);
  const [mode, setMode] = useState<"stdio" | "http" | "json">("stdio");
  const [form, setForm] = useState({ id: "", command: "", args: "", env: "", url: "", headers: "", timeout: 120, json: "" });
  const [tested, setTested] = useState<Record<string, { tools: { name: string; description?: string | null; read_only: boolean }[]; error?: string } | "busy">>({});
  const [del, setDel] = useState<McpServerRow | null>(null);
  const [signing, setSigning] = useState<string | null>(null);
  const load = () => api.mcpServers().then(setRows);
  const signIn = (id: string) => {
    setSigning(id);
    app.notify(app.t("library.signingIn"));
    api
      .loginMcp(id)
      .then(() => (app.notify(`${id}: ${app.t("library.signedIn")}`), setTested((t) => ({ ...t, [id]: undefined as never })), load()))
      .catch((e) => app.notify(String(e)))
      .finally(() => setSigning(null));
  };
  const signOut = (id: string) =>
    api
      .logoutMcp(id)
      .then(load)
      .catch((e) => app.notify(String(e)));
  useEffect(() => {
    load();
  }, [app.tick]);
  const set = (p: Partial<typeof form>) => setForm((f) => ({ ...f, ...p }));
  const save = () => {
    const done = () => (app.notify(app.t("common.done")), setForm({ id: "", command: "", args: "", env: "", url: "", headers: "", timeout: 120, json: "" }), load());
    if (mode === "json") {
      api
        .importMcpJson(form.json)
        .then((r) => (app.notify(r.servers.join(", ")), done()))
        .catch((e) => app.notify(String(e)));
      return;
    }
    const server: McpServerConfig =
      mode === "stdio"
        ? { id: form.id.trim(), command: form.command.trim(), args: lines(form.args), env: pairs(form.env, "="), timeout_secs: form.timeout }
        : { id: form.id.trim(), url: form.url.trim(), headers: pairs(form.headers, ":"), timeout_secs: form.timeout };
    api
      .putMcp(server)
      .then(done)
      .catch((e) => app.notify(String(e)));
  };
  const test = (id: string) => {
    setTested((t) => ({ ...t, [id]: "busy" }));
    api
      .testMcp(id)
      .then((r) => setTested((t) => ({ ...t, [id]: { tools: r.tools } })))
      .catch((e) => setTested((t) => ({ ...t, [id]: { tools: [], error: String(e) } })));
  };
  return (
    <div className="space-y-4">
      <div className="card space-y-3 p-4">
        <div className="flex flex-wrap items-center gap-3">
          <div className="flex items-center gap-2 text-xs font-semibold">
            <Plug className="h-4 w-4 text-accent" /> {app.t("library.addMcp")}
          </div>
          <Segmented
            id="mcp-mode"
            value={mode}
            onChange={setMode}
            items={[
              { value: "stdio", label: app.t("library.stdio") },
              { value: "http", label: "URL (HTTP)" },
              { value: "json", label: app.t("library.pasteJson") },
            ]}
          />
        </div>
        <PanelSwap id={mode}>
          {mode === "json" ? (
            <textarea
              className="input min-h-[140px] font-mono text-[11px]"
              value={form.json}
              placeholder={'{"mcpServers": {"github": {"command": "npx", "args": ["-y", "@modelcontextprotocol/server-github"], "env": {"GITHUB_PERSONAL_ACCESS_TOKEN": "…"}}}}'}
              onChange={(e) => set({ json: e.target.value })}
            />
          ) : (
            <div className="grid gap-3 md:grid-cols-2">
              <label>
                <span className="label">id</span>
                <input className="input mt-1" value={form.id} placeholder="github" onChange={(e) => set({ id: e.target.value.toLowerCase() })} />
              </label>
              {mode === "stdio" ? (
                <label>
                  <span className="label">{app.t("library.command")}</span>
                  <input className="input mt-1 font-mono" value={form.command} placeholder="npx" onChange={(e) => set({ command: e.target.value })} />
                </label>
              ) : (
                <label>
                  <span className="label">URL</span>
                  <input className="input mt-1 font-mono" value={form.url} placeholder="https://mcp.example.com/mcp" onChange={(e) => set({ url: e.target.value })} />
                </label>
              )}
              {mode === "stdio" ? (
                <>
                  <label>
                    <span className="label">{app.t("library.args")}</span>
                    <textarea className="input mt-1 min-h-[70px] font-mono text-[11px]" value={form.args} placeholder={"-y\n@modelcontextprotocol/server-github"} onChange={(e) => set({ args: e.target.value })} />
                  </label>
                  <label>
                    <span className="label">env (KEY=value)</span>
                    <textarea className="input mt-1 min-h-[70px] font-mono text-[11px]" value={form.env} placeholder={"GITHUB_PERSONAL_ACCESS_TOKEN=ghp_…"} onChange={(e) => set({ env: e.target.value })} />
                  </label>
                </>
              ) : (
                <label className="md:col-span-2">
                  <span className="label">headers (Name: value)</span>
                  <textarea className="input mt-1 min-h-[70px] font-mono text-[11px]" value={form.headers} placeholder={"Authorization: Bearer sk-…"} onChange={(e) => set({ headers: e.target.value })} />
                </label>
              )}
            </div>
          )}
        </PanelSwap>
        <div className="text-[11px] text-subtle">{app.t("library.secretNote")}</div>
        <button className="btn-primary" disabled={mode === "json" ? !form.json.trim() : !form.id.trim() || (mode === "stdio" ? !form.command.trim() : !form.url.trim())} onClick={save}>
          <Plus className="h-3.5 w-3.5" /> {app.t("common.save")}
        </button>
      </div>
      {!rows && <div className="text-xs text-subtle">{app.t("common.loading")}</div>}
      <div className="space-y-2">
        {rows?.map((r) => {
          const t = tested[r.config.id];
          return (
            <div key={r.config.id} className="card p-4">
              <div className="flex flex-wrap items-center gap-2">
                <Plug className="h-4 w-4 text-purple-500" />
                <span className="text-sm font-semibold">{r.config.id}</span>
                <Mono className="truncate text-subtle">{r.config.url ?? [r.config.command, ...(r.config.args ?? [])].join(" ")}</Mono>
                {r.signed_in && <span className="chip border-emerald-500/30 text-emerald-600">{app.t("library.signedIn")}</span>}
                <div className="ml-auto flex gap-1.5">
                  {r.config.url &&
                    (r.signed_in ? (
                      <button className="btn" onClick={() => signOut(r.config.id)}>
                        {app.t("library.signOut")}
                      </button>
                    ) : (
                      <button className="btn" disabled={signing === r.config.id} onClick={() => signIn(r.config.id)}>
                        <RefreshCw className={cn("h-3.5 w-3.5", signing === r.config.id && "animate-spin")} /> {app.t("library.signIn")}
                      </button>
                    ))}
                  <button className="btn" disabled={t === "busy"} onClick={() => test(r.config.id)}>
                    <RefreshCw className={cn("h-3.5 w-3.5", t === "busy" && "animate-spin")} /> {app.t("library.test")}
                  </button>
                  <button className="btn" onClick={() => setDel(r)}>
                    <Trash2 className="h-3.5 w-3.5" />
                  </button>
                </div>
              </div>
              {Object.keys({ ...r.config.env, ...r.config.headers }).length > 0 && (
                <div className="mt-2 space-y-0.5">
                  <span className="label">{app.t("agents.systemKey")}</span>
                  {Object.entries({ ...r.config.env, ...r.config.headers }).map(([k, v]) => (
                    <div key={k} className="truncate font-mono text-[11px]">
                      <span className="text-subtle">{k}</span> = {v}
                    </div>
                  ))}
                </div>
              )}
              <div className="mt-2 flex flex-wrap items-center gap-1">
                <span className="label mr-1">{app.t("library.enabledBy")}</span>
                {r.users.length ? (
                  r.users.map((u) => (
                    <span key={u} className="chip border-line-strong bg-elevated text-muted">
                      {u}
                    </span>
                  ))
                ) : (
                  <span className="text-[11px] text-subtle">–</span>
                )}
              </div>
              {t && t !== "busy" && (
                <div className="mt-3 rounded-lg border border-line bg-elevated/40 p-3">
                  {t.error ? (
                    <div className="text-[11px] text-rose-600 dark:text-rose-400">{t.error}</div>
                  ) : (
                    <div className="space-y-1">
                      <div className="text-[11px] text-emerald-600 dark:text-emerald-400">
                        {app.t("library.connected")} · {t.tools.length} tool{app.lang === "en" && t.tools.length !== 1 ? "s" : ""}
                      </div>
                      {t.tools.map((x) => (
                        <div key={x.name} className="text-[11px]">
                          <Mono className="font-semibold">{x.name}</Mono>
                          {x.read_only && <span className="chip ml-1 border-emerald-500/30 text-emerald-600">read-only</span>}
                          <span className="ml-2 text-subtle">{x.description}</span>
                        </div>
                      ))}
                    </div>
                  )}
                </div>
              )}
            </div>
          );
        })}
      </div>
      {rows && rows.length === 0 && <Empty>{app.t("library.noMcp")}</Empty>}
      <ConfirmModal
        open={!!del}
        title={`${app.t("common.delete")} ${del?.config.id ?? ""}`}
        note={del?.users.length ? `${app.t("library.inUse")} ${del.users.join(", ")}.` : app.t("library.deleteMcpNote")}
        confirm={del?.users.length ? app.t("library.deleteForce") : app.t("common.delete")}
        onClose={() => setDel(null)}
        onConfirm={() =>
          api
            .removeMcp(del!.config.id, del!.users.length > 0)
            .then(() => (setDel(null), load()))
            .catch((e) => (setDel(null), app.notify(String(e))))
        }
      />
    </div>
  );
}

export function LibraryView() {
  const app = useApp();
  const [tab, setTab] = useState<"skills" | "mcp">("skills");
  return (
    <div className="space-y-4">
      <PageHeader title={app.t("nav.library")} sub={app.t("library.sub")}>
        <Segmented
          id="library-tab"
          value={tab}
          onChange={setTab}
          items={[
            { value: "skills", label: <><Wrench className="h-3.5 w-3.5" /> Skill</> },
            { value: "mcp", label: <><Plug className="h-3.5 w-3.5" /> MCP</> },
          ]}
        />
      </PageHeader>
      <PanelSwap id={tab} dir={tab === "mcp" ? 1 : -1}>
        {tab === "skills" ? <SkillsPanel /> : <McpPanel />}
      </PanelSwap>
    </div>
  );
}
