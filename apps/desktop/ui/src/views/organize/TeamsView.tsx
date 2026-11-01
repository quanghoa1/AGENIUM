import { Crown, Lightbulb, Plus, Sparkles, Trash2, UserMinus, UserPlus, Users, X } from "lucide-react";
import { Fragment, useEffect, useMemo, useState } from "react";
import { ConfirmModal, Segmented } from "../../components/motion";
import { AgentStatusBadge, Avatar, Empty, PageHeader, ProjectStateChips, RoleChip, Section, cn, modelLabel } from "../../components/ui";
import { api, type ModelArg } from "../../lib/api";
import { friendly } from "../../lib/errors";
import type { Key } from "../../lib/i18n";
import { useApp } from "../../lib/store";
import type { AgentRecord, Capabilities, LeadSuggestion, RoleInfo, TeamTemplate, TeamsData } from "../../lib/types";
import { remember, rememberedModel } from "../../lib/remembered";
import { ModelPicker, emptyModel } from "./ModelPicker";

/** Compose a team from pool agents: pick members, the Lead (the system suggests one, with its reasons), a project. */
/** Why an agent is suggested as Lead, in the interface language (the server says `role X is meant to lead`, `tag Y`). */
function leadReason(r: string, t: (k: Key) => string): string {
  const m = r.match(/^role (.+) is meant to lead$/) ?? r.match(/^(role|tag|skill) (.+)$/);
  if (!m) return r;
  if (m.length === 2) return t("teams.why.leadRole").replace("{x}", m[1]);
  return t(`teams.why.${m[1]}` as Key).replace("{x}", m[2]);
}

