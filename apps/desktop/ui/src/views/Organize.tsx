import { FolderPlus, Plus, RotateCcw, Settings2, UserPlus, Users } from "lucide-react";
import { useEffect, useState } from "react";
import { AgentStatusBadge, Avatar, Empty, Mono, PageHeader, RoleChip, RuntimeBadge, Section, cn } from "../components/ui";
import { api, type ModelArg } from "../lib/api";
import { useApp } from "../lib/store";
import type { AgentEntry, Capabilities, TeamTemplate } from "../lib/types";

/** Provider (API) or CLI program, model and effort. */
export function ModelPicker({ value, onChange, caps }: { value: ModelArg; onChange: (m: ModelArg) => void; caps: Capabilities | null }) {
  const cli = value.backend === "cli";
  return (
    <div className="grid grid-cols-4 gap-2">
      <select className="input" value={value.backend ?? "api"} onChange={(e) => onChange({ ...value, backend: e.target.value as "api" | "cli", provider: "" })}>
        <option value="api">API</option>
        <option value="cli">CLI</option>
      </select>
      <select className="input" value={value.provider} onChange={(e) => onChange({ ...value, provider: e.target.value })}>
        <option value="">{cli ? "CLI…" : "provider…"}</option>
        {cli
          ? caps?.clis.filter((c) => c.compatible).map((c) => (
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
      <input className="input" placeholder="model" value={value.model} onChange={(e) => onChange({ ...value, model: e.target.value })} />
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
  const [path, setPath] = useState("");
  return (
    <div className="space-y-4">
      <PageHeader title={app.t("nav.projects")} />
      <div className="card flex flex-wrap items-end gap-2 p-4">
        <label className="w-48">
          <span className="label">name</span>
          <input className="input mt-1" value={name} onChange={(e) => setName(e.target.value)} />
        </label>
        <label className="min-w-[280px] flex-1">
          <span className="label">folder (optional: an existing repository)</span>
          <input className="input mt-1" value={path} placeholder="<root>/projects/<name>" onChange={(e) => setPath(e.target.value)} />
        </label>
        <button
          className="btn-primary"
          disabled={!name.trim()}
          onClick={() =>
            api
              .createProject(name.trim(), path.trim() || undefined)
              .then((p) => (app.setProject(p.id), setName(""), setPath(""), app.refresh(), app.go("teams")))
              .catch((e) => app.notify(String(e)))
          }
        >
          <FolderPlus className="h-3.5 w-3.5" /> Create project
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
            <div className="mt-1 truncate font-mono text-[11px] text-subtle">{p.path}</div>
            <div className="mt-3 grid grid-cols-4 gap-2 text-center text-xs">
              {[
                ["team", p.team ? (p.team.solo ? "solo" : p.team.members) : "–"],
                ["tickets", p.tickets],
                ["plan", `${p.plan_done}/${p.plan_items}`],
                ["needs you", p.needs_you],
              ].map(([k, v]) => (
                <div key={k as string} className="rounded-lg border border-line bg-elevated/50 p-2">
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
                  <Users className="h-3.5 w-3.5" /> Add a team
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

export function AgentsView() {
  const app = useApp();
  const [rows, setRows] = useState<AgentEntry[]>([]);
  const [caps, setCaps] = useState<Capabilities | null>(null);
  const [edit, setEdit] = useState<string | null>(null);
  const [model, setModel] = useState<ModelArg>(emptyModel);
  useEffect(() => {
    api.agents().then(setRows);
  }, [app.tick]);
  useEffect(() => {
    api.capabilities().then(setCaps);
  }, []);
  return (
    <div className="space-y-4">
      <PageHeader title={app.t("nav.agents")} sub={`${rows.length}`} />
      <div className="grid gap-4 lg:grid-cols-2 2xl:grid-cols-3">
        {rows.map((r) => {
          const a = r.agent;
          return (
            <div key={a.id} className={cn("card p-4", app.route.id === a.id && "border-accent")}>
              <div className="flex items-start gap-3">
                <Avatar name={a.name} className="h-10 w-10" />
                <div className="min-w-0 flex-1">
                  <div className="flex items-center gap-2">
                    <span className="truncate text-sm font-semibold">{a.name}</span>
                    <RoleChip solo={r.solo} lead={r.is_lead} planner={r.is_planner} />
                  </div>
                  <div className="truncate text-[11px] text-subtle">
                    {a.role} · {r.team} · {r.project_name}
                  </div>
                </div>
                <AgentStatusBadge status={a.status} />
              </div>
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
                <RuntimeBadge backend={a.backend} model={a.model} />
                <div className="flex gap-1.5">
                  {(a.status === "QuotaExhausted" || a.status === "Offline") && (
                    <button className="btn" title="Back to Idle (after changing the model or adding credit)" onClick={() => api.setAgentIdle(a.id).then(app.refresh)}>
                      <RotateCcw className="h-3.5 w-3.5" />
                    </button>
                  )}
                  <button
                    className="btn"
                    onClick={() => {
                      setEdit(edit === a.id ? null : a.id);
                      setModel({ provider: a.model.provider, model: a.model.model, effort: a.model.effort, backend: a.backend === "Cli" ? "cli" : "api" });
                    }}
                  >
                    <Settings2 className="h-3.5 w-3.5" /> Model
                  </button>
                </div>
              </div>
              {edit === a.id && (
                <div className="mt-3 space-y-2 rounded-lg border border-line bg-elevated/50 p-3">
                  <ModelPicker value={model} onChange={setModel} caps={caps} />
                  <div className="text-[11px] text-subtle">Memory, skills and MCP stay with the agent when the model changes.</div>
                  <button
                    className="btn-primary"
                    disabled={!model.provider}
                    onClick={() =>
                      api
                        .setAgentModel(a.id, model)
                        .then(() => (setEdit(null), app.refresh(), app.notify("✓")))
                        .catch((e) => app.notify(String(e)))
                    }
                  >
                    {app.t("common.save")}
                  </button>
                </div>
              )}
            </div>
          );
        })}
      </div>
      {rows.length === 0 && <Empty>{app.t("common.none")}</Empty>}
    </div>
  );
}

export function TeamsView() {
  const app = useApp();
  const [rows, setRows] = useState<AgentEntry[]>([]);
  const [templates, setTemplates] = useState<TeamTemplate[]>([]);
  const [caps, setCaps] = useState<Capabilities | null>(null);
  const [tpl, setTpl] = useState("coding");
  const [model, setModel] = useState<ModelArg>(emptyModel);
  const [leadModel, setLeadModel] = useState<ModelArg | null>(null);
  const [plannerModel, setPlannerModel] = useState<ModelArg | null>(null);
  const [solo, setSolo] = useState(false);
  const [soloName, setSoloName] = useState("");
  useEffect(() => {
    api.agents().then(setRows);
  }, [app.tick]);
  useEffect(() => {
    api.templates().then(setTemplates);
    api.capabilities().then((c) => {
      setCaps(c);
      if (c.providers[0]) setModel((m) => ({ ...m, provider: m.provider || c.providers[0].id }));
    });
  }, []);
  const p = app.projects.find((x) => x.id === app.project);
  const byProject = new Map<string, AgentEntry[]>();
  rows.forEach((r) => byProject.set(r.project_name, [...(byProject.get(r.project_name) ?? []), r]));
  const t = templates.find((x) => x.id === tpl);
  return (
    <div className="space-y-4">
      <PageHeader title={app.t("nav.teams")} />
      {p && !p.team && (
        <Section title={`New team for ${p.key_prefix} — ${p.name}`}>
          <div className="space-y-3">
            <div className="flex rounded-lg border border-line-strong bg-elevated p-0.5 text-xs">
              <button className={cn("flex-1 rounded-md py-1", !solo && "bg-surface font-medium shadow-sm")} onClick={() => setSolo(false)}>
                Team from a template
              </button>
              <button className={cn("flex-1 rounded-md py-1", solo && "bg-surface font-medium shadow-sm")} onClick={() => setSolo(true)}>
                Solo agent
              </button>
            </div>
            {!solo && (
              <div className="grid gap-2 md:grid-cols-4">
                {templates.map((x) => (
                  <button key={x.id} onClick={() => setTpl(x.id)} className={cn("rounded-lg border p-3 text-left", tpl === x.id ? "border-accent bg-blue-600/10" : "border-line hover:border-line-strong")}>
                    <div className="text-xs font-semibold">{x.name}</div>
                    <div className="mt-1 text-[11px] text-subtle">{x.description}</div>
                    <div className="mt-2 flex flex-wrap gap-1">
                      {x.roles.map((r) => (
                        <span key={r.id} className="chip border-line-strong bg-elevated text-muted">
                          {r.id}
                          {r.count > 1 ? ` ×${r.count}` : ""}
                        </span>
                      ))}
                    </div>
                  </button>
                ))}
              </div>
            )}
            {solo && <input className="input w-64" placeholder="agent name" value={soloName} onChange={(e) => setSoloName(e.target.value)} />}
            <div>
              <div className="label mb-1">{solo ? "model" : "model of every agent"}</div>
              <ModelPicker value={model} onChange={setModel} caps={caps} />
            </div>
            {!solo && (
              <div className="flex flex-wrap gap-3 text-xs">
                <label className="flex items-center gap-1.5">
                  <input type="checkbox" checked={!!leadModel} onChange={(e) => setLeadModel(e.target.checked ? { ...model } : null)} /> different model for the Lead
                </label>
                {t?.roles.some((r) => r.planner) && (
                  <label className="flex items-center gap-1.5">
                    <input type="checkbox" checked={!!plannerModel} onChange={(e) => setPlannerModel(e.target.checked ? { ...model } : null)} /> different model for the planner
                  </label>
                )}
              </div>
            )}
            {leadModel && !solo && <ModelPicker value={leadModel} onChange={setLeadModel} caps={caps} />}
            {plannerModel && !solo && <ModelPicker value={plannerModel} onChange={setPlannerModel} caps={caps} />}
            <button
              className="btn-primary"
              disabled={!model.provider || (solo && !soloName.trim())}
              onClick={() =>
                (solo ? api.createSolo(p.id, soloName.trim(), model) : api.createTeam(p.id, tpl, model, leadModel ?? undefined, plannerModel ?? undefined))
                  .then(() => (app.notify("✓"), app.refresh()))
                  .catch((e) => app.notify(String(e)))
              }
            >
              <UserPlus className="h-3.5 w-3.5" /> Create
            </button>
          </div>
        </Section>
      )}
      {[...byProject.entries()].map(([project, members]) => (
        <Section key={project} title={`${project} · ${members[0].team}${members[0].solo ? " (solo)" : ""}`}>
          <div className="grid gap-2 md:grid-cols-2 xl:grid-cols-4">
            {[...members]
              .sort((a, b) => Number(b.is_lead) - Number(a.is_lead) || Number(b.is_planner) - Number(a.is_planner))
              .map((m) => (
                <button key={m.agent.id} onClick={() => app.go("agents", m.agent.id)} className="flex items-center gap-3 rounded-lg border border-line bg-elevated/40 p-3 text-left hover:border-accent/40">
                  <Avatar name={m.agent.name} />
                  <div className="min-w-0 flex-1">
                    <div className="flex items-center gap-1.5">
                      <span className="truncate text-xs font-semibold">{m.agent.name}</span>
                      <RoleChip solo={m.solo} lead={m.is_lead} planner={m.is_planner} />
                    </div>
                    <div className="truncate text-[11px] text-subtle">{m.agent.role}</div>
                  </div>
                  <AgentStatusBadge status={m.agent.status} />
                </button>
              ))}
          </div>
        </Section>
      ))}
      {rows.length === 0 && !p && <Empty>{app.t("common.none")}</Empty>}
      {p?.team && (
        <div className="text-[11px] text-subtle">
          <Plus className="mr-1 inline h-3 w-3" />
          One team per project: create another project for another team.
        </div>
      )}
    </div>
  );
}
