// Team templates and their wizard (M4): the built-in templates, the user's own (saved under the root, a copy of a
// built-in with the same id overrides it), a template made from a team as it is, import and export as TOML.
import { Download, Pencil, Plus, Trash2, Upload, Users, X } from "lucide-react";
import { useEffect, useState } from "react";
import { Modal } from "../components/motion";
import { PageHeader, Section, cn } from "../components/ui";
import { api } from "../lib/api";
import { useApp } from "../lib/store";
import type { RoleInfo, RoleSpec, TeamTemplate, TeamsData } from "../lib/types";

const REPORTS = ["coding@1", "research@1", "pentest@1", "content@1"];
const LIBRARY = "library@1";
const SKELETON = "## Mission\n\n## Rules\n\n## Deliverables\n\n## Workflow\n\n## Done when\n";

const blankRole = (id: string, kind: RoleSpec["kind"]): RoleSpec => ({ id, kind, tags: [], prompt: SKELETON, skills: [], allowed_paths: [], default_model: null, count: 1, planner: false });

const blank = (): TeamTemplate => ({
  id: "",
  version: "1.0.0",
  name: "",
  description: "",
  report_template: "coding@1",
  roles: [blankRole("lead", "lead"), blankRole("worker", "worker")],
});

const list = (s: string) =>
  s
    .split(",")
    .map((x) => x.trim())
    .filter(Boolean);

export function TemplatesView() {
  const app = useApp();
  const [ts, setTs] = useState<TeamTemplate[]>([]);
  const [edit, setEdit] = useState<TeamTemplate | null>(null);
  const [fromTeam, setFromTeam] = useState(false);
  const [importing, setImporting] = useState(false);
  const load = () => api.templates().then(setTs);
  useEffect(() => {
    load();
  }, [app.tick]);
  const fail = (e: unknown) => app.notify(String(e));
  if (edit) return <TemplateEditor initial={edit} onDone={() => (setEdit(null), load())} />;
  return (
    <div className="space-y-4">
      <PageHeader title={app.t("nav.templates")} sub={app.t("templates.sub")}>
        <button className="btn" onClick={() => setImporting(true)}>
          <Upload className="h-3.5 w-3.5" /> {app.t("templates.import")}
        </button>
        <button className="btn" onClick={() => setFromTeam(true)}>
          <Users className="h-3.5 w-3.5" /> {app.t("templates.fromTeam")}
        </button>
        <button className="btn-primary" onClick={() => setEdit(blank())}>
          <Plus className="h-3.5 w-3.5" /> {app.t("templates.new")}
        </button>
      </PageHeader>
      {ts.map((t) => (
        <Section
          key={t.id}
          title={
            <span className="flex items-center gap-2">
              {t.name} · {t.id}@{t.version} · report {t.report_template}
              {t.custom ? (
                <span className="chip border-cyan-500/30 normal-case text-cyan-600 dark:text-cyan-300">{t.builtin ? app.t("templates.customCopy") : app.t("templates.custom")}</span>
              ) : (
                <span className="chip border-line-strong normal-case text-muted">{app.t("templates.builtin")}</span>
              )}
            </span>
          }
        >
          <div className="mb-3 flex items-start gap-3">
            <div className="min-w-0 flex-1 text-xs text-muted">{t.description}</div>
            <button className="btn" onClick={() => setEdit(structuredClone(t))}>
              <Pencil className="h-3.5 w-3.5" /> {app.t("templates.edit")}
            </button>
            <button
              className="btn"
              onClick={() =>
                api
                  .templateToml(t.id)
                  .then((toml) => navigator.clipboard.writeText(String(toml)))
                  .then(() => app.notify(app.t("templates.copied")))
                  .catch(fail)
              }
            >
              <Download className="h-3.5 w-3.5" /> TOML
            </button>
            {t.custom && (
              <button className="btn-danger" onClick={() => api.removeTemplate(t.id).then(load).catch(fail)}>
                <Trash2 className="h-3.5 w-3.5" /> {t.builtin ? app.t("templates.reset") : app.t("common.delete")}
              </button>
            )}
          </div>
          <div className="grid gap-2 md:grid-cols-2 xl:grid-cols-4">
            {t.roles.map((r) => (
              <div key={r.id} className="rounded-lg border border-line bg-elevated/40 p-3">
                <div className="flex items-center gap-2">
                  <span className="text-xs font-semibold">{r.id}</span>
                  <span className={cn("chip", r.kind === "lead" ? "border-purple-500/30 text-purple-600 dark:text-purple-300" : r.planner ? "border-cyan-500/30 text-cyan-600 dark:text-cyan-300" : "border-line-strong text-muted")}>
                    {r.planner ? "planner" : r.kind}
                    {r.count > 1 ? ` ×${r.count}` : ""}
                  </span>
                  {r.default_model && <span className="truncate font-mono text-[10px] text-subtle">{r.default_model}</span>}
                </div>
                <div className="mt-1.5 line-clamp-4 whitespace-pre-line text-[11px] text-subtle">{r.prompt}</div>
                {r.allowed_paths.length > 0 && <div className="mt-2 font-mono text-[10px] text-muted">scope: {r.allowed_paths.join(", ")}</div>}
              </div>
            ))}
          </div>
        </Section>
      ))}
      <FromTeam open={fromTeam} onClose={() => setFromTeam(false)} onDraft={(t) => (setFromTeam(false), setEdit(t))} />
      <ImportToml open={importing} onClose={() => setImporting(false)} onSaved={() => (setImporting(false), load())} />
    </div>
  );
}

