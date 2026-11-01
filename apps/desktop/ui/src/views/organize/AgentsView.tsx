import { FileUp, Plug, RotateCcw, Settings2, Sparkles, UserPlus, Wrench } from "lucide-react";
import { useEffect, useState } from "react";
import { Modal, Overlay, Segmented } from "../../components/motion";
import { AgentStatusBadge, Avatar, Empty, PageHeader, RoleChip, RuntimeBadge, cn } from "../../components/ui";
import { SearchBox, found } from "../../components/SearchBox";
import { api, type ModelArg } from "../../lib/api";
import { friendly } from "../../lib/errors";
import { useFileDrop } from "../../lib/drops";
import { useApp } from "../../lib/store";
import type { AgentEntry, Capabilities, RoleInfo } from "../../lib/types";
import { remember, rememberedModel } from "../../lib/remembered";
import { AgentModel, ModelPicker, emptyModel } from "./ModelPicker";
import { NEW_ROLE, RoleDraft, RolePicker, draftLint, emptyRole, saveRoleIfNew } from "./roles";
import { Switches, useLibrary } from "./switches";

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
  // The model picked last here, when it is gone (ADR-152): the form says so instead of opening with it.
  const [gone, setGone] = useState<string | null>(null);
  const [skills, setSkills] = useState<string[]>([]);
  const [mcp, setMcp] = useState<string[]>([]);
  const [busy, setBusy] = useState(false);
  useEffect(() => {
    if (!open) return;
    // Each opening starts clean: what was typed before a Cancel does not come back.
    setName("");
    setRole("");
    setDraft(emptyRole);
    setSkills([]);
    setMcp([]);
    setModel(emptyModel);
    api.roles().then((r) => setRoles(r ?? []));
    Promise.all([api.capabilities(), api.preference("model.agent").catch(() => null)]).then(([c, kept]) => {
      setCaps(c);
      // The model picked last for an agent (ADR-152), when it can still run here.
      const r = rememberedModel(kept, c);
      setGone(r.gone);
      if (r.model) return setModel(r.model);
      // An API provider with a key first; with none, a CLI this computer can already run.
      const keyed = c?.providers.find((p) => p.has_key);
      const cli = c?.clis.find((x) => x.compatible && x.login === "logged_in") ?? c?.clis.find((x) => x.compatible && x.login !== "not_logged_in");
      if (keyed) setModel({ ...emptyModel, provider: keyed.id });
      else if (cli) setModel({ ...emptyModel, backend: "cli", provider: cli.kind.toLowerCase() });
      else if (c?.providers[0]) setModel({ ...emptyModel, provider: c.providers[0].id });
    });
  }, [open]);
  const flip = (xs: string[], x: string, on: boolean) => (on ? [...new Set([...xs, x])] : xs.filter((y) => y !== x));
  const ready = name.trim() && role && (role !== NEW_ROLE || (draft.id && !draftLint(draft.prompt).includes("## Mission"))) && model.provider && (model.backend === "cli" || model.model);
  const create = async () => {
    setBusy(true);
    try {
      const r = await saveRoleIfNew(role, draft);
      const a = await api.createAgent({ name: name.trim(), role: r, model, skills, mcp });
      remember("model.agent", model);
      app.notify(`${a.name}: ${app.t("agents.inPool")}`);
      app.refresh();
      onClose();
    } catch (e) {
      app.notify(friendly(e, app.t));
    } finally {
      setBusy(false);
    }
  };
  return (
    <Overlay open={open} onClose={onClose} kind="dialog" top="pt-10">
      <div className="card max-h-[86vh] w-[760px] max-w-[94vw] overflow-y-auto p-5 pb-0 shadow-2xl">
        <div className="mb-4 flex items-center gap-2 text-sm font-semibold">
          <UserPlus className="h-4 w-4 text-accent" /> {app.t("agents.create")}
        </div>
        <div className="space-y-4">
          <label className="block">
            <span className="label">{app.t("common.name")}</span>
            <input className="input mt-1" value={name} placeholder="recon-worker" onChange={(e) => setName(e.target.value)} autoFocus />
          </label>
          <div>
            <div className="label mb-1">{app.t("agents.role")}</div>
            <RolePicker roles={roles} value={role} onChange={setRole} draft={draft} setDraft={setDraft} />
          </div>
          <div>
            <div className="label mb-1">{app.t("common.model")}</div>
            <ModelPicker value={model} onChange={setModel} caps={caps} />
            {gone && <div className="mt-1 text-[11px] text-amber-600 dark:text-amber-400">{app.t("models.keptGone").replace("{m}", gone)}</div>}
          </div>
          <Switches skills={skills} mcp={mcp} lib={lib} servers={servers} onSkills={(s, on) => setSkills((xs) => flip(xs, s, on))} onMcp={(s, on) => setMcp((xs) => flip(xs, s, on))} />
          <div className="text-[11px] text-subtle">{app.t("agents.poolNote")}</div>
          {/* Always in view, however long the form grows. */}
          <div className="sticky bottom-0 -mx-5 flex justify-end gap-2 border-t border-line bg-surface px-5 py-3">
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
  // A file dropped on the window while the dialog is open.
  useFileDrop((e) => e.type === "drop" && e.paths[0] && setFile(e.paths[0]), open);
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
              {p.name}
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
  const [query, setQuery] = useState("");
  const shown = rows.filter(
    (r) => (filter === "all" || (filter === "pool" ? r.pool : !r.pool)) && found(query, r.agent.name, r.agent.role, r.agent.model.model, r.agent.model.provider, r.team, r.project_name),
  );
  return (
    <div className="space-y-4">
      <PageHeader title={app.t("nav.agents")} sub={`${rows.length} · ${rows.filter((r) => r.pool).length} ${app.t("agents.pool")}`}>
        <SearchBox value={query} onChange={setQuery} placeholder={app.t("search.agents")} />
        <Segmented
          id="agent-filter"
          value={filter}
          onChange={setFilter}
          items={[
            { value: "all", label: app.t("agents.all") },
            { value: "team", label: app.t("agents.inTeam") },
            { value: "pool", label: app.t("agents.poolChip") },
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
                    {r.pool ? <span className="chip border-dashed border-line-strong text-subtle">{app.t("agents.poolChip").toUpperCase()}</span> : <RoleChip solo={r.solo} lead={r.is_lead} planner={r.is_planner} />}
                  </div>
                  <div className="truncate text-[11px] text-subtle">
                    {a.role}
                    {r.team ? ` · ${r.team}` : ""}
                    {r.project_name ? ` · ${r.project_name}` : ""}
                  </div>
                </div>
                <AgentStatusBadge status={a.status} />
              </button>
              {a.tags.length > 0 && (
                <div className="mt-3 flex flex-wrap gap-1">
                  {a.tags.map((t) => (
                    <span key={t} className="chip border-line-strong bg-elevated text-muted">
                      {t}
                    </span>
                  ))}
                </div>
              )}
              <div className="mt-3 flex items-center justify-between gap-2">
                <div className="flex min-w-0 flex-wrap items-center gap-1.5">
                  <RuntimeBadge backend={a.backend} model={a.model} />
                  {/* How much the agent works with, at a glance (the names are on its page). */}
                  {(
                    [
                      ["agents.skillCount", r.skill_count ?? a.skills.length, "border-cyan-500/30 text-cyan-600 dark:text-cyan-300", Wrench],
                      ["agents.learnedCount", r.learned_skills ?? 0, "border-emerald-500/30 text-emerald-600 dark:text-emerald-300", Sparkles],
                      ["agents.mcpCount", r.mcp_count ?? a.mcp?.length ?? 0, "border-purple-500/30 text-purple-600 dark:text-purple-300", Plug],
                    ] as const
                  ).map(([k, n, tone, Icon]) => (
                    <span key={k} className={cn("chip font-mono", n ? tone : "border-line text-subtle")}>
                      <Icon className="h-2.5 w-2.5" /> {app.t(k)}: {n}
                    </span>
                  ))}
                  {/* The fields the agent's skills cover (their tags). */}
                  {(r.skill_domains ?? []).map((t) => (
                    <span key={t} className="chip border-indigo-500/30 text-indigo-600 dark:text-indigo-300">
                      {t}
                    </span>
                  ))}
                </div>
                <div className="flex gap-1.5">
                  {(a.status === "QuotaExhausted" || a.status === "Offline") && (
                    <button className="btn-primary" title={app.t("agents.resumeHint")} onClick={() => api.setAgentIdle(a.id).then(app.refresh)}>
                      <RotateCcw className="h-3.5 w-3.5" /> {app.t("agents.resume")}
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
