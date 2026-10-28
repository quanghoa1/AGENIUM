import { Check, FolderPlus, Users } from "lucide-react";
import { useState } from "react";
import { Empty, Mono, PageHeader, ProjectStateChips, cn, projectGlow } from "../../components/ui";
import { api } from "../../lib/api";
import { friendly } from "../../lib/errors";
import { useApp } from "../../lib/store";

/** The project name a folder or repository suggests (`https://github.com/o/my-site.git` -> `my-site`). */
function nameFrom(source: string): string {
  const last = source.trim().replace(/[\\/]+$/, "").split(/[\\/:]/).pop() ?? "";
  return last.replace(/\.git$/i, "");
}

export function ProjectsView() {
  const app = useApp();
  const [name, setName] = useState("");
  const [source, setSource] = useState("");
  const [busy, setBusy] = useState(false);
  // The project just made: the page stays here and points to the next step (a team), instead of jumping away.
  const [created, setCreated] = useState<{ id: string; name: string } | null>(null);
  const suggested = nameFrom(source);
  const finalName = name.trim() || suggested;
  const create = () => {
    setBusy(true);
    api
      .createProject(finalName, source.trim() || undefined)
      .then((p) => (app.setProject(p.id), setName(""), setSource(""), setCreated(p), app.refresh()))
      .catch((e) => app.notify(friendly(e, app.t)))
      .finally(() => setBusy(false));
  };
  return (
    <div className="space-y-4">
      <PageHeader title={app.t("nav.projects")} sub={app.t("projects.sub")} />
      <div className="card flex flex-wrap items-end gap-2 p-4">
        <label className="w-64">
          <span className="label">{app.t("common.name")}</span>
          <input className="input mt-1" value={name} placeholder={suggested || app.t("projects.namePh")} onChange={(e) => setName(e.target.value)} />
        </label>
        <label className="min-w-[280px] flex-1">
          <span className="label">{app.t("projects.folder")}</span>
          <input className="input mt-1" value={source} placeholder={app.t("projects.folderHint")} onChange={(e) => setSource(e.target.value)} />
        </label>
        <button className="btn-primary" disabled={!finalName || busy} title={finalName ? undefined : app.t("projects.needName")} onClick={create}>
          <FolderPlus className="h-3.5 w-3.5" /> {busy ? app.t("projects.creating") : app.t("projects.create")}
        </button>
      </div>
      {created && (
        <div className="flex flex-wrap items-center gap-3 rounded-xl border border-emerald-500/40 bg-emerald-500/10 p-3 text-xs">
          <Check className="h-4 w-4 text-emerald-500" />
          <span>{app.t("projects.created").replace("{x}", created.name)}</span>
          <button className="btn-primary ml-auto" onClick={() => (app.setProject(created.id), app.go("teams"))}>
            <Users className="h-3.5 w-3.5" /> {app.t("projects.nextTeam")}
          </button>
          <button className="btn" onClick={() => setCreated(null)}>
            {app.t("common.close")}
          </button>
        </div>
      )}
      <div className="grid gap-4 lg:grid-cols-2">
        {app.projects.map((p) => (
          <div key={p.id} className={cn("card p-4 transition-shadow", projectGlow(p.run_state, p.needs_you))}>
            <div className="flex items-center justify-between">
              <div>
                <Mono className="text-cyan-600 dark:text-cyan-400">{p.key_prefix}</Mono> <span className="text-sm font-semibold">{p.name}</span>
              </div>
              <ProjectStateChips state={p.run_state} needsYou={p.needs_you} cancelled={p.status === "Cancelled"} />
            </div>
            <div className="mt-3 grid grid-cols-4 gap-2 text-center text-xs">
              {[
                [app.t("projects.team"), p.team ? (p.team.solo ? app.t("teams.solo") : `${p.team.members} ${app.t("projects.members")}`) : "–"],
                [app.t("nav.tickets"), p.tickets],
                [app.t("projects.plan"), `${p.plan_done}/${p.plan_items}`],
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
      {app.projects.length === 0 && <Empty>{app.t("projects.empty")}</Empty>}
    </div>
  );
}