function FromTeam({ open, onClose, onDraft }: { open: boolean; onClose: () => void; onDraft: (t: TeamTemplate) => void }) {
  const app = useApp();
  const [teams, setTeams] = useState<TeamsData["teams"]>([]);
  const [team, setTeam] = useState("");
  const [id, setId] = useState("");
  const [name, setName] = useState("");
  useEffect(() => {
    if (open) api.teams().then((d) => setTeams(d.teams.filter((t) => !t.team.solo)));
  }, [open]);
  return (
    <Modal open={open} title={app.t("templates.fromTeam")} onClose={onClose}>
      <div className="space-y-2">
        <select className="input" value={team} onChange={(e) => setTeam(e.target.value)}>
          <option value="">{app.t("templates.pickTeam")}</option>
          {teams.map((t) => (
            <option key={t.team.id} value={t.team.id}>
              {t.team.name} ({t.members.length})
            </option>
          ))}
        </select>
        <input className="input" placeholder="id (my-team)" value={id} onChange={(e) => setId(e.target.value.toLowerCase())} />
        <input className="input" placeholder={app.t("templates.name")} value={name} onChange={(e) => setName(e.target.value)} />
      </div>
      <div className="mt-4 flex justify-end gap-2">
        <button className="btn" onClick={onClose}>
          {app.t("common.cancel")}
        </button>
        <button
          className="btn-primary"
          disabled={!team || !id || !name}
          onClick={() =>
            api
              .templateFromTeam(team, id, name)
              .then(onDraft)
              .catch((e) => app.notify(String(e)))
          }
        >
          {app.t("templates.draft")}
        </button>
      </div>
    </Modal>
  );
}

function ImportToml({ open, onClose, onSaved }: { open: boolean; onClose: () => void; onSaved: () => void }) {
  const app = useApp();
  const [text, setText] = useState("");
  return (
    <Modal open={open} title={app.t("templates.import")} onClose={onClose}>
      <textarea className="input h-64 font-mono text-[11px]" placeholder={app.t("templates.paste")} value={text} onChange={(e) => setText(e.target.value)} />
      <div className="mt-4 flex justify-end gap-2">
        <button className="btn" onClick={onClose}>
          {app.t("common.cancel")}
        </button>
        <button
          className="btn-primary"
          disabled={!text.trim()}
          onClick={() =>
            api
              .templateToml(undefined, text)
              .then(() => (setText(""), app.notify(app.t("common.done")), onSaved()))
              .catch((e) => app.notify(String(e)))
          }
        >
          {app.t("common.save")}
        </button>
      </div>
    </Modal>
  );
}

/** Edits a template: identity, report template, and its roles (kind, count, tags, write scope, default model,
 *  ROLE.md, optionally started from a library or existing role). Saving validates on the backend. */
