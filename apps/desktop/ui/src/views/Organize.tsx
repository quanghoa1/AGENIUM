import { ArrowLeft, Crown, FolderPlus, RotateCcw, Settings2, UserPlus, Users } from "lucide-react";
import { useEffect, useState } from "react";
import { PanelSwap, Segmented } from "../components/motion";
import { AgentStatusBadge, Avatar, Empty, Mono, OutcomeBadge, PageHeader, RoleChip, RuntimeBadge, Section, ago, cn, tokens } from "../components/ui";
import { api, type ModelArg } from "../lib/api";
import { useApp } from "../lib/store";
import type { AgentDetail, AgentEntry, Capabilities, TeamTemplate } from "../lib/types";

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
          <span className="label">{app.t("common.name")}</span>
          <input className="input mt-1" value={name} onChange={(e) => setName(e.target.value)} />
        </label>
        <label className="min-w-[280px] flex-1">
          <span className="label">{app.t("projects.folder")}</span>
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
            <div className="mt-1 truncate font-mono text-[11px] text-subtle">{p.path}</div>
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

export function AgentsView() {
  const app = useApp();
  const [rows, setRows] = useState<AgentEntry[]>([]);
  const [edit, setEdit] = useState<string | null>(null);
  useEffect(() => {
    api.agents().then(setRows);
  }, [app.tick]);
  return (
    <div className="space-y-4">
      <PageHeader title={app.t("nav.agents")} sub={`${rows.length}`} />
      <div className="grid gap-4 lg:grid-cols-2 2xl:grid-cols-3">
        {rows.map((r) => {
          const a = r.agent;
          return (
            <div key={a.id} className="card p-4">
              <button className="flex w-full items-start gap-3 text-left" onClick={() => app.go("agent", a.id)}>
                <Avatar name={a.name} className="h-10 w-10" />
                <div className="min-w-0 flex-1">
                  <div className="flex items-center gap-2">
                    <span className="truncate text-sm font-semibold hover:text-accent">{a.name}</span>
                    <RoleChip solo={r.solo} lead={r.is_lead} planner={r.is_planner} />
                  </div>
                  <div className="truncate text-[11px] text-subtle">
                    {a.role} · {r.team} · {r.project_name}
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
      {rows.length === 0 && <Empty>{app.t("common.none")}</Empty>}
    </div>
  );
}

export function AgentView({ id }: { id: string }) {
  const app = useApp();
  const [d, setD] = useState<AgentDetail | null>(null);
  const [entry, setEntry] = useState<AgentEntry | null>(null);
  const [tab, setTab] = useState<"overview" | "runs" | "memory" | "skills">("overview");
  const [dir, setDir] = useState(1);
  const order = ["overview", "runs", "memory", "skills"];
  useEffect(() => {
    api.agentDetail(id).then(setD).catch((e) => app.notify(String(e)));
    api.agents().then((xs) => setEntry(xs.find((x) => x.agent.id === id) ?? null));
  }, [id, app.tick]);
  if (!d) return <div className="text-xs text-subtle">{app.t("common.loading")}</div>;
  const a = d.agent;
  const canLead = entry && !entry.solo && !entry.is_lead && !d.planner;
  const mcp = Array.isArray(a.mcp) ? a.mcp : a.mcp && typeof a.mcp === "object" ? Object.keys(a.mcp).length ? [a.mcp] : [] : [];
  return (
    <div className="space-y-4">
      <button className="btn" onClick={() => app.go("agents")}>
        <ArrowLeft className="h-3.5 w-3.5" /> {app.t("nav.agents")}
      </button>
      <div className="card flex flex-wrap items-center gap-4 p-5">
        <Avatar name={a.name} className="h-12 w-12 text-sm" />
        <div className="min-w-0 flex-1">
          <div className="flex items-center gap-2">
            <span className="text-lg font-semibold">{a.name}</span>
            {entry && <RoleChip solo={entry.solo} lead={entry.is_lead} planner={d.planner} />}
            <AgentStatusBadge status={a.status} />
          </div>
          <div className="mt-0.5 text-xs text-subtle">
            {a.role} · {entry?.team} · {entry?.project_name}
          </div>
        </div>
        <RuntimeBadge backend={a.backend} model={a.model} />
        {canLead && (
          <button
            className="btn"
            onClick={() =>
              api
                .setTeamLead(entry!.project, a.id)
                .then(() => (app.notify(app.t("common.done")), app.refresh()))
                .catch((e) => app.notify(String(e)))
            }
          >
            <Crown className="h-3.5 w-3.5 text-purple-500" /> {app.t("agents.makeLead")}
          </button>
        )}
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
          <Section title="tags">
            <div className="flex flex-wrap gap-1">
              {a.tags.map((t) => (
                <span key={t} className="chip border-line-strong bg-elevated text-muted">
                  {t}
                </span>
              ))}
              {a.tags.length === 0 && <span className="text-xs text-subtle">–</span>}
            </div>
            {(a.status === "QuotaExhausted" || a.status === "Offline") && (
              <button className="btn mt-3" onClick={() => api.setAgentIdle(a.id).then(app.refresh)}>
                <RotateCcw className="h-3.5 w-3.5" /> {app.t("agents.toIdle")}
              </button>
            )}
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
        <div className="grid gap-4 md:grid-cols-2">
          <Section title="Skills">
            <div className="space-y-2">
              {d.skills.map((s) => (
                <div key={s.name} className="rounded-lg border border-line p-2.5">
                  <div className="flex items-center gap-2">
                    <span className="text-xs font-semibold">{s.name}</span>
                    {s.version && <Mono className="text-subtle">v{s.version}</Mono>}
                    <span className="chip ml-auto border-line-strong text-muted">{String(s.origin)}</span>
                  </div>
                  <div className="mt-1 text-[11px] text-subtle">{s.description}</div>
                </div>
              ))}
              {d.skills.length === 0 && <Empty>{app.t("agents.noSkills")}</Empty>}
            </div>
          </Section>
          <Section title="MCP">
            {mcp.length ? <pre className="whitespace-pre-wrap font-mono text-[11px] text-muted">{JSON.stringify(a.mcp, null, 2)}</pre> : <Empty>{app.t("agents.noMcp")}</Empty>}
          </Section>
        </div>
      )}
      </PanelSwap>
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
        <Section title={`${app.t("teams.newFor")} ${p.key_prefix} — ${p.name}`}>
          <div className="space-y-3">
            <Segmented
              id="team-kind"
              value={solo ? "solo" : "team"}
              onChange={(v) => setSolo(v === "solo")}
              itemClassName="flex-1 justify-center"
              items={[
                { value: "team", label: app.t("teams.fromTemplate") },
                { value: "solo", label: app.t("teams.solo") },
              ]}
            />
            {!solo && (
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
            )}
            {solo && <input className="input w-64" placeholder={app.t("teams.agentName")} value={soloName} onChange={(e) => setSoloName(e.target.value)} />}
            <div>
              <div className="label mb-1">{solo ? app.t("common.model") : app.t("teams.modelAll")}</div>
              <ModelPicker value={model} onChange={setModel} caps={caps} />
            </div>
            {!solo && (
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
            )}
            {leadModel && !solo && <ModelPicker value={leadModel} onChange={setLeadModel} caps={caps} />}
            {plannerModel && !solo && <ModelPicker value={plannerModel} onChange={setPlannerModel} caps={caps} />}
            <button
              className="btn-primary"
              disabled={!model.provider || (solo && !soloName.trim())}
              onClick={() =>
                (solo ? api.createSolo(p.id, soloName.trim(), model) : api.createTeam(p.id, tpl, model, leadModel ?? undefined, plannerModel ?? undefined))
                  .then(() => (app.notify(app.t("common.done")), app.refresh()))
                  .catch((e) => app.notify(String(e)))
              }
            >
              <UserPlus className="h-3.5 w-3.5" /> {app.t("common.create")}
            </button>
          </div>
        </Section>
      )}
      {[...byProject.entries()].map(([project, members]) => (
        <Section key={project} title={`${project} · ${members[0].team}`}>
          <div className="grid gap-2 md:grid-cols-2 xl:grid-cols-4">
            {[...members]
              .sort((a, b) => Number(b.is_lead) - Number(a.is_lead) || Number(b.is_planner) - Number(a.is_planner))
              .map((m) => (
                <button key={m.agent.id} onClick={() => app.go("agent", m.agent.id)} className="press flex items-center gap-3 rounded-lg border border-line bg-elevated/40 p-3 text-left hover:border-accent/40 hover:shadow-md">
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
      {p?.team && <div className="text-[11px] text-subtle">{app.t("teams.onePerProject")}</div>}
    </div>
  );
}