function TeamComposer({ data, onDone, onCancel }: { data: TeamsData; onDone: () => void; onCancel: () => void }) {
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
  const freeProjects = app.projects.filter((p) => !p.team && p.run_state !== "Closed");
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
      .catch((e) => app.notify(friendly(e, app.t)));
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
                {p.name}
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
                  <div className="truncate text-[11px] text-subtle">
                    {a.role} · {modelLabel(a.backend, a.model, app.t("model.default"))}
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
            {app.t("teams.suggested")} <b>{byId.get(suggestion.agent)?.name}</b> — {suggestion.reasons.map((r) => leadReason(r, app.t)).join(", ")}
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
      <div className="flex gap-2">
        <button className="btn-primary" disabled={!ok} onClick={create}>
          <Users className="h-3.5 w-3.5" /> {app.t("teams.compose")}
        </button>
        <button className="btn" onClick={onCancel}>
          {app.t("common.cancel")}
        </button>
      </div>
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
  const [gone, setGone] = useState<string | null>(null);
  // Each role on the model its template names (ADR-152), where this machine runs it; kept with the models.
  const [roleModels, setRoleModels] = useState(true);
  useEffect(() => {
    api.templates().then((t) => setTemplates(t ?? []));
    Promise.all([api.capabilities(), api.preference("model.team").catch(() => null)]).then(([c, kept]) => {
      setCaps(c);
      // The models picked last for a team (ADR-152), each when it can still run here.
      const k = (kept ?? {}) as { model?: unknown; lead?: unknown; planner?: unknown; roleModels?: boolean };
      if (typeof k.roleModels === "boolean") setRoleModels(k.roleModels);
      const m = rememberedModel(k.model, c);
      setGone(m.gone);
      if (m.model) setModel(m.model);
      else if (c?.providers[0]) setModel((x) => ({ ...x, provider: x.provider || c.providers[0].id }));
      const lead = rememberedModel(k.lead, c).model;
      if (lead) setLeadModel(lead);
      const planner = rememberedModel(k.planner, c).model;
      if (planner) setPlannerModel(planner);
    });
  }, []);
  const t = templates.find((x) => x.id === tpl);
  const named = (t?.roles ?? []).filter((r) => r.default_model);
  const free = app.projects.filter((p) => !p.team && p.run_state !== "Closed");
  return (
    <div className="space-y-3">
      <div className="text-[11px] text-subtle">{app.t("teams.templateNote")}</div>
      <select className="input w-64" value={project} onChange={(e) => setProject(e.target.value)}>
        <option value="">{app.t("teams.pickProject")}</option>
        {free.map((p) => (
          <option key={p.id} value={p.id}>
            {p.name}
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
        {gone && <div className="mt-1 text-[11px] text-amber-600 dark:text-amber-400">{app.t("models.keptGone").replace("{m}", gone)}</div>}
      </div>
      {named.length > 0 && (
        <label className="flex items-start gap-1.5 text-xs">
          <input type="checkbox" className="mt-0.5" checked={roleModels} onChange={(e) => setRoleModels(e.target.checked)} />
          <span>
            {app.t("teams.roleModels")}
            <span className="block font-mono text-[11px] text-subtle">{named.map((r) => `${r.id}: ${r.default_model}`).join(" · ")}</span>
          </span>
        </label>
      )}
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
            .createTeam(project, tpl, model, leadModel ?? undefined, plannerModel ?? undefined, undefined, roleModels && named.length > 0)
            .then((r) => {
              remember("model.team", { model, lead: leadModel, planner: plannerModel, roleModels });
              const gone = (r.model_notes ?? []).filter((n) => n.includes("not usable"));
              app.notify(gone.length ? `${app.t("common.done")} · ${gone.join("; ")}` : app.t("common.done"));
              app.refresh();
            })
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
      <PageHeader title={app.t("nav.teams")} sub={`${data.teams.length} ${app.t(data.teams.length === 1 ? "teams.one" : "teams.many")} · ${data.pool.length} ${app.t("agents.pool")}`}>
        {mode === "none" ? (
          <>
            <button className="btn" onClick={() => setMode("compose")}>
              <Users className="h-3.5 w-3.5" /> {app.t("teams.compose")}
            </button>
            <button className="btn" onClick={() => setMode("template")}>
              <Plus className="h-3.5 w-3.5" /> {app.t("teams.fromTemplate")}
            </button>
            <button className="btn" onClick={() => app.go("templates", "design")} title={app.t("design.teamsHint")}>
              <Sparkles className="h-3.5 w-3.5" /> {app.t("design.button")}
            </button>
          </>
        ) : (
          <button className="btn" onClick={() => setMode("none")}>
            <X className="h-3.5 w-3.5" /> {app.t("common.close")}
          </button>
        )}
      </PageHeader>
      {mode === "compose" && (
        <Section title={app.t("teams.compose")}>
          <TeamComposer data={data} onDone={() => setMode("none")} onCancel={() => setMode("none")} />
        </Section>
      )}
      {mode === "template" && (
        <Section title={app.t("teams.fromTemplate")}>
          <TemplateQuickStart />
        </Section>
      )}
      {data.teams.map(({ team, members, project_name, past_projects }) => {
        // A closed project is reopened first (its card, the header): only then does it take a team.
        const free = app.projects.filter((p) => (!p.team && p.run_state !== "Closed") || p.id === team.project);
        // A team works one project at a time: it moves on once its project has no open ticket left.
        const current = app.projects.find((p) => p.id === team.project);
        const open = current ? Object.entries(current.by_status ?? {}).filter(([s]) => s !== "Closed" && s !== "Cancelled").reduce((n, [, c]) => n + c, 0) : 0;
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
                <select
                  className="input h-7 w-52 py-0 text-[11px]"
                  value={team.project ?? ""}
                  disabled={open > 0}
                  title={open > 0 ? app.t("teams.busyHint").replace("{p}", current?.name ?? "").replace("{n}", String(open)) : undefined}
                  onChange={(e) => act(api.setTeamProject(team.id, e.target.value || null))}
                >
                  <option value="">{app.t("teams.noProject")}</option>
                  {free.map((p) => (
                    <option key={p.id} value={p.id}>
                      {p.name}
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
                        <div className="truncate text-[11px] text-subtle">
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
            <div className="mt-3 flex flex-wrap items-center gap-x-3 gap-y-1.5 text-[11px] text-subtle">
              {current ? (
                <span className="flex items-center gap-2">
                  {app.t("teams.worksOn")} <b className="text-fg">{project_name}</b>
                  <ProjectStateChips state={current.run_state} needsYou={current.needs_you} cancelled={current.status === "Cancelled"} />
                  {open > 0 && <span>· {app.t("teams.openTickets").replace("{n}", String(open))}</span>}
                </span>
              ) : (
                <span>{app.t("teams.noProjectNote")}</span>
              )}
              {!!past_projects?.length && (
                <span>
                  · {app.t("teams.workedOn")} {past_projects.join(", ")}
                </span>
              )}
            </div>
          </Section>
        );
      })}
      <Section title={`${app.t("agents.poolChip")} (${data.pool.length})`}>
        <div className="grid gap-2 md:grid-cols-2 xl:grid-cols-4">
          {data.pool.map((a) => (
            <Fragment key={a.id}>
              <button onClick={() => app.go("agent", a.id)} className="press flex items-center gap-3 rounded-lg border border-dashed border-line-strong p-3 text-left hover:border-accent/40">
                <Avatar name={a.name} />
                <div className="min-w-0 flex-1">
                  <div className="truncate text-xs font-semibold">{a.name}</div>
                  <div className="truncate text-[11px] text-subtle">{a.role}</div>
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
