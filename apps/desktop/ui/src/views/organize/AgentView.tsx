import { ArrowLeft, Check, Crown, Download, Pencil, Plug, RotateCcw, Trash2, X } from "lucide-react";
import { useEffect, useState } from "react";
import { Modal, PanelSwap, Segmented } from "../../components/motion";
import { AgentStatusBadge, Avatar, CacheHit, Empty, Mono, OutcomeBadge, RoleChip, RuntimeBadge, Section, ago, cn, tokens } from "../../components/ui";
import { api } from "../../lib/api";
import type { Key } from "../../lib/i18n";
import { friendly } from "../../lib/errors";
import { useApp } from "../../lib/store";
import type { AgentDetail, AgentEntry, AgentMcpKey, AgentRecord, RoleInfo } from "../../lib/types";
import { AgentModel } from "./ModelPicker";
import { RoleDraft, RolePicker, emptyRole, saveRoleIfNew } from "./roles";
import { Switches, useLibrary } from "./switches";

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
              <div className="mt-1 truncate font-mono text-[11px] text-subtle" title={r.system ?? ""}>
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
        {app.t("agents.saveRole")}
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
      .catch((e) => app.notify(friendly(e, app.t)));
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
      <button className="btn-primary px-2" title={app.t("agents.saveName")} aria-label={app.t("agents.saveName")} disabled={!draft.trim() || draft.trim() === name} onClick={save}>
        <Check className="h-3.5 w-3.5" />
      </button>
      <button className="btn px-2" title={app.t("common.cancel")} aria-label={app.t("common.cancel")} onClick={() => setDraft(null)}>
        <X className="h-3.5 w-3.5" />
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
              {d.cache && (
                <div className="mt-3 flex items-center gap-2 text-[11px] text-subtle">
                  <CacheHit value={d.cache.hit} title={app.t("agents.cacheNote")} />
                  <span>
                    {app.t("agents.cacheOver")} {d.cache.runs} API run · {tokens(d.cache.cached)} / {tokens(d.cache.input)} tok
                  </span>
                </div>
              )}
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
                <div className="mt-3 rounded-lg border border-amber-500/40 bg-amber-500/5 p-2.5">
                  <button className="btn-primary" onClick={() => api.setAgentIdle(a.id).then(app.refresh)}>
                    <RotateCcw className="h-3.5 w-3.5" /> {app.t("agents.resume")}
                  </button>
                  <div className="mt-1.5 text-[11px] text-subtle">{app.t("agents.resumeHint")}</div>
                </div>
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
                <span className="ml-auto">
                  <CacheHit value={r.cache_hit} />
                </span>
                <Mono className="text-subtle">
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
                <div className="mt-1 text-[11px] text-subtle">
                  {ago(f.created_at)}
                  {f.used_at ? ` · ${app.t("memory.used")} ${ago(f.used_at)}` : ""}
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
                      <SkillOrigin origin={String(s.origin)} />
                    </div>
                    {!!s.tags?.length && (
                      <div className="mt-1 flex flex-wrap gap-1">
                        {s.tags.map((t) => (
                          <span key={t} className="chip border-indigo-500/30 text-indigo-600 dark:text-indigo-300">
                            {t}
                          </span>
                        ))}
                      </div>
                    )}
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

const ORIGIN: Record<string, [Key, Key, string]> = {
  overlay: ["skills.origin.overlay", "skills.origin.overlayHint", "border-amber-500/40 bg-amber-500/10 text-amber-600 dark:text-amber-300"],
  agent: ["skills.origin.agent", "skills.origin.agentHint", "border-violet-500/40 bg-violet-500/10 text-violet-600 dark:text-violet-300"],
  role: ["skills.origin.role", "skills.origin.roleHint", "border-line-strong text-muted"],
  library: ["skills.origin.library", "skills.origin.libraryHint", "border-line-strong text-muted"],
};

/** Where an agent's skill comes from: its own updated copy of a system skill (the system's stays as it is), one it
 *  learned, its role's, or the system library's. */
function SkillOrigin({ origin }: { origin: string }) {
  const app = useApp();
  const o = ORIGIN[origin.toLowerCase()];
  if (!o) return <span className="chip ml-auto border-line-strong text-muted">{origin}</span>;
  return (
    <span className={cn("chip ml-auto", o[2])} title={app.t(o[1])}>
      {app.t(o[0])}
    </span>
  );
}

