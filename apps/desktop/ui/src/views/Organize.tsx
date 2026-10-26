import { AlertTriangle, ArrowLeft, Crown, Download, FileUp, FolderPlus, Lightbulb, Pencil, Plug, Plus, RotateCcw, Settings2, Trash2, UserMinus, UserPlus, Users, Wrench } from "lucide-react";
import { Fragment, useEffect, useMemo, useState } from "react";
import { ConfirmModal, Modal, Overlay, PanelSwap, Segmented } from "../components/motion";
import { AgentStatusBadge, Avatar, Empty, FreeTag, Mono, OutcomeBadge, PageHeader, RoleChip, RuntimeBadge, Section, ago, cn, tokens } from "../components/ui";
import { api, type ModelArg } from "../lib/api";
import { isFree, priceLabel, priceOf, usePrices } from "../lib/prices";
import { inTauri, useApp } from "../lib/store";
import type { AgentDetail, AgentEntry, AgentMcpKey, AgentRecord, Capabilities, LeadSuggestion, LibrarySkill, McpServerRow, ModelInfo, RoleInfo, TeamTemplate, TeamsData } from "../lib/types";

/** Provider (API) or CLI program, model and effort. The model list comes from the provider when it can list them. */
export function ModelPicker({ value, onChange, caps }: { value: ModelArg; onChange: (m: ModelArg) => void; caps: Capabilities | null }) {
  const cli = value.backend === "cli";
  const [models, setModels] = useState<ModelInfo[]>([]);
  useEffect(() => {
    setModels([]);
    if (cli) {
      // What this CLI runs with the user's login (codex model/list, agy models, Claude aliases, OpenCode's free models).
      setModels((caps?.cli_models?.[value.provider] ?? []).map((id) => ({ id })));
      return;
    }
    if (!value.provider) return;
    const listed = caps?.providers.find((p) => p.id === value.provider)?.models ?? [];
    if (listed.length) setModels(listed.map((id) => ({ id })));
    api
      .providerModels(value.provider)
      .then((ms) => ms.length && setModels(ms))
      .catch(() => undefined);
  }, [value.provider, cli, caps]);
  const listId = `models-${value.provider || "none"}`;
  const prices = usePrices(useApp().tick);
  // Only the opencode CLI's list is OpenCode's free models; the other CLIs list what their login can run.
  const freeCli = new Set(cli && value.provider === "opencode" ? (caps?.cli_models?.[value.provider] ?? []) : []);
  const price = (id: string) => (cli ? (freeCli.has(id) ? { input: 0, output: 0, cache_read: 0, cache_write: 0 } : undefined) : priceOf(prices, value.provider, id));
  return (
    <div className="grid grid-cols-4 gap-2">
      <select className="input" value={value.backend ?? "api"} onChange={(e) => onChange({ ...value, backend: e.target.value as "api" | "cli", provider: "", model: "" })}>
        <option value="api">API</option>
        <option value="cli">CLI</option>
      </select>
      <select className="input" value={value.provider} onChange={(e) => onChange({ ...value, provider: e.target.value, model: "" })}>
        <option value="">{cli ? "CLI…" : "provider…"}</option>
        {cli
          ? caps?.clis
              .filter((c) => c.compatible)
              .map((c) => (
                <option key={c.kind} value={c.kind.toLowerCase()}>
                  {c.kind.toLowerCase()} {c.version}
                </option>
              ))
          : caps?.providers.map((p) => (
              <option key={p.id} value={p.id}>
                {p.id}
              </option>
            ))}
      </select>
      <div className="relative">
        <input className={cn("input w-full", isFree(price(value.model)) && "pr-14")} list={listId} placeholder="model" value={value.model} onChange={(e) => onChange({ ...value, model: e.target.value })} />
        {isFree(price(value.model)) && <FreeTag className="absolute right-2 top-1/2 -translate-y-1/2" />}
      </div>
      <datalist id={listId}>
        {models.map((m) => {
          const p = price(m.id);
          return (
            <option key={m.id} value={m.id}>
              {[isFree(p) ? "FREE" : p ? `${priceLabel(p)} per 1M` : "", m.supports_tools === false ? "no tool calling" : ""].filter(Boolean).join(" · ")}
            </option>
          );
        })}
      </datalist>
      <select className="input" value={value.effort} onChange={(e) => onChange({ ...value, effort: e.target.value })}>
        {["low", "medium", "high", "xhigh", "max"].map((x) => (
          <option key={x}>{x}</option>
        ))}
      </select>
    </div>
  );
}

const emptyModel: ModelArg = { provider: "", model: "", effort: "high", backend: "api" };

export function ProjectsView() {
  const app = useApp();
  const [name, setName] = useState("");
  const [source, setSource] = useState("");
  return (
    <div className="space-y-4">
      <PageHeader title={app.t("nav.projects")} />
      <div className="card flex flex-wrap items-end gap-2 p-4">
        <label className="w-48">
          <span className="label">{app.t("common.name")}</span>
          <input className="input mt-1" value={name} onChange={(e) => setName(e.target.value)} />
        </label>
        <label className="min-w-[280px] flex-1">
          <span className="label">{app.t("projects.folder")}</span>
          <input className="input mt-1" value={source} placeholder={app.t("projects.folderHint")} onChange={(e) => setSource(e.target.value)} />
        </label>
        <button
          className="btn-primary"
          disabled={!name.trim()}
          onClick={() =>
            api
              .createProject(name.trim(), source.trim() || undefined)
              .then((p) => (app.setProject(p.id), setName(""), setSource(""), app.refresh(), app.go("teams")))
              .catch((e) => app.notify(String(e)))
          }
        >
          <FolderPlus className="h-3.5 w-3.5" /> {app.t("projects.create")}
        </button>
      </div>
      <div className="grid gap-4 lg:grid-cols-2">
        {app.projects.map((p) => (
          <div key={p.id} className="card p-4">
            <div className="flex items-center justify-between">
              <div>
                <Mono className="text-cyan-600 dark:text-cyan-400">{p.key_prefix}</Mono> <span className="text-sm font-semibold">{p.name}</span>
              </div>
              <span className="chip border-line-strong bg-elevated text-muted">{p.status}</span>
            </div>
            <div className="mt-3 grid grid-cols-4 gap-2 text-center text-xs">
              {[
                [app.t("nav.teams"), p.team ? (p.team.solo ? "solo" : p.team.members) : "–"],
                [app.t("nav.tickets"), p.tickets],
                ["plan", `${p.plan_done}/${p.plan_items}`],
                [app.t("nav.needs"), p.needs_you],
              ].map(([k, v]) => (
                <div key={String(k)} className="rounded-lg border border-line bg-elevated/50 p-2">
                  <div className="label">{k}</div>
                  <div className="mt-1 font-mono">{v}</div>
                </div>
              ))}
            </div>
            <div className="mt-3 flex gap-2">
              <button className="btn" onClick={() => (app.setProject(p.id), app.go("tickets"))}>
                {app.t("nav.tickets")}
              </button>
              <button className="btn" onClick={() => (app.setProject(p.id), app.go("chat"))}>
                {app.t("top.talk")}
              </button>
              {!p.team && (
                <button className="btn-primary" onClick={() => (app.setProject(p.id), app.go("teams"))}>
                  <Users className="h-3.5 w-3.5" /> {app.t("projects.addTeam")}
                </button>
              )}
            </div>
          </div>
        ))}
      </div>
      {app.projects.length === 0 && <Empty>{app.t("common.none")}</Empty>}
    </div>
  );
}

