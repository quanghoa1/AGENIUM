// Capabilities the user manages (ADR-61): model providers (API key or local model; agent CLIs are detected), and the
// system's skill library and MCP servers that each agent switches on or off.
import { CheckCircle2, Eye, FileUp, KeyRound, Laptop, ListTree, Pencil, Plug, Plus, RefreshCw, Search, Trash2, Wrench, XCircle } from "lucide-react";
import { useEffect, useState } from "react";
import { Markdown } from "../components/Markdown";
import { Modal, PanelSwap, Segmented } from "../components/motion";
import { Empty, Mono, PageHeader, Section, cn } from "../components/ui";
import { api, inTauri } from "../lib/api";
import { useApp } from "../lib/store";
import type { Capabilities, LibrarySkill, LocalServer, McpServerConfig, McpServerRow, ModelInfo } from "../lib/types";

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
      })
      .then(() => (app.notify(app.t("common.done")), onDone()))
      .catch((e) => app.notify(String(e)));
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
          <span className="label">id</span>
          <input className="input mt-1" value={d.id} disabled={d.editing} placeholder="my-provider" onChange={(e) => set({ id: e.target.value.toLowerCase() })} />
        </label>
        <label>
          <span className="label">base URL</span>
          <input className="input mt-1 font-mono" value={d.base_url} disabled={d.kind !== "openai_compat"} placeholder={d.kind === "openai_compat" ? "https://…/v1" : app.t("providers.defaultUrl")} onChange={(e) => set({ base_url: e.target.value })} />
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
      {d.has_key && d.editing && (
        <label className="flex items-center gap-2 text-xs text-muted">
          <input type="checkbox" checked={d.clear_key} onChange={(e) => set({ clear_key: e.target.checked, api_key: "" })} /> {app.t("providers.clearKey")}
        </label>
      )}
      <div className="flex gap-2">
        <button className="btn-primary" disabled={!d.id.trim()} onClick={save}>
          {app.t("common.save")}
        </button>
        <button className="btn" onClick={onDone}>
          {app.t("common.cancel")}
        </button>
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

export function ProvidersView() {
  const app = useApp();
  const [c, setC] = useState<Capabilities | null>(null);
  const [draft, setDraft] = useState<ProviderDraft | null>(null);
  const [models, setModels] = useState<string | null>(null);
  const [del, setDel] = useState<string | null>(null);
  const [scanning, setScanning] = useState(false);
  const load = (refresh = false) => api.capabilities(refresh).then(setC);
  useEffect(() => {
    load();
  }, [app.tick]);
  if (!c) return <div className="text-xs text-subtle">{app.t("common.loading")}</div>;
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
              <>
                <tr key={p.id}>
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
                          })
                        }
                      >
                        <Pencil className="h-3.5 w-3.5" />
                      </button>
                      <button className="btn" onClick={() => setDel(p.id)}>
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
              </>
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
              <th>version</th>
              <th>login</th>
              <th>{app.t("providers.usable")}</th>
              <th>note</th>
            </tr>
          </thead>
          <tbody className="divide-y divide-line">
            {c.clis.map((x) => (
              <tr key={x.kind}>
                <td className="py-2 font-medium">{x.kind.toLowerCase()}</td>
                <td className="font-mono">{x.version ?? "–"}</td>
                <td className="text-muted">{typeof x.login === "string" ? x.login : JSON.stringify(x.login)}</td>
                <td>{x.compatible ? <CheckCircle2 className="h-4 w-4 text-emerald-500" /> : <XCircle className="h-4 w-4 text-subtle" />}</td>
                <td className="text-[11px] text-subtle">{x.note ?? (x.path ? "" : app.t("providers.notInstalled"))}</td>
              </tr>
            ))}
          </tbody>
        </table>
        <div className="mt-3 text-[11px] text-subtle">
          {app.t("providers.fallback")}: {c.cli_fallback.provider ? <Mono>{c.cli_fallback.provider}/{c.cli_fallback.model}</Mono> : app.t("providers.notSet")}
        </div>
      </Section>
      <Modal open={!!del} title={`${app.t("providers.remove")} ${del ?? ""}`} onClose={() => setDel(null)}>
        <p className="text-xs text-muted">{app.t("providers.removeNote")}</p>
        <div className="mt-4 flex justify-end gap-2">
          <button className="btn" onClick={() => setDel(null)}>
            {app.t("common.cancel")}
          </button>
          <button
            className="btn-danger"
            onClick={() =>
              api
                .removeProvider(del!)
                .then(() => (setDel(null), load(), app.notify(app.t("common.done"))))
                .catch((e) => (setDel(null), app.notify(String(e))))
            }
          >
            <Trash2 className="h-3.5 w-3.5" /> {app.t("common.delete")}
          </button>
        </div>
      </Modal>
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
      .then((m) => (app.notify(`${app.t("library.imported")} ${m.name}`), setPath(""), load()))
      .catch((e) => app.notify(String(e)));
  // Drop a skill folder or .zip on the window (Tauri gives the paths).
  useEffect(() => {
    if (!inTauri) return;
    let un: (() => void) | undefined;
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
        .then((f) => (un = f)),
    );
    return () => un?.();
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
                {s.files} {app.t("library.files")}
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
        {view && view.files.length > 0 && <div className="mt-3 font-mono text-[10px] text-subtle">{view.files.join(" · ")}</div>}
      </Modal>
      <Modal open={!!del} title={`${app.t("common.delete")} ${del?.name ?? ""}`} onClose={() => setDel(null)}>
        <p className="text-xs text-muted">{del?.users.length ? `${app.t("library.inUse")} ${del.users.join(", ")}.` : app.t("library.deleteNote")}</p>
        <div className="mt-4 flex justify-end gap-2">
          <button className="btn" onClick={() => setDel(null)}>
            {app.t("common.cancel")}
          </button>
          <button
            className="btn-danger"
            onClick={() =>
              api
                .removeSkill(del!.name, del!.users.length > 0)
                .then(() => (setDel(null), load()))
                .catch((e) => (setDel(null), app.notify(String(e))))
            }
          >
            <Trash2 className="h-3.5 w-3.5" /> {del?.users.length ? app.t("library.deleteForce") : app.t("common.delete")}
          </button>
        </div>
      </Modal>
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
  const load = () => api.mcpServers().then(setRows);
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
                <div className="ml-auto flex gap-1.5">
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
                        {app.t("library.connected")} · {t.tools.length} tool
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
      <Modal open={!!del} title={`${app.t("common.delete")} ${del?.config.id ?? ""}`} onClose={() => setDel(null)}>
        <p className="text-xs text-muted">{del?.users.length ? `${app.t("library.inUse")} ${del.users.join(", ")}.` : app.t("library.deleteMcpNote")}</p>
        <div className="mt-4 flex justify-end gap-2">
          <button className="btn" onClick={() => setDel(null)}>
            {app.t("common.cancel")}
          </button>
          <button
            className="btn-danger"
            onClick={() =>
              api
                .removeMcp(del!.config.id, del!.users.length > 0)
                .then(() => (setDel(null), load()))
                .catch((e) => (setDel(null), app.notify(String(e))))
            }
          >
            <Trash2 className="h-3.5 w-3.5" /> {del?.users.length ? app.t("library.deleteForce") : app.t("common.delete")}
          </button>
        </div>
      </Modal>
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
