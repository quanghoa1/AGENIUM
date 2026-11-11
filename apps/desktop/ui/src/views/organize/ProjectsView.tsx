import { Check, FolderOpen, FolderPlus, Users } from "lucide-react";
import { CloseProject } from "../../components/CloseProject";
import { HandoverReport } from "../../components/HandoverReport";
import { ReopenProject } from "../../components/ReopenProject";
import { useEffect, useState } from "react";
import { Empty, Mono, PageHeader, ProjectStateChips, cn, projectGlow } from "../../components/ui";
import { api } from "../../lib/api";
import { friendly } from "../../lib/errors";
import { useApp } from "../../lib/store";
import type { Key } from "../../lib/i18n";
import type { FolderInfo, Reference } from "../../lib/types";
import { ProjectReferences, ReferenceFolders } from "./ReferenceFolders";

/** The project name a folder or repository suggests (`https://github.com/o/my-site.git` -> `my-site`). */
function nameFrom(source: string): string {
  const last = source.trim().replace(/[\\/]+$/, "").split(/[\\/:]/).pop() ?? "";
  return last.replace(/\.git$/i, "");
}

/** A full path (a folder picked anywhere) rather than a folder name or a repository. */
const isFullPath = (s: string) => /^([A-Za-z]:[\\/]|\/|\\\\)/.test(s.trim());

/** A repository to clone (as the core reads it: `is_repo_url`). */
const isRepoUrl = (s: string) => /^(https?:\/\/|git@|ssh:\/\/)/.test(s.trim()) || s.trim().endsWith(".git");

function mb(bytes: number): string {
  return bytes < 1e6 ? `${Math.max(1, Math.round(bytes / 1e3))} KB` : `${(bytes / 1e6).toFixed(bytes < 1e8 ? 1 : 0)} MB`;
}

/** What creating the project in the picked folder does, in one line. */
function folderNote(f: FolderInfo, t: (k: Key) => string): { text: string; warn: boolean } {
  if (f.inside) return { text: t("projects.inside").replace("{x}", f.inside), warn: true };
  if (f.project) return { text: t("projects.isProject"), warn: false };
  if (f.repo)
    return f.repo.branch
      ? { text: t(f.repo.changes ? "projects.repoChanges" : "projects.repo").replace("{b}", f.repo.branch), warn: false }
      : { text: t("projects.repoDetached"), warn: true };
  if (!f.exists || f.empty) return { text: t("projects.emptyFolder"), warn: false };
  return { text: t("projects.files").replace("{n}", String(f.files)).replace("{s}", mb(f.bytes)), warn: f.bytes > 500e6 };
}