function AgentModel({ id, backend, model, onSaved }: { id: string; backend: string; model: { provider: string; model: string; effort: string }; onSaved: () => void }) {
  const app = useApp();
  const [caps, setCaps] = useState<Capabilities | null>(null);
  const [m, setM] = useState<ModelArg>({ provider: model.provider, model: model.model, effort: model.effort, backend: backend === "Cli" ? "cli" : "api" });
  useEffect(() => {
    api.capabilities().then(setCaps);
  }, []);
  return (
    <div className="space-y-2">
      <ModelPicker value={m} onChange={setM} caps={caps} />
      <div className="text-[11px] text-subtle">{app.t("agents.keep")}</div>
      <button
        className="btn-primary"
        disabled={!m.provider}
        onClick={() =>
          api
            .setAgentModel(id, m)
            .then(() => (onSaved(), app.refresh(), app.notify(app.t("common.done"))))
            .catch((e) => app.notify(String(e)))
        }
      >
        {app.t("common.save")}
      </button>
    </div>
  );
}

// ---------------------------------------------------------------- roles

const NEW_ROLE = "__new__";

/** Role picker: the user's roles, then template presets, then "new role" with its editor. */
function RolePicker({ roles, value, onChange, draft, setDraft }: { roles: RoleInfo[]; value: string; onChange: (id: string) => void; draft: RoleDraft; setDraft: (d: RoleDraft) => void }) {
  const app = useApp();
  const mine = roles.filter((r) => !r.template);
  const presets = roles.filter((r) => r.template && r.template !== LIBRARY);
  const library = roles.filter((r) => r.template === LIBRARY);
  const r = roles.find((x) => x.id === value);
  return (
    <div className="space-y-2">
      <select className="input" value={value} onChange={(e) => onChange(e.target.value)}>
        <option value="">{app.t("agents.pickRole")}</option>
        {mine.length > 0 && (
          <optgroup label={app.t("agents.yourRoles")}>
            {mine.map((x) => (
              <option key={x.id} value={x.id}>
                {x.id}
                {x.lead ? " ★" : ""}
              </option>
            ))}
          </optgroup>
        )}
        <optgroup label={app.t("agents.presets")}>
          {presets.map((x) => (
            <option key={x.id} value={x.id}>
              {x.id}
              {x.lead ? " ★ lead" : x.planner ? " · planner" : ""}
            </option>
          ))}
        </optgroup>
        {library.length > 0 && (
          <optgroup label={app.t("agents.library")}>
            {library.map((x) => (
              <option key={x.id} value={x.id}>
                {x.id}
              </option>
            ))}
          </optgroup>
        )}
        <option value={NEW_ROLE}>+ {app.t("agents.newRole")}</option>
      </select>
      {r && (
        <div className="rounded-lg border border-line bg-elevated/40 p-2.5 text-[11px]">
          <div className="line-clamp-3 text-muted">{r.prompt}</div>
          <div className="mt-1.5 flex flex-wrap gap-1">
            {r.lead && <span className="chip border-purple-500/30 text-purple-600 dark:text-purple-300">lead</span>}
            {r.planner && <span className="chip border-cyan-500/30 text-cyan-600 dark:text-cyan-300">planner</span>}
            {r.tags.map((t) => (
              <span key={t} className="chip border-line-strong text-muted">
                {t}
              </span>
            ))}
            {r.allowed_paths.length > 0 && <Mono className="text-subtle">scope: {r.allowed_paths.join(", ")}</Mono>}
          </div>
          <RoleLint lint={r.lint} />
        </div>
      )}
      {value === NEW_ROLE && <RoleEditor draft={draft} setDraft={setDraft} />}
    </div>
  );
}

interface RoleDraft {
  id: string;
  prompt: string;
  tags: string;
  paths: string;
  lead: boolean;
  planner: boolean;
}

/** The sections of every ROLE.md (same list as `role_library::ROLE_SECTIONS`, ADR-64). */
const ROLE_SECTIONS = ["Mission", "Rules", "Deliverables", "Workflow", "Done when"];
const ROLE_SKELETON = ROLE_SECTIONS.map((s) => `## ${s}\n`).join("\n");
const LIBRARY = "library@1";

const emptyRole: RoleDraft = { id: "", prompt: ROLE_SKELETON, tags: "", paths: "", lead: false, planner: false };

/** Sections still missing or empty in a role text (the server's check adds length and near-duplicates). */
function draftLint(text: string): string[] {
  const body: Record<string, string> = {};
  let cur: string | null = null;
  for (const line of text.split("\n")) {
    if (line.startsWith("## ")) body[(cur = line.slice(3).trim().toLowerCase())] = "";
    else if (cur) body[cur] += line;
  }
  return ROLE_SECTIONS.filter((s) => !body[s.toLowerCase()]?.trim()).map((s) => `## ${s}`);
}

function RoleLint({ lint }: { lint: string[] }) {
  if (!lint?.length) return null;
  return (
    <ul className="mt-1.5 space-y-0.5 text-amber-600 dark:text-amber-300">
      {lint.map((w) => (
        <li key={w} className="flex items-start gap-1">
          <AlertTriangle className="mt-px h-3 w-3 shrink-0" /> {w}
        </li>
      ))}
    </ul>
  );
}

function RoleEditor({ draft, setDraft }: { draft: RoleDraft; setDraft: (d: RoleDraft) => void }) {
  const app = useApp();
  const set = (p: Partial<RoleDraft>) => setDraft({ ...draft, ...p });
  return (
    <div className="grid gap-2 rounded-lg border border-accent/40 bg-blue-600/5 p-3 md:grid-cols-2">
      <label>
        <span className="label">role id</span>
        <input className="input mt-1" value={draft.id} placeholder="data-analyst" onChange={(e) => set({ id: e.target.value.toLowerCase().replace(/[^a-z0-9-]/g, "-") })} />
      </label>
      <label>
        <span className="label">tags</span>
        <input className="input mt-1" value={draft.tags} placeholder="data, csv" onChange={(e) => set({ tags: e.target.value })} />
      </label>
      <label className="md:col-span-2">
        <span className="label">{app.t("agents.roleText")}</span>
        <textarea className="input mt-1 min-h-[220px] font-mono text-xs" value={draft.prompt} placeholder={app.t("agents.roleTextHint")} onChange={(e) => set({ prompt: e.target.value })} />
        <div className="mt-1 text-[11px] text-subtle">{app.t("agents.roleSections")}</div>
        {draftLint(draft.prompt).length > 0 && (
          <div className="text-[11px]">
            <RoleLint lint={[`${app.t("agents.roleMissing")}: ${draftLint(draft.prompt).join(", ")}`]} />
          </div>
        )}
      </label>
      <label className="md:col-span-2">
        <span className="label">{app.t("agents.rolePaths")}</span>
        <input className="input mt-1 font-mono" value={draft.paths} placeholder="src/api/**, tests/api/**" onChange={(e) => set({ paths: e.target.value })} />
      </label>
      <div className="flex flex-wrap gap-4 text-xs md:col-span-2">
        <label className="flex items-center gap-1.5">
          <input type="checkbox" checked={draft.lead} onChange={(e) => set({ lead: e.target.checked, planner: e.target.checked ? false : draft.planner })} /> {app.t("agents.roleLead")}
        </label>
        <label className="flex items-center gap-1.5">
          <input type="checkbox" checked={draft.planner} onChange={(e) => set({ planner: e.target.checked, lead: e.target.checked ? false : draft.lead })} /> {app.t("agents.rolePlanner")}
        </label>
      </div>
    </div>
  );
}

const list = (s: string) =>
  s
    .split(",")
    .map((x) => x.trim())
    .filter(Boolean);

