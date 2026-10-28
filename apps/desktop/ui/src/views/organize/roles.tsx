import { AlertTriangle } from "lucide-react";
import { Mono } from "../../components/ui";
import { api } from "../../lib/api";
import { missionOf } from "../../lib/roles";
import { useApp } from "../../lib/store";
import type { RoleInfo } from "../../lib/types";

export const NEW_ROLE = "__new__";

/** Role picker: the user's roles, then template presets, then "new role" with its editor. */
export function RolePicker({ roles, value, onChange, draft, setDraft }: { roles: RoleInfo[]; value: string; onChange: (id: string) => void; draft: RoleDraft; setDraft: (d: RoleDraft) => void }) {
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
          <div className="line-clamp-3 text-muted">{missionOf(r.prompt)}</div>
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

export interface RoleDraft {
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

export const emptyRole: RoleDraft = { id: "", prompt: ROLE_SKELETON, tags: "", paths: "", lead: false, planner: false };

/** Sections still missing or empty in a role text (the server's check adds length and near-duplicates). */
export function draftLint(text: string): string[] {
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

export const list = (s: string) =>
  s
    .split(",")
    .map((x) => x.trim())
    .filter(Boolean);

export async function saveRoleIfNew(role: string, draft: RoleDraft): Promise<string> {
  if (role !== NEW_ROLE) return role;
  const r = await api.putRole({ id: draft.id, prompt: draft.prompt, tags: list(draft.tags), allowed_paths: list(draft.paths), planner: draft.planner, lead: draft.lead });
  return r.id;
}