export function ProjectsView() {
  const app = useApp();
  const [name, setName] = useState("");
  const [source, setSource] = useState("");
  // The tickets' prefix (UIDE-1, UIDE-2…): the user's, or the one made from the name, shown until one is typed.
  const [prefix, setPrefix] = useState("");
  const [suggestedPrefix, setSuggestedPrefix] = useState("");
  const [busy, setBusy] = useState(false);
  // The project just made: the page stays here and points to the next step (a team), instead of jumping away.
  const [created, setCreated] = useState<{ id: string; name: string } | null>(null);
  const [refs, setRefs] = useState<Reference[]>([]);
  // A machine without git (a clean Windows) is told here, before the form, not after Create.
  const [noGit, setNoGit] = useState(false);
  useEffect(() => {
    api.gitAvailable().then((ok) => setNoGit(!ok), () => undefined);
  }, []);
  const [folder, setFolder] = useState<FolderInfo | null>(null);
  // A repository (cloned or picked): the team works on its own branch, or on a new one made for it (ADR-159).
  const [onNew, setOnNew] = useState(false);
  const [newBranch, setNewBranch] = useState("agenium-work");
  const repoBranch = isRepoUrl(source) ? "" : folder?.repo?.branch && !folder.project ? folder.repo.branch : null;
  useEffect(() => {
    setFolder(null);
    if (!isFullPath(source)) return;
    const t = window.setTimeout(() => api.inspectFolder(source.trim()).then(setFolder, () => undefined), 300);
    return () => window.clearTimeout(t);
  }, [source]);
  const suggested = nameFrom(source);
  const finalName = name.trim() || suggested;
  useEffect(() => {
    if (!finalName) return setSuggestedPrefix("");
    const t = window.setTimeout(() => api.suggestKeyPrefix(finalName).then(setSuggestedPrefix, () => undefined), 250);
    return () => window.clearTimeout(t);
  }, [finalName, app.projects.length]);
  const create = () => {
    setBusy(true);
    api
      .createProject(finalName, source.trim() || undefined, prefix.trim() || undefined, repoBranch !== null && onNew ? newBranch.trim() : undefined)
      // The reference folders are the project's setting: saved right after it exists.
      .then((p) => (refs.length ? api.setReferences(p.id, refs).then(() => p) : p))
      .then((p) => (app.setProject(p.id), setName(""), setSource(""), setPrefix(""), setRefs([]), setCreated(p), app.refresh()))
      .catch((e) => app.notify(friendly(e, app.t)))
      .finally(() => setBusy(false));
  };
  return (
    <div className="space-y-4">
      <PageHeader title={app.t("nav.projects")} sub={app.t("projects.sub")} />
      {noGit && (
        <div role="alert" className="rounded-xl border border-amber-500/40 bg-amber-500/10 p-3 text-xs text-amber-700 dark:text-amber-200">
          {app.t("err.noGit")}
        </div>
      )}
      <div className="card flex flex-wrap items-end gap-2 p-4">
        <label className="w-64">
          <span className="label">{app.t("common.name")}</span>
          <input className="input mt-1" value={name} placeholder={suggested || app.t("projects.namePh")} onChange={(e) => setName(e.target.value)} />
        </label>
        <label className="w-40" title={app.t("projects.prefixHint")}>
          <span className="label">{app.t("projects.prefix")}</span>
          <input
            className="input mt-1 font-mono uppercase"
            value={prefix}
            maxLength={10}
            placeholder={suggestedPrefix}
            onChange={(e) => setPrefix(e.target.value.toUpperCase().replace(/[^A-Z]/g, ""))}
          />
        </label>
        <label className="min-w-[280px] flex-1">
          <span className="label">{app.t("projects.folder")}</span>
          <div className="mt-1 flex gap-1">
            <input className="input" value={source} placeholder={app.t("projects.folderHint")} onChange={(e) => setSource(e.target.value)} />
            <button className="btn shrink-0" title={app.t("projects.pickHint")} onClick={() => api.pickFolder().then((p) => p && setSource(p), (e) => app.notify(friendly(e, app.t)))}>
              <FolderOpen className="h-3.5 w-3.5" /> {app.t("projects.pick")}
            </button>
          </div>
        </label>
        <button className="btn-primary" disabled={!finalName || busy} title={finalName ? undefined : app.t("projects.needName")} onClick={create}>
          <FolderPlus className="h-3.5 w-3.5" /> {busy ? app.t("projects.creating") : app.t("projects.create")}
        </button>
        {folder && (
          <div className={cn("w-full text-xs", folderNote(folder, app.t).warn ? "text-amber-600 dark:text-amber-300" : "text-subtle")}>
            {folderNote(folder, app.t).text}
          </div>
        )}
        {repoBranch !== null && (
          <div className="flex w-full flex-wrap items-center gap-3 text-xs" role="radiogroup" aria-label={app.t("projects.branch")}>
            <span className="label">{app.t("projects.branch")}</span>
            <label className="flex items-center gap-1.5">
              <input type="radio" checked={!onNew} onChange={() => setOnNew(false)} />
              {repoBranch ? app.t("projects.branchOwn").replace("{b}", repoBranch) : app.t("projects.branchDefault")}
            </label>
            <label className="flex items-center gap-1.5">
              <input type="radio" checked={onNew} onChange={() => setOnNew(true)} />
              {app.t("projects.branchNew")}
            </label>
            <input
              className="input w-40 py-1 font-mono"
              value={newBranch}
              disabled={!onNew}
              aria-label={app.t("projects.branchNew")}
              onChange={(e) => setNewBranch(e.target.value.replace(/\s/g, "-"))}
            />
            <span className="text-subtle">{app.t(onNew ? (repoBranch ? "projects.branchNewPicked" : "projects.branchNewClone") : "projects.branchOwnHint")}</span>
          </div>
        )}
        <div className="w-full">
          <ReferenceFolders value={refs} onChange={setRefs} />
        </div>
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
              <div className="flex items-center gap-2">
                <span className="text-sm font-semibold">{p.name}</span>
                <span title={app.t("projects.prefixHint")}>
                  <Mono className="text-subtle">{p.key_prefix}-#</Mono>
                </span>
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
            <div className="mt-2 truncate font-mono text-[11px] text-subtle" title={p.path}>
              {p.path}
            </div>
            {p.run_state !== "Closed" && <ProjectReferences project={p.id} refs={p.references ?? []} />}
            <div className="mt-3 flex gap-2">
              <button className="btn" onClick={() => (app.setProject(p.id), app.go("tickets"))}>
                {app.t("nav.tickets")}
              </button>
              <button className="btn" onClick={() => (app.setProject(p.id), app.go("chat"))}>
                {app.t("top.talk")}
              </button>
              <HandoverReport p={p} />
              {p.run_state === "Done" && <CloseProject p={p} />}
              {p.run_state === "Closed" && <ReopenProject p={p} />}
              {p.waits_for && (
                <span className="chip self-center border-amber-500/40 bg-amber-500/10 text-amber-600 dark:text-amber-300">
                  {app.t("run.waiting").replace("{team}", p.waits_for)}
                </span>
              )}
              {!p.team && !p.waits_for && p.run_state !== "Closed" && (
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