async function saveRoleIfNew(role: string, draft: RoleDraft): Promise<string> {
  if (role !== NEW_ROLE) return role;
  const r = await api.putRole({ id: draft.id, prompt: draft.prompt, tags: list(draft.tags), allowed_paths: list(draft.paths), planner: draft.planner, lead: draft.lead });
  return r.id;
}

// ---------------------------------------------------------------- agents

/** Switches for the library skills and the system's MCP servers. */
function Switches({ skills, mcp, onSkills, onMcp, lib, servers }: { skills: string[]; mcp: string[]; onSkills: (name: string, on: boolean) => void; onMcp: (id: string, on: boolean) => void; lib: LibrarySkill[]; servers: McpServerRow[] }) {
  const app = useApp();
  return (
    <div className="grid gap-3 md:grid-cols-2">
      <div>
        <div className="label mb-1.5 flex items-center gap-1">
          <Wrench className="h-3 w-3" /> {app.t("agents.librarySkills")}
        </div>
        <div className="space-y-1.5">
          {lib.map((s) => (
            <Toggle key={s.name} on={skills.includes(s.name)} onChange={(v) => onSkills(s.name, v)} title={s.name} sub={s.description} />
          ))}
          {lib.length === 0 && (
            <button className="text-[11px] text-accent hover:underline" onClick={() => app.go("library")}>
              {app.t("agents.noLibrary")}
            </button>
          )}
        </div>
      </div>
      <div>
        <div className="label mb-1.5 flex items-center gap-1">
          <Plug className="h-3 w-3" /> MCP
        </div>
        <div className="space-y-1.5">
          {servers.map((s) => (
            <Toggle key={s.config.id} on={mcp.includes(s.config.id)} onChange={(v) => onMcp(s.config.id, v)} title={s.config.id} sub={s.config.url ?? s.config.command ?? ""} />
          ))}
          {servers.length === 0 && (
            <button className="text-[11px] text-accent hover:underline" onClick={() => app.go("library")}>
              {app.t("agents.noServers")}
            </button>
          )}
        </div>
      </div>
    </div>
  );
}

/** The keys of the MCP servers an agent uses: the system key, and the agent's own key when it has one. */
function McpKeys({ agent }: { agent: string }) {
  const app = useApp();
  const [rows, setRows] = useState<AgentMcpKey[]>([]);
  const [edit, setEdit] = useState<Record<string, string>>({});
  const load = () => api.agentMcpKeys(agent).then((r) => setRows(r ?? []));
  useEffect(() => {
    load();
  }, [agent, app.tick]);
  const id = (r: AgentMcpKey) => `${r.server}/${r.name}`;
  const save = (r: AgentMcpKey, value: string | null) =>
    api
      .setAgentMcpKey(agent, r.server, r.name, value)
      .then(() => (app.notify(app.t("common.done")), setEdit((e) => ({ ...e, [id(r)]: undefined as unknown as string })), load()))
      .catch((e) => app.notify(String(e)));
  return (
    <div className="space-y-2">
      <div className="text-[11px] text-subtle">{app.t("agents.mcpKeysNote")}</div>
      {rows.length === 0 && <div className="text-[11px] text-muted">{app.t("agents.noMcpKeys")}</div>}
      {rows.map((r) => {
        const draft = edit[id(r)] ?? r.own ?? "";
        return (
          <div key={id(r)} className="grid items-center gap-2 rounded-lg border border-line p-2.5 text-xs md:grid-cols-[minmax(0,1fr)_minmax(0,1.4fr)_auto]">
            <div className="min-w-0">
              <div className="flex items-center gap-1.5">
                <Plug className="h-3 w-3 text-purple-500" />
                <b>{r.server}</b>
                <Mono className="text-subtle">{r.name}</Mono>
                <span className={cn("chip", r.own ? "border-accent/40 text-accent" : "border-line-strong text-muted")}>{r.own ? app.t("agents.ownKey") : app.t("agents.systemKey")}</span>
              </div>
              <div className="mt-1 truncate font-mono text-[10px] text-subtle" title={r.system ?? ""}>
                {app.t("agents.systemKey")}: {r.system || "–"}
              </div>
            </div>
            <input className="input font-mono text-[11px]" value={draft} placeholder={app.t("agents.ownKeyHint")} onChange={(e) => setEdit((x) => ({ ...x, [id(r)]: e.target.value }))} />
            <div className="flex gap-1.5">
              <button className="btn-primary" disabled={!draft.trim() || draft === (r.own ?? "")} onClick={() => save(r, draft)}>
                {app.t("common.save")}
              </button>
              {r.own && (
                <button className="btn" onClick={() => save(r, null)}>
                  {app.t("agents.useSystem")}
                </button>
              )}
            </div>
          </div>
        );
      })}
    </div>
  );
}

function Toggle({ on, onChange, title, sub }: { on: boolean; onChange: (v: boolean) => void; title: string; sub?: string }) {
  return (
    <button type="button" onClick={() => onChange(!on)} className={cn("press flex w-full items-center gap-3 rounded-lg border p-2 text-left", on ? "border-accent/50 bg-blue-600/10" : "border-line hover:border-line-strong")}>
      <span className={cn("relative inline-flex h-4 w-7 shrink-0 rounded-full transition-colors", on ? "bg-accent" : "bg-line-strong")}>
        <span className={cn("absolute top-0.5 h-3 w-3 rounded-full bg-white shadow transition-all", on ? "left-3.5" : "left-0.5")} />
      </span>
      <span className="min-w-0">
        <span className="block truncate text-xs font-medium">{title}</span>
        {sub && <span className="block truncate text-[10px] text-subtle">{sub}</span>}
      </span>
    </button>
  );
}

function useLibrary(tick: number) {
  const [lib, setLib] = useState<LibrarySkill[]>([]);
  const [servers, setServers] = useState<McpServerRow[]>([]);
  useEffect(() => {
    api.skillLibrary().then((l) => setLib(l?.skills ?? []));
    api.mcpServers().then((s) => setServers(s ?? []));
  }, [tick]);
  return { lib, servers };
}