function TemplateEditor({ initial, onDone }: { initial: TeamTemplate; onDone: () => void }) {
  const app = useApp();
  const [t, setT] = useState<TeamTemplate>(initial);
  const [roles, setRoles] = useState<RoleInfo[]>([]);
  const [saving, setSaving] = useState(false);
  useEffect(() => {
    api.roles().then(setRoles);
  }, []);
  const role = (i: number, patch: Partial<RoleSpec>) => setT({ ...t, roles: t.roles.map((r, j) => (j === i ? { ...r, ...patch } : r)) });
  const kindOf = (r: RoleSpec) => (r.planner ? "planner" : r.kind);
  return (
    <div className="space-y-4">
      <PageHeader title={initial.id ? `${app.t("templates.edit")} ${initial.id}` : app.t("templates.new")} sub={app.t("templates.editorSub")}>
        <button className="btn" onClick={onDone}>
          <X className="h-3.5 w-3.5" /> {app.t("common.cancel")}
        </button>
        <button
          className="btn-primary"
          disabled={saving || !t.id || !t.name}
          onClick={() => {
            setSaving(true);
            api
              .saveTemplate(t)
              .then(() => (app.notify(app.t("common.done")), onDone()))
              .catch((e) => app.notify(String(e)))
              .finally(() => setSaving(false));
          }}
        >
          {app.t("common.save")}
        </button>
      </PageHeader>
      <Section title={app.t("templates.identity")}>
        <div className="grid gap-3 md:grid-cols-4">
          <label className="text-xs">
            <span className="label">id</span>
            <input className="input mt-1" value={t.id} onChange={(e) => setT({ ...t, id: e.target.value.toLowerCase() })} />
          </label>
          <label className="text-xs">
            <span className="label">{app.t("templates.name")}</span>
            <input className="input mt-1" value={t.name} onChange={(e) => setT({ ...t, name: e.target.value })} />
          </label>
          <label className="text-xs">
            <span className="label">version</span>
            <input className="input mt-1 font-mono" value={t.version} onChange={(e) => setT({ ...t, version: e.target.value })} />
          </label>
          <label className="text-xs">
            <span className="label">report</span>
            <select className="input mt-1" value={t.report_template} onChange={(e) => setT({ ...t, report_template: e.target.value })}>
              {[...new Set([...REPORTS, t.report_template])].map((r) => (
                <option key={r}>{r}</option>
              ))}
            </select>
          </label>
        </div>
        <textarea className="input mt-3 h-16" placeholder={app.t("templates.description")} value={t.description} onChange={(e) => setT({ ...t, description: e.target.value })} />
      </Section>
      <Section title={`${app.t("templates.roles")} (${t.roles.length})`}>
        <div className="space-y-3">
          {t.roles.map((r, i) => (
            <div key={i} className="rounded-lg border border-line bg-elevated/40 p-3">
              <div className="grid gap-2 md:grid-cols-6">
                <input className="input font-mono" placeholder="role id" value={r.id} onChange={(e) => role(i, { id: e.target.value.toLowerCase() })} />
                <select
                  className="input"
                  value={kindOf(r)}
                  onChange={(e) => {
                    const k = e.target.value;
                    role(i, { kind: k === "lead" ? "lead" : "worker", planner: k === "planner", count: k === "worker" ? r.count : 1 });
                  }}
                >
                  <option value="lead">lead</option>
                  <option value="worker">worker</option>
                  <option value="planner">planner</option>
                </select>
                <input className="input" type="number" min={1} max={10} disabled={kindOf(r) !== "worker"} value={r.count} onChange={(e) => role(i, { count: Math.max(1, Math.min(10, Number(e.target.value) || 1)) })} />
                <input className="input" placeholder="tags: a, b" value={r.tags.join(", ")} onChange={(e) => role(i, { tags: list(e.target.value) })} />
                <input className="input font-mono" placeholder="provider/model" value={r.default_model ?? ""} onChange={(e) => role(i, { default_model: e.target.value.trim() || null })} />
                <div className="flex gap-2">
                  <select
                    className="input"
                    value=""
                    onChange={(e) => {
                      const from = roles.find((x) => x.id === e.target.value);
                      if (from) role(i, { prompt: from.prompt, tags: r.tags.length ? r.tags : from.tags, allowed_paths: r.allowed_paths.length ? r.allowed_paths : from.allowed_paths });
                    }}
                  >
                    <option value="">{app.t("templates.startFrom")}</option>
                    <optgroup label={app.t("agents.library")}>
                      {roles
                        .filter((x) => x.template === LIBRARY)
                        .map((x) => (
                          <option key={x.id}>{x.id}</option>
                        ))}
                    </optgroup>
                    <optgroup label={app.t("agents.presets")}>
                      {roles
                        .filter((x) => x.template !== LIBRARY)
                        .map((x) => (
                          <option key={x.id}>{x.id}</option>
                        ))}
                    </optgroup>
                  </select>
                  <button className="btn-danger" title={app.t("common.delete")} disabled={t.roles.length <= 2} onClick={() => setT({ ...t, roles: t.roles.filter((_, j) => j !== i) })}>
                    <Trash2 className="h-3.5 w-3.5" />
                  </button>
                </div>
              </div>
              <input className="input mt-2 font-mono text-[11px]" placeholder={app.t("templates.scope")} value={r.allowed_paths.join(", ")} onChange={(e) => role(i, { allowed_paths: list(e.target.value) })} />
              <textarea className="input mt-2 h-40 font-mono text-[11px]" value={r.prompt} onChange={(e) => role(i, { prompt: e.target.value })} />
            </div>
          ))}
          <button className="btn" onClick={() => setT({ ...t, roles: [...t.roles, blankRole(`role-${t.roles.length + 1}`, "worker")] })}>
            <Plus className="h-3.5 w-3.5" /> {app.t("templates.addRole")}
          </button>
        </div>
      </Section>
    </div>
  );
}