/** New agent for the pool: name, role, its own model, the library skills and MCP servers it uses. */
function AgentCreate({ open, onClose }: { open: boolean; onClose: () => void }) {
  const app = useApp();
  const [roles, setRoles] = useState<RoleInfo[]>([]);
  const [caps, setCaps] = useState<Capabilities | null>(null);
  const { lib, servers } = useLibrary(app.tick);
  const [name, setName] = useState("");
  const [role, setRole] = useState("");
  const [draft, setDraft] = useState<RoleDraft>(emptyRole);
  const [model, setModel] = useState<ModelArg>(emptyModel);
  const [skills, setSkills] = useState<string[]>([]);
  const [mcp, setMcp] = useState<string[]>([]);
  const [busy, setBusy] = useState(false);
  useEffect(() => {
    if (!open) return;
    api.roles().then((r) => setRoles(r ?? []));
    api.capabilities().then((c) => {
      setCaps(c);
      if (c?.providers[0]) setModel((m) => ({ ...m, provider: m.provider || c.providers[0].id }));
    });
  }, [open]);
  const flip = (xs: string[], x: string, on: boolean) => (on ? [...new Set([...xs, x])] : xs.filter((y) => y !== x));
  const ready = name.trim() && role && (role !== NEW_ROLE || (draft.id && !draftLint(draft.prompt).includes("## Mission"))) && model.provider && (model.backend === "cli" || model.model);
  const create = async () => {
    setBusy(true);
    try {
      const r = await saveRoleIfNew(role, draft);
      const a = await api.createAgent({ name: name.trim(), role: r, model, skills, mcp });
      app.notify(`${a.name}: ${app.t("agents.inPool")}`);
      setName("");
      setRole("");
      setDraft(emptyRole);
      setSkills([]);
      setMcp([]);
      app.refresh();
      onClose();
    } catch (e) {
      app.notify(String(e));
    } finally {
      setBusy(false);
    }
  };
  return (
    <Overlay open={open} onClose={onClose} kind="dialog" top="pt-10">
      <div className="card max-h-[86vh] w-[760px] max-w-[94vw] overflow-y-auto p-5 shadow-2xl">
        <div className="mb-4 flex items-center gap-2 text-sm font-semibold">
          <UserPlus className="h-4 w-4 text-accent" /> {app.t("agents.create")}
        </div>
        <div className="space-y-4">
          <label className="block">
            <span className="label">{app.t("common.name")}</span>
            <input className="input mt-1" value={name} placeholder="recon-worker" onChange={(e) => setName(e.target.value)} autoFocus />
          </label>
          <div>
            <div className="label mb-1">role</div>
            <RolePicker roles={roles} value={role} onChange={setRole} draft={draft} setDraft={setDraft} />
          </div>
          <div>
            <div className="label mb-1">{app.t("common.model")}</div>
            <ModelPicker value={model} onChange={setModel} caps={caps} />
          </div>
          <Switches skills={skills} mcp={mcp} lib={lib} servers={servers} onSkills={(s, on) => setSkills((xs) => flip(xs, s, on))} onMcp={(s, on) => setMcp((xs) => flip(xs, s, on))} />
          <div className="text-[11px] text-subtle">{app.t("agents.poolNote")}</div>
          <div className="flex justify-end gap-2">
            <button className="btn" onClick={onClose}>
              {app.t("common.cancel")}
            </button>
            <button className="btn-primary" disabled={!ready || busy} onClick={create}>
              <UserPlus className="h-3.5 w-3.5" /> {app.t("common.create")}
            </button>
          </div>
        </div>
      </div>
    </Overlay>
  );
}

/** Imports an agent exported elsewhere (`.agenium.jsonl`: memory, skills, enablement; no secrets) as a project's
 *  Solo agent. The file is named by its path or dropped on the window. */
function AgentImport({ open, onClose }: { open: boolean; onClose: () => void }) {
  const app = useApp();
  const free = app.projects.filter((p) => !p.team);
  const [file, setFile] = useState("");
  const [project, setProject] = useState("");
  const [name, setName] = useState("");
  const [busy, setBusy] = useState(false);
  useEffect(() => {
    if (open) setProject((cur) => (free.some((p) => p.id === cur) ? cur : (free[0]?.id ?? "")));
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [open, app.projects]);
  // A file dropped on the window while the dialog is open (the listener lands asynchronously; one that lands after
  // the dialog closed is removed at once).
  useEffect(() => {
    if (!open || !inTauri) return;
    let un: (() => void) | undefined;
    let gone = false;
    import("@tauri-apps/api/webview").then(({ getCurrentWebview }) =>
      getCurrentWebview()
        .onDragDropEvent((e) => {
          if (e.payload.type === "drop" && e.payload.paths[0]) setFile(e.payload.paths[0]);
        })
        .then((f) => (gone ? f() : (un = f))),
    );
    return () => {
      gone = true;
      un?.();
    };
  }, [open]);
  const run = () => {
    setBusy(true);
    api
      .importAgent(file.trim(), project, name.trim() || undefined)
      .then((a) => (app.notify(`${app.t("agents.imported")} ${a.name}`), setFile(""), setName(""), onClose(), app.refresh(), app.go("agent", a.id)))
      .catch((e) => app.notify(String(e)))
      .finally(() => setBusy(false));
  };
  return (
    <Modal open={open} title={app.t("agents.import")} onClose={onClose}>
      <p className="text-[11px] text-subtle">{app.t("agents.importNote")}</p>
      <label className="mt-3 block text-xs">
        <span className="label">{app.t("agents.importFile")}</span>
        <input className="input mt-1 font-mono" value={file} placeholder="…\name.agenium.jsonl" onChange={(e) => setFile(e.target.value)} />
      </label>
      <label className="mt-3 block text-xs">
        <span className="label">{app.t("nav.projects")}</span>
        <select className="input mt-1" value={project} onChange={(e) => setProject(e.target.value)}>
          {free.map((p) => (
            <option key={p.id} value={p.id}>
              {p.key_prefix} — {p.name}
            </option>
          ))}
        </select>
      </label>
      {free.length === 0 && <div className="mt-1 text-[11px] text-amber-600 dark:text-amber-400">{app.t("agents.importNoProject")}</div>}
      <label className="mt-3 block text-xs">
        <span className="label">{app.t("agents.importName")}</span>
        <input className="input mt-1" value={name} onChange={(e) => setName(e.target.value)} />
      </label>
      <div className="mt-4 flex justify-end gap-2">
        <button className="btn" onClick={onClose}>
          {app.t("common.cancel")}
        </button>
        <button className="btn-primary" disabled={busy || !file.trim() || !project} onClick={run}>
          <FileUp className="h-3.5 w-3.5" /> {app.t("agents.import")}
        </button>
      </div>
    </Modal>
  );
}

export function AgentsView() {
  const app = useApp();
  const [rows, setRows] = useState<AgentEntry[]>([]);
  const [edit, setEdit] = useState<string | null>(null);
  const [creating, setCreating] = useState(false);
  const [importing, setImporting] = useState(false);
  const [filter, setFilter] = useState<"all" | "team" | "pool">("all");
  useEffect(() => {
    api.agents().then((r) => setRows(r ?? []));
  }, [app.tick]);
  const shown = rows.filter((r) => filter === "all" || (filter === "pool" ? r.pool : !r.pool));
  return (
    <div className="space-y-4">
      <PageHeader title={app.t("nav.agents")} sub={`${rows.length} · ${rows.filter((r) => r.pool).length} ${app.t("agents.pool")}`}>
        <Segmented
          id="agent-filter"
          value={filter}
          onChange={setFilter}
          items={[
            { value: "all", label: app.t("agents.all") },
            { value: "team", label: app.t("agents.inTeam") },
            { value: "pool", label: app.t("agents.pool") },
          ]}
        />
        <button className="btn" onClick={() => setImporting(true)}>
          <FileUp className="h-3.5 w-3.5" /> {app.t("agents.import")}
        </button>
        <button className="btn-primary" onClick={() => setCreating(true)}>
          <UserPlus className="h-3.5 w-3.5" /> {app.t("agents.create")}
        </button>
      </PageHeader>
      <div className="grid gap-4 lg:grid-cols-2 2xl:grid-cols-3">
        {shown.map((r) => {
          const a = r.agent;
          return (
            <div key={a.id} className="card p-4">
              <button className="flex w-full items-start gap-3 text-left" onClick={() => app.go("agent", a.id)}>
                <Avatar name={a.name} className="h-10 w-10" />
                <div className="min-w-0 flex-1">
                  <div className="flex items-center gap-2">
                    <span className="truncate text-sm font-semibold hover:text-accent">{a.name}</span>
                    {r.pool ? <span className="chip border-dashed border-line-strong text-subtle">POOL</span> : <RoleChip solo={r.solo} lead={r.is_lead} planner={r.is_planner} />}
                  </div>
                  <div className="truncate text-[11px] text-subtle">
                    {a.role}
                    {r.team ? ` · ${r.team}` : ""}
                    {r.project_name ? ` · ${r.project_name}` : ""}
                  </div>
                </div>
                <AgentStatusBadge status={a.status} />
              </button>
              {(a.tags.length > 0 || a.skills.length > 0) && (
                <div className="mt-3 flex flex-wrap gap-1">
                  {a.tags.map((t) => (
                    <span key={t} className="chip border-line-strong bg-elevated text-muted">
                      {t}
                    </span>
                  ))}
                  {a.skills.map((s) => (
                    <span key={s} className="chip border-cyan-500/30 text-cyan-600 dark:text-cyan-300">
                      <Wrench className="h-2.5 w-2.5" /> {s}
                    </span>
                  ))}
                </div>
              )}
              <div className="mt-3 flex items-center justify-between gap-2">
                <RuntimeBadge backend={a.backend} model={a.model} />
                <div className="flex gap-1.5">
                  {(a.status === "QuotaExhausted" || a.status === "Offline") && (
                    <button className="btn" title={app.t("agents.toIdle")} onClick={() => api.setAgentIdle(a.id).then(app.refresh)}>
                      <RotateCcw className="h-3.5 w-3.5" />
                    </button>
                  )}
                  <button className="btn" onClick={() => setEdit(edit === a.id ? null : a.id)}>
                    <Settings2 className="h-3.5 w-3.5" /> {app.t("common.model")}
                  </button>
                </div>
              </div>
              {edit === a.id && (
                <div className="mt-3 rounded-lg border border-line bg-elevated/50 p-3">
                  <AgentModel id={a.id} backend={a.backend} model={a.model} onSaved={() => setEdit(null)} />
                </div>
              )}
            </div>
          );
        })}
      </div>
      {shown.length === 0 && (
        <Empty>
          {app.t("agents.empty")}{" "}
          <button className="text-accent hover:underline" onClick={() => setCreating(true)}>
            {app.t("agents.create")}
          </button>
        </Empty>
      )}
      <AgentCreate open={creating} onClose={() => setCreating(false)} />
      <AgentImport open={importing} onClose={() => setImporting(false)} />
    </div>
  );
}

function AgentRole({ agent }: { agent: AgentRecord }) {
  const app = useApp();
  const [roles, setRoles] = useState<RoleInfo[]>([]);
  const [role, setRole] = useState(agent.role);
  const [draft, setDraft] = useState<RoleDraft>(emptyRole);
  useEffect(() => {
    api.roles().then((r) => setRoles(r ?? []));
  }, []);
  return (
    <div className="space-y-2">
      <RolePicker roles={roles} value={role} onChange={setRole} draft={draft} setDraft={setDraft} />
      <button
        className="btn-primary"
        disabled={!role || role === agent.role}
        onClick={() =>
          saveRoleIfNew(role, draft)
            .then((r) => api.setAgentRole(agent.id, r))
            .then(() => (app.notify(app.t("common.done")), app.refresh()))
            .catch((e) => app.notify(String(e)))
        }
      >
        {app.t("common.save")}
      </button>
    </div>
  );
}

function DeleteAgent({ agent }: { agent: AgentRecord }) {
  const app = useApp();
  const [open, setOpen] = useState(false);
  const [typed, setTyped] = useState("");
  return (
    <>
      <button className="btn-danger" onClick={() => setOpen(true)}>
        <Trash2 className="h-3.5 w-3.5" /> {app.t("agents.delete")}
      </button>
      <Modal open={open} title={`${app.t("agents.delete")} ${agent.name}`} onClose={() => setOpen(false)}>
        <p className="text-xs text-muted">{app.t("agents.deleteNote")}</p>
        <input className="input mt-3" value={typed} placeholder={agent.name} onChange={(e) => setTyped(e.target.value)} />
        <div className="mt-4 flex justify-end gap-2">
          <button className="btn" onClick={() => setOpen(false)}>
            {app.t("common.cancel")}
          </button>
          <button
            className="btn-danger"
            disabled={typed !== agent.name}
            onClick={() =>
              api
                .deleteAgent(agent.id, typed)
                .then(() => (setOpen(false), app.refresh(), app.go("agents")))
                .catch((e) => (setOpen(false), app.notify(String(e))))
            }
          >
            <Trash2 className="h-3.5 w-3.5" /> {app.t("common.delete")}
          </button>
        </div>
      </Modal>
    </>
  );
}

/** The agent's name with an inline rename (unique ignoring case; the backend refuses a taken name). */
function AgentName({ id, name }: { id: string; name: string }) {
  const app = useApp();
  const [draft, setDraft] = useState<string | null>(null);
  if (draft === null)
    return (
      <span className="flex items-center gap-1">
        <span className="text-lg font-semibold">{name}</span>
        <button className="rounded p-1 text-subtle hover:bg-elevated hover:text-fg" title={app.t("agents.rename")} onClick={() => setDraft(name)}>
          <Pencil className="h-3.5 w-3.5" />
        </button>
      </span>
    );
  const save = () =>
    api
      .renameAgent(id, draft)
      .then(() => (setDraft(null), app.refresh(), app.notify(app.t("common.done"))))
      .catch((e) => app.notify(String(e)));
  return (
    <span className="flex items-center gap-1.5">
      <input
        className="input h-8 w-56"
        autoFocus
        maxLength={60}
        value={draft}
        onChange={(e) => setDraft(e.target.value)}
        onKeyDown={(e) => (e.key === "Enter" ? save() : e.key === "Escape" ? setDraft(null) : undefined)}
      />
      <button className="btn-primary" disabled={!draft.trim() || draft.trim() === name} onClick={save}>
        {app.t("common.save")}
      </button>
      <button className="btn" onClick={() => setDraft(null)}>
        {app.t("common.cancel")}
      </button>
    </span>
  );
}

export function AgentView({ id }: { id: string }) {
  const app = useApp();
  const [d, setD] = useState<AgentDetail | null>(null);
  const [entry, setEntry] = useState<AgentEntry | null>(null);
  const [tab, setTab] = useState<"overview" | "runs" | "memory" | "skills">("overview");
  const [dir, setDir] = useState(1);
  const { lib, servers } = useLibrary(app.tick);
  const order = ["overview", "runs", "memory", "skills"];
  useEffect(() => {
    api.agentDetail(id).then(setD).catch((e) => app.notify(String(e)));
    api.agents().then((xs) => setEntry(xs.find((x) => x.agent.id === id) ?? null));
  }, [id, app.tick]);
  if (!d) return <div className="text-xs text-subtle">{app.t("common.loading")}</div>;
  const a = d.agent;
  const canLead = entry && entry.team_id && !entry.solo && !entry.is_lead && !d.planner;
  const bound: string[] = Array.isArray(a.mcp) ? a.mcp.map((b: { server: string }) => b.server) : [];
  const ownSkills = d.skills.filter((s) => String(s.origin) !== "library");
  const toggle = (p: Promise<unknown>) => p.then(() => app.refresh()).catch((e) => app.notify(String(e)));
  return (
    <div className="space-y-4">
      <button className="btn" onClick={() => app.go("agents")}>
        <ArrowLeft className="h-3.5 w-3.5" /> {app.t("nav.agents")}
      </button>
      <div className="card flex flex-wrap items-center gap-4 p-5">
        <Avatar name={a.name} className="h-12 w-12 text-sm" />
        <div className="min-w-0 flex-1">
          <div className="flex items-center gap-2">
            <AgentName id={a.id} name={a.name} />
            {entry && (entry.pool ? <span className="chip border-dashed border-line-strong text-subtle">POOL</span> : <RoleChip solo={entry.solo} lead={entry.is_lead} planner={d.planner} />)}
            <AgentStatusBadge status={a.status} />
          </div>
          <div className="mt-0.5 text-xs text-subtle">
            {a.role}
            {entry?.team ? ` · ${entry.team}` : ` · ${app.t("agents.noTeam")}`}
            {entry?.project_name ? ` · ${entry.project_name}` : ""}
          </div>
        </div>
        <RuntimeBadge backend={a.backend} model={a.model} />
        {canLead && (
          <button
            className="btn"
            onClick={() =>
              api
                .setLeadOfTeam(entry!.team_id!, a.id)
                .then(() => (app.notify(app.t("common.done")), app.refresh()))
                .catch((e) => app.notify(String(e)))
            }
          >
            <Crown className="h-3.5 w-3.5 text-purple-500" /> {app.t("agents.makeLead")}
          </button>
        )}
        <button
          className="btn"
          title={app.t("agents.exportNote")}
          onClick={() =>
            api
              .exportAgent(a.id, "")
              .then((f) => app.notify(`${app.t("agents.exported")} ${f}`))
              .catch((e) => app.notify(String(e)))
          }
        >
          <Download className="h-3.5 w-3.5" /> {app.t("agents.export")}
        </button>
      </div>
      <Segmented
        id="agent-tab"
        value={tab}
        onChange={(x) => (setDir(order.indexOf(x) >= order.indexOf(tab) ? 1 : -1), setTab(x))}
        itemClassName="flex-1 justify-center"
        items={[
          { value: "overview" as const, label: app.t("agents.overview") },
          { value: "runs" as const, label: app.t("nav.runs") },
          { value: "memory" as const, label: `${app.t("agents.memory")} (${d.facts_total})` },
          { value: "skills" as const, label: app.t("agents.skills") },
        ]}
      />
      <PanelSwap id={tab} dir={dir}>
        {tab === "overview" && (
          <div className="grid gap-4 md:grid-cols-2">
            <Section title={app.t("common.model")}>
              <AgentModel id={a.id} backend={a.backend} model={a.model} onSaved={() => undefined} />
            </Section>
            <Section title="role">
              <AgentRole agent={a} />
              <div className="mt-3 flex flex-wrap gap-1">
                {a.tags.map((t) => (
                  <span key={t} className="chip border-line-strong bg-elevated text-muted">
                    {t}
                  </span>
                ))}
              </div>
              {(a.status === "QuotaExhausted" || a.status === "Offline") && (
                <button className="btn mt-3" onClick={() => api.setAgentIdle(a.id).then(app.refresh)}>
                  <RotateCcw className="h-3.5 w-3.5" /> {app.t("agents.toIdle")}
                </button>
              )}
            </Section>
            <Section title={app.t("agents.danger")} className="md:col-span-2">
              <div className="flex flex-wrap items-center justify-between gap-3">
                <span className="text-xs text-muted">{app.t("agents.deleteNote")}</span>
                <DeleteAgent agent={a} />
              </div>
            </Section>
          </div>
        )}
        {tab === "runs" && (
          <div className="card divide-y divide-line">
            {d.runs.map((r) => (
              <button key={r.id} onClick={() => app.go("run", r.id)} className="flex w-full items-center gap-3 px-4 py-2.5 text-left text-xs hover:bg-elevated/60">
                <OutcomeBadge outcome={r.outcome} />
                <span className="text-muted">{r.model}</span>
                <Mono className="ml-auto text-subtle">
                  {r.steps} · {tokens(r.usage.tokens_in + r.usage.tokens_out)} · {ago(r.started_at)}
                </Mono>
              </button>
            ))}
            {d.runs.length === 0 && (
              <div className="p-4">
                <Empty>{app.t("common.none")}</Empty>
              </div>
            )}
          </div>
        )}
        {tab === "memory" && (
          <div className="space-y-2">
            {d.facts.map((f) => (
              <div key={f.id} className="card p-3">
                <div className="flex items-center gap-2">
                  <span className={cn("chip", f.kind === "lesson" ? "border-purple-500/30 text-purple-600 dark:text-purple-300" : f.kind === "preference" ? "border-cyan-500/30 text-cyan-600 dark:text-cyan-300" : "border-line-strong text-muted")}>{f.kind}</span>
                  {f.untrusted && <span className="chip border-amber-500/30 text-amber-600">untrusted</span>}
                  <Mono className="text-subtle">conf {f.confidence.toFixed(2)}</Mono>
                  {f.project && <Mono className="text-subtle">· {app.projects.find((p) => p.id === f.project)?.name ?? f.project}</Mono>}
                  <Mono className="ml-auto text-subtle">{f.source}</Mono>
                </div>
                <div className="mt-1.5 text-xs">{f.text}</div>
                <div className="mt-1 text-[10px] text-subtle">
                  {ago(f.created_at)}
                  {f.used_at ? ` · used ${ago(f.used_at)}` : ""}
                </div>
              </div>
            ))}
            {d.facts.length === 0 && <Empty>{app.t("agents.noFacts")}</Empty>}
          </div>
        )}
        {tab === "skills" && (
          <div className="space-y-4">
            <Section title={app.t("agents.switches")}>
              <Switches
                skills={a.skills}
                mcp={bound}
                lib={lib}
                servers={servers}
                onSkills={(s, on) => toggle(api.toggleAgentSkill(a.id, s, on))}
                onMcp={(s, on) => toggle(api.toggleAgentMcp(a.id, s, on))}
              />
            </Section>
            {bound.length > 0 && (
              <Section title={app.t("agents.mcpKeys")}>
                <McpKeys agent={a.id} />
              </Section>
            )}
            <Section title={app.t("agents.alwaysOn")}>
              <div className="space-y-2">
                {ownSkills.map((s) => (
                  <div key={s.name} className="rounded-lg border border-line p-2.5">
                    <div className="flex items-center gap-2">
                      <span className="text-xs font-semibold">{s.name}</span>
                      {s.version && <Mono className="text-subtle">v{s.version}</Mono>}
                      <span className="chip ml-auto border-line-strong text-muted">{String(s.origin)}</span>
                    </div>
                    <div className="mt-1 text-[11px] text-subtle">{s.description}</div>
                  </div>
                ))}
                {ownSkills.length === 0 && <Empty>{app.t("agents.noSkills")}</Empty>}
              </div>
            </Section>
          </div>
        )}
      </PanelSwap>
    </div>
  );
}

// ---------------------------------------------------------------- teams

/** Compose a team from pool agents: pick members, the Lead (the system suggests one, with its reasons), a project. */
function TeamComposer({ data, onDone }: { data: TeamsData; onDone: () => void }) {
  const app = useApp();
  const [name, setName] = useState("");
  const [solo, setSolo] = useState(false);
  const [picked, setPicked] = useState<string[]>([]);
  const [lead, setLead] = useState<string | null>(null);
  const [suggestion, setSuggestion] = useState<LeadSuggestion | null>(null);
  const [project, setProject] = useState<string>("");
  const [roles, setRoles] = useState<RoleInfo[]>([]);
  useEffect(() => {
    api.roles().then((r) => setRoles(r ?? []));
  }, []);
  const planner = (a: AgentRecord) => roles.find((r) => r.id === a.role)?.planner ?? false;
  useEffect(() => {
    if (solo || picked.length < 2) {
      setSuggestion(null);
      return;
    }
    api.suggestLead(picked).then(setSuggestion);
  }, [picked.join(","), solo]);
  const chosenLead = solo ? picked[0] : (lead ?? suggestion?.agent ?? null);
  const freeProjects = app.projects.filter((p) => !p.team);
  const pick = (id: string) =>
    setPicked((xs) => {
      if (solo) return xs[0] === id ? [] : [id];
      if (xs.includes(id)) {
        if (lead === id) setLead(null);
        return xs.filter((x) => x !== id);
      }
      return [...xs, id];
    });
  const ok = name.trim() && chosenLead && (solo ? picked.length === 1 : picked.length >= 2);
  const create = () =>
    api
      .composeTeam({ name: name.trim(), project: project || null, lead: chosenLead!, members: solo ? [] : picked.filter((x) => x !== chosenLead), solo })
      .then(() => (app.notify(app.t("common.done")), setName(""), setPicked([]), setLead(null), onDone(), app.refresh()))
      .catch((e) => app.notify(String(e)));
  const byId = new Map(data.pool.map((a) => [a.id, a]));
  return (
    <div className="space-y-3">
      <div className="flex flex-wrap items-end gap-3">
        <label className="w-56">
          <span className="label">{app.t("teams.name")}</span>
          <input className="input mt-1" value={name} placeholder="vuln-audit-team" onChange={(e) => setName(e.target.value)} />
        </label>
        <label className="w-64">
          <span className="label">{app.t("teams.project")}</span>
          <select className="input mt-1" value={project} onChange={(e) => setProject(e.target.value)}>
            <option value="">{app.t("teams.noProject")}</option>
            {freeProjects.map((p) => (
              <option key={p.id} value={p.id}>
                {p.key_prefix} — {p.name}
              </option>
            ))}
          </select>
        </label>
        <Segmented
          id="compose-kind"
          value={solo ? "solo" : "team"}
          onChange={(v) => (setSolo(v === "solo"), setPicked([]), setLead(null))}
          items={[
            { value: "team", label: app.t("teams.team") },
            { value: "solo", label: app.t("teams.solo") },
          ]}
        />
      </div>
      <div className="label">{solo ? app.t("teams.pickOne") : app.t("teams.pickMembers")}</div>
      <div className="grid gap-2 md:grid-cols-2 xl:grid-cols-3">
        {data.pool.map((a) => {
          const on = picked.includes(a.id);
          const isLead = on && chosenLead === a.id;
          return (
            <div key={a.id} className={cn("press flex items-center gap-3 rounded-lg border p-3", on ? "border-accent/60 bg-blue-600/10" : "border-line hover:border-line-strong")}>
              <button className="flex min-w-0 flex-1 items-center gap-3 text-left" onClick={() => pick(a.id)}>
                <input type="checkbox" readOnly checked={on} className="pointer-events-none" />
                <Avatar name={a.name} />
                <div className="min-w-0">
                  <div className="flex items-center gap-1.5">
                    <span className="truncate text-xs font-semibold">{a.name}</span>
                    {planner(a) && <span className="chip border-cyan-500/30 text-cyan-600 dark:text-cyan-300">planner</span>}
                  </div>
                  <div className="truncate text-[10px] text-subtle">
                    {a.role} · {a.backend === "Cli" ? `CLI:${a.model.provider}` : a.model.provider}/{a.model.model}
                  </div>
                </div>
              </button>
              {on && !solo && !planner(a) && (
                <button title={app.t("teams.makeLead")} onClick={() => setLead(a.id)} className={cn("rounded-md p-1.5", isLead ? "bg-purple-500/20 text-purple-500" : "text-subtle hover:bg-elevated")}>
                  <Crown className="h-4 w-4" />
                </button>
              )}
            </div>
          );
        })}
      </div>
      {data.pool.length === 0 && <Empty>{app.t("teams.poolEmpty")}</Empty>}
      {!solo && suggestion && (
        <div className="flex flex-wrap items-center gap-2 rounded-lg border border-purple-500/30 bg-purple-500/5 p-2.5 text-xs">
          <Lightbulb className="h-4 w-4 text-purple-500" />
          <span>
            {app.t("teams.suggested")} <b>{byId.get(suggestion.agent)?.name}</b> — {suggestion.reasons.join(", ")}
          </span>
          {lead && lead !== suggestion.agent && (
            <button className="btn ml-auto" onClick={() => setLead(null)}>
              {app.t("teams.useSuggestion")}
            </button>
          )}
        </div>
      )}
      {!solo && picked.length >= 2 && !chosenLead && <div className="text-[11px] text-amber-600 dark:text-amber-400">{app.t("teams.pickLead")}</div>}
      {chosenLead && (
        <div className="text-[11px] text-muted">
          <Crown className="mr-1 inline h-3 w-3 text-purple-500" />
          Lead: <b>{byId.get(chosenLead)?.name}</b>
          {lead === null && suggestion?.agent === chosenLead ? ` (${app.t("teams.bySuggestion")})` : ""}
        </div>
      )}
      <button className="btn-primary" disabled={!ok} onClick={create}>
        <Users className="h-3.5 w-3.5" /> {app.t("teams.compose")}
      </button>
    </div>
  );
}

function TemplateQuickStart() {
  const app = useApp();
  const [templates, setTemplates] = useState<TeamTemplate[]>([]);
  const [caps, setCaps] = useState<Capabilities | null>(null);
  const [tpl, setTpl] = useState("coding");
  const [model, setModel] = useState<ModelArg>(emptyModel);
  const [leadModel, setLeadModel] = useState<ModelArg | null>(null);
  const [plannerModel, setPlannerModel] = useState<ModelArg | null>(null);
  const [project, setProject] = useState(app.projects.find((p) => p.id === app.project && !p.team)?.id ?? "");
  useEffect(() => {
    api.templates().then((t) => setTemplates(t ?? []));
    api.capabilities().then((c) => {
      setCaps(c);
      if (c?.providers[0]) setModel((m) => ({ ...m, provider: m.provider || c.providers[0].id }));
    });
  }, []);
  const t = templates.find((x) => x.id === tpl);
  const free = app.projects.filter((p) => !p.team);
  return (
    <div className="space-y-3">
      <div className="text-[11px] text-subtle">{app.t("teams.templateNote")}</div>
      <select className="input w-64" value={project} onChange={(e) => setProject(e.target.value)}>
        <option value="">{app.t("teams.pickProject")}</option>
        {free.map((p) => (
          <option key={p.id} value={p.id}>
            {p.key_prefix} — {p.name}
          </option>
        ))}
      </select>
      <div className="grid gap-2 md:grid-cols-4">
        {templates.map((x) => (
          <button key={x.id} onClick={() => setTpl(x.id)} className={cn("press rounded-lg border p-3 text-left hover:shadow-md", tpl === x.id ? "border-accent bg-blue-600/10" : "border-line hover:border-line-strong")}>
            <div className="text-xs font-semibold">{x.name}</div>
            <div className="mt-1 text-[11px] text-subtle">{x.description}</div>
            <div className="mt-2 flex flex-wrap gap-1">
              {x.roles.map((r) => (
                <span key={r.id} className={cn("chip", r.kind === "lead" ? "border-purple-500/30 text-purple-600 dark:text-purple-300" : r.planner ? "border-cyan-500/30 text-cyan-600 dark:text-cyan-300" : "border-line-strong bg-elevated text-muted")}>
                  {r.id}
                  {r.count > 1 ? ` ×${r.count}` : ""}
                </span>
              ))}
            </div>
          </button>
        ))}
      </div>
      <div>
        <div className="label mb-1">{app.t("teams.modelAll")}</div>
        <ModelPicker value={model} onChange={setModel} caps={caps} />
      </div>
      <div className="flex flex-wrap gap-3 text-xs">
        <label className="flex items-center gap-1.5">
          <input type="checkbox" checked={!!leadModel} onChange={(e) => setLeadModel(e.target.checked ? { ...model } : null)} /> {app.t("teams.leadModel")}
        </label>
        {t?.roles.some((r) => r.planner) && (
          <label className="flex items-center gap-1.5">
            <input type="checkbox" checked={!!plannerModel} onChange={(e) => setPlannerModel(e.target.checked ? { ...model } : null)} /> {app.t("teams.plannerModel")}
          </label>
        )}
      </div>
      {leadModel && <ModelPicker value={leadModel} onChange={setLeadModel} caps={caps} />}
      {plannerModel && <ModelPicker value={plannerModel} onChange={setPlannerModel} caps={caps} />}
      <button
        className="btn-primary"
        disabled={!model.provider || !project}
        onClick={() =>
          api
            .createTeam(project, tpl, model, leadModel ?? undefined, plannerModel ?? undefined)
            .then(() => (app.notify(app.t("common.done")), app.refresh()))
            .catch((e) => app.notify(String(e)))
        }
      >
        <UserPlus className="h-3.5 w-3.5" /> {app.t("common.create")}
      </button>
    </div>
  );
}

export function TeamsView() {
  const app = useApp();
  const [data, setData] = useState<TeamsData | null>(null);
  const [mode, setMode] = useState<"none" | "compose" | "template">("none");
  const [dissolve, setDissolve] = useState<string | null>(null);
  const [adding, setAdding] = useState<Record<string, string>>({});
  useEffect(() => {
    api.teams().then(setData);
  }, [app.tick]);
  const act = (p: Promise<unknown>) => p.then(() => (app.refresh(), app.notify(app.t("common.done")))).catch((e) => app.notify(String(e)));
  const plannerIds = useMemo(() => new Set((data?.teams ?? []).map((t) => t.planner).filter(Boolean) as string[]), [data]);
  if (!data) return <div className="text-xs text-subtle">{app.t("common.loading")}</div>;
  return (
    <div className="space-y-4">
      <PageHeader title={app.t("nav.teams")} sub={`${data.teams.length} team · ${data.pool.length} ${app.t("agents.pool")}`}>
        <button className={cn(mode === "compose" ? "btn-primary" : "btn")} onClick={() => setMode(mode === "compose" ? "none" : "compose")}>
          <Users className="h-3.5 w-3.5" /> {app.t("teams.compose")}
        </button>
        <button className={cn(mode === "template" ? "btn-primary" : "btn")} onClick={() => setMode(mode === "template" ? "none" : "template")}>
          <Plus className="h-3.5 w-3.5" /> {app.t("teams.fromTemplate")}
        </button>
      </PageHeader>
      {mode === "compose" && (
        <Section title={app.t("teams.compose")}>
          <TeamComposer data={data} onDone={() => setMode("none")} />
        </Section>
      )}
      {mode === "template" && (
        <Section title={app.t("teams.fromTemplate")}>
          <TemplateQuickStart />
        </Section>
      )}
      {data.teams.map(({ team, members, project_name }) => {
        const free = app.projects.filter((p) => !p.team || p.id === team.project);
        const sorted = [...members].sort((a, b) => Number(b.id === team.lead) - Number(a.id === team.lead) || Number(plannerIds.has(b.id)) - Number(plannerIds.has(a.id)));
        return (
          <Section
            key={team.id}
            title={
              <span className="flex items-center gap-2">
                {team.name}
                <RoleChip solo={team.solo} lead={!team.solo} />
              </span>
            }
            right={
              <div className="flex items-center gap-2">
                <select className="input h-7 w-52 py-0 text-[11px]" value={team.project ?? ""} onChange={(e) => act(api.setTeamProject(team.id, e.target.value || null))}>
                  <option value="">{app.t("teams.noProject")}</option>
                  {free.map((p) => (
                    <option key={p.id} value={p.id}>
                      {p.key_prefix} — {p.name}
                    </option>
                  ))}
                </select>
                <button className="btn" title={app.t("teams.dissolve")} onClick={() => setDissolve(team.id)}>
                  <Trash2 className="h-3.5 w-3.5" />
                </button>
              </div>
            }
          >
            <div className="grid gap-2 md:grid-cols-2 xl:grid-cols-4">
              {sorted.map((m) => {
                const isLead = team.lead === m.id;
                return (
                  <div key={m.id} className={cn("flex items-center gap-3 rounded-lg border bg-elevated/40 p-3", isLead ? "border-purple-500/40" : "border-line")}>
                    <button className="flex min-w-0 flex-1 items-center gap-3 text-left" onClick={() => app.go("agent", m.id)}>
                      <Avatar name={m.name} />
                      <div className="min-w-0 flex-1">
                        <div className="flex items-center gap-1.5">
                          {isLead && <Crown className="h-3.5 w-3.5 text-purple-500" />}
                          <span className="truncate text-xs font-semibold hover:text-accent">{m.name}</span>
                        </div>
                        <div className="truncate text-[10px] text-subtle">
                          {m.role}
                          {plannerIds.has(m.id) ? " · planner" : ""}
                        </div>
                      </div>
                    </button>
                    {!team.solo && !isLead && (
                      <div className="flex flex-col gap-1">
                        {!plannerIds.has(m.id) && (
                          <button title={app.t("agents.makeLead")} className="rounded p-1 text-subtle hover:bg-elevated hover:text-purple-500" onClick={() => act(api.setLeadOfTeam(team.id, m.id))}>
                            <Crown className="h-3.5 w-3.5" />
                          </button>
                        )}
                        <button title={app.t("teams.toPool")} className="rounded p-1 text-subtle hover:bg-elevated hover:text-rose-500" onClick={() => act(api.teamMember(team.id, m.id, false))}>
                          <UserMinus className="h-3.5 w-3.5" />
                        </button>
                      </div>
                    )}
                  </div>
                );
              })}
            </div>
            {!team.solo && data.pool.length > 0 && (
              <div className="mt-3 flex items-center gap-2">
                <select className="input h-8 w-64 text-xs" value={adding[team.id] ?? ""} onChange={(e) => setAdding((x) => ({ ...x, [team.id]: e.target.value }))}>
                  <option value="">{app.t("teams.addFromPool")}</option>
                  {data.pool.map((a) => (
                    <option key={a.id} value={a.id}>
                      {a.name} · {a.role}
                    </option>
                  ))}
                </select>
                <button className="btn" disabled={!adding[team.id]} onClick={() => act(api.teamMember(team.id, adding[team.id], true)).then(() => setAdding((x) => ({ ...x, [team.id]: "" })))}>
                  <UserPlus className="h-3.5 w-3.5" /> {app.t("teams.add")}
                </button>
              </div>
            )}
            <div className="mt-2 text-[10px] text-subtle">{project_name ? `${app.t("teams.worksOn")} ${project_name}` : app.t("teams.noProjectNote")}</div>
          </Section>
        );
      })}
      <Section title={`${app.t("agents.pool")} (${data.pool.length})`}>
        <div className="grid gap-2 md:grid-cols-2 xl:grid-cols-4">
          {data.pool.map((a) => (
            <Fragment key={a.id}>
              <button onClick={() => app.go("agent", a.id)} className="press flex items-center gap-3 rounded-lg border border-dashed border-line-strong p-3 text-left hover:border-accent/40">
                <Avatar name={a.name} />
                <div className="min-w-0 flex-1">
                  <div className="truncate text-xs font-semibold">{a.name}</div>
                  <div className="truncate text-[10px] text-subtle">{a.role}</div>
                </div>
                <AgentStatusBadge status={a.status} />
              </button>
            </Fragment>
          ))}
        </div>
        {data.pool.length === 0 && (
          <Empty>
            {app.t("teams.poolEmpty")}{" "}
            <button className="text-accent hover:underline" onClick={() => app.go("agents")}>
              {app.t("agents.create")}
            </button>
          </Empty>
        )}
      </Section>
      {data.teams.length === 0 && mode === "none" && <Empty>{app.t("teams.none")}</Empty>}
      <ConfirmModal
        open={!!dissolve}
        title={app.t("teams.dissolve")}
        note={app.t("teams.dissolveNote")}
        confirm={app.t("teams.dissolve")}
        onClose={() => setDissolve(null)}
        onConfirm={() => act(api.dissolveTeam(dissolve!)).finally(() => setDissolve(null))}
      />
    </div>
  );
}
