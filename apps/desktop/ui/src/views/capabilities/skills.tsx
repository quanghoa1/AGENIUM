import { AlertTriangle, CheckCircle2, Eye, FilePlus2, FileUp, FolderOpen, Pencil, Plus, Sparkles, Tag, Trash2, Wand2, Wrench } from "lucide-react";
import { useEffect, useState } from "react";
import { Markdown } from "../../components/Markdown";
import { ConfirmModal, Modal } from "../../components/motion";
import { Empty, Mono, Section, cn } from "../../components/ui";
import { api, type FoundSkill, type SkillEffect } from "../../lib/api";
import { useFileDrop } from "../../lib/drops";
import { EffectBadge } from "./Scorecard";
import { useApp } from "../../lib/store";
import type { LibrarySkill } from "../../lib/types";
import { SkillEditor } from "./SkillEditor";
import { SkillPicker } from "./SkillPicker";
import { SkillWriter } from "./SkillWriter";


/** A new skill's starting SKILL.md: frontmatter (name, description, tags) + the section skeleton. */
function newSkillTemplate(skeleton: string): string {
  return `---
name: 
description: 
tags: 
---

${skeleton}`;
}

export function SkillsPanel() {
  const app = useApp();
  const [lib, setLib] = useState<{ skills: LibrarySkill[]; problems: { dir: string; error: string }[] } | null>(null);
  const [path, setPath] = useState("");
  const [replace, setReplace] = useState(false);
  const [view, setView] = useState<{ name: string; body: string; files: string[] } | null>(null);
  const [del, setDel] = useState<LibrarySkill | null>(null);
  const [drop, setDrop] = useState(false);
  // The skills a chosen folder holds, when it holds more than the one skill it is: ticked, then imported together.
  const [found, setFound] = useState<FoundSkill[] | null>(null);
  const [tag, setTag] = useState<string | null>(null);
  // The in-app SKILL.md editor (author a new skill or edit one); `create` tells the save to make a new folder.
  const [edit, setEdit] = useState<{ initial: string; create: boolean; name0: string; files0: string[] } | null>(null);
  // Writing a skill with an agent (ADR-151), beside writing it by hand.
  const [writing, setWriting] = useState(false);
  const openNew = () => api.skillSkeleton().then((r) => setEdit({ initial: newSkillTemplate(r.body), create: true, name0: "", files0: [] }));
  const openEdit = (name: string) => api.skillText(name).then((r) => setEdit({ initial: r.raw || r.body, create: false, name0: name, files0: r.files }));
  // On-demand evaluation (ADR-88/90/92): which skill is running, and the result shown in a popup (keeps the cards the
  // same size — it does not grow the card).
  const [evaluating, setEvaluating] = useState<string | null>(null);
  const [evalResult, setEvalResult] = useState<{ name: string; shape_notes: string[]; quality: string[] | null } | null>(null);
  // Each skill's effect on the closed tickets it was loaded in (ADR-152, criterion 12): free, from the run log.
  const [effects, setEffects] = useState<Record<string, SkillEffect>>({});
  const load = () => (api.skillLibrary().then(setLib), api.skillEffects().then(setEffects, () => undefined));
  const evaluate = (name: string) => {
    setEvaluating(name);
    api
      .evaluateSkill(name)
      .then((r) => setEvalResult({ name, ...r }))
      .catch((err) => app.notify(String(err)))
      .finally(() => setEvaluating(null));
  };
  // Every tag used in the library, for the filter (free-form; no fixed vocabulary).
  const allTags = (lib?.skills ?? []).flatMap((s) => s.tags ?? []).filter((t, i, a) => a.indexOf(t) === i).sort();
  useEffect(() => {
    load();
  }, [app.tick]);
  // A folder is read first: a skill folder imports at once, a folder of skills opens the list to tick (ADR-88).
  const doImport = (p: string) =>
    /\.zip$/i.test(p.trim())
      ? importOne(p)
      : api
          .scanSkills(p)
          .then((fs) => {
            if (fs.length === 0) return app.notify(app.t("library.noneFound").replace("{x}", p));
            const self = fs.length === 1 && fs[0].path.replace(/[\\/]+$/, "").toLowerCase() === p.trim().replace(/[\\/]+$/, "").toLowerCase();
            if (self) return importOne(p);
            setFound(fs);
          })
          .catch((e) => app.notify(String(e)));
  const importOne = (p: string) =>
    api
      .importSkill(p, replace)
      .then((m) => {
        app.notify(`${app.t("library.imported")} ${m.name}`);
        // ADR-88: surface the skill-shape advice (one job, when-to-use, steps, tools, proof) so the user can improve it.
        if (m.shape_notes?.length) {
          app.notify(`${app.t("library.shapeNotes")} ${m.name}: ${m.shape_notes.join("; ")}`);
        }
        // ADR-90: the judge's calibrated read of the skill against the methodology principles (only when it is on).
        if (m.quality?.length) {
          app.notify(`${app.t("library.quality")} ${m.name}: ${m.quality.join("; ")}`);
        }
        setPath("");
        load();
      })
      .catch((e) => app.notify(String(e)));
  // Drop a skill folder or .zip on the window (Tauri gives the paths); a chat opened over the page takes drops instead.
  useFileDrop((e) => {
    if (e.type === "enter" || e.type === "over") setDrop(true);
    else if (e.type === "leave") setDrop(false);
    else {
      setDrop(false);
      if (e.paths[0]) doImport(e.paths[0]);
    }
  });
  return (
    <div className="space-y-4">
      {/* Two ways in: write a new skill in the app, or bring one in (a folder or a .zip, typed or dropped). */}
      <div className="grid gap-3 lg:grid-cols-[minmax(260px,340px)_minmax(260px,340px)_1fr]">
        <div className="card flex flex-col gap-2 p-4">
          <div className="flex items-center gap-2 text-xs font-semibold">
            <FilePlus2 className="h-4 w-4 text-accent" /> {app.t("library.writeTitle")}
          </div>
          <div className="flex-1 text-[11px] leading-relaxed text-subtle">{app.t("library.writeNote")}</div>
          <button className="btn-primary self-start" onClick={openNew}>
            <Plus className="h-3.5 w-3.5" /> {app.t("library.newSkill")}
          </button>
        </div>
        <div className="card flex flex-col gap-2 p-4">
          <div className="flex items-center gap-2 text-xs font-semibold">
            <Wand2 className="h-4 w-4 text-accent" /> {app.t("skillWriter.cardTitle")}
          </div>
          <div className="flex-1 text-[11px] leading-relaxed text-subtle">{app.t("skillWriter.cardNote")}</div>
          <button className="btn-primary self-start" onClick={() => setWriting(true)}>
            <Wand2 className="h-3.5 w-3.5" /> {app.t("skillWriter.auto")}
          </button>
        </div>
        <div className={cn("card flex flex-col gap-2 border-dashed p-4 transition-colors", drop && "border-accent bg-blue-600/10")}>
          <div className="flex items-center gap-2 text-xs font-semibold">
            <FileUp className="h-4 w-4 text-accent" /> {app.t("library.importTitle")}
          </div>
          <div className="flex-1 text-[11px] leading-relaxed text-subtle">{app.t("library.importNote")}</div>
          <div className="flex flex-wrap items-center gap-2">
            <input className="input min-w-[280px] flex-1 font-mono" value={path} placeholder="C:\skills\my-skill  ·  my-skill.zip" onChange={(e) => setPath(e.target.value)} />
            <label className="flex items-center gap-1.5 text-xs text-muted">
              <input type="checkbox" checked={replace} onChange={(e) => setReplace(e.target.checked)} /> {app.t("library.replace")}
            </label>
            <button className="btn" title={app.t("library.chooseHint")} onClick={() => api.pickFolder().then((p) => { if (p) { setPath(p); doImport(p); } }, (e) => app.notify(String(e)))}>
              <FolderOpen className="h-3.5 w-3.5" /> {app.t("projects.pick")}
            </button>
            <button className="btn" disabled={!path.trim()} onClick={() => doImport(path.trim())}>
              <FileUp className="h-3.5 w-3.5" /> {app.t("library.import")}
            </button>
          </div>
        </div>
      </div>
      {!lib && <div className="text-xs text-subtle">{app.t("common.loading")}</div>}
      {allTags.length > 0 && (
        <div className="flex flex-wrap items-center gap-1.5">
          <Tag className="h-3.5 w-3.5 text-subtle" />
          <button className={cn("chip", tag === null ? "border-accent text-accent" : "border-line-strong text-muted")} onClick={() => setTag(null)}>
            {app.t("library.allTags")}
          </button>
          {allTags.map((t) => (
            <button key={t} className={cn("chip", tag === t ? "border-indigo-500/50 bg-indigo-500/10 text-indigo-600 dark:text-indigo-300" : "border-line-strong text-muted")} onClick={() => setTag(tag === t ? null : t)}>
              {t}
            </button>
          ))}
        </div>
      )}
      <div className="grid gap-3 lg:grid-cols-2 2xl:grid-cols-3">
        {lib?.skills.filter((s) => tag === null || (s.tags ?? []).includes(tag)).map((s) => (
          <div key={s.name} className="card flex flex-col p-4">
            <div className="flex items-start gap-2">
              <Wrench className="mt-0.5 h-4 w-4 shrink-0 text-cyan-500" />
              <span className="text-sm font-semibold">{s.name}</span>
              {s.version && <Mono className="mt-0.5 text-subtle">v{s.version}</Mono>}
              {/* The skill's domain tags on the right of the name row: a glance shows which fields it belongs to. */}
              {!!s.tags?.length && (
                <div className="ml-auto flex flex-wrap justify-end gap-1">
                  {s.tags.map((t) => (
                    <button key={t} className="chip border-indigo-500/30 text-indigo-600 dark:text-indigo-300" onClick={() => setTag(t)} title={app.t("library.filterTag")}>
                      {t}
                    </button>
                  ))}
                </div>
              )}
            </div>
            <div className="mt-1.5 line-clamp-3 text-xs text-muted">{s.description}</div>
            <div className="mt-2 flex flex-1 flex-wrap items-center gap-1">
              <span className="label mr-1">{app.t("library.enabledBy")}</span>
              {s.users.length ? (
                s.users.map((u) => (
                  <span key={u} className="chip border-line-strong bg-elevated text-muted">
                    {u}
                  </span>
                ))
              ) : (
                <span className="text-[11px] text-subtle">–</span>
              )}
              <EffectBadge e={effects[s.name]} />
              <Mono className="ml-auto text-subtle">
                {s.files} {app.t(s.files === 1 ? "library.file" : "library.files")}
              </Mono>
            </div>
            <div className="mt-3 flex gap-2">
              <button className="btn" onClick={() => api.skillText(s.name).then(setView)}>
                <Eye className="h-3.5 w-3.5" /> {app.t("library.read")}
              </button>
              <button className="btn" onClick={() => openEdit(s.name)}>
                <Pencil className="h-3.5 w-3.5" /> {app.t("library.edit")}
              </button>
              <button className="btn" disabled={evaluating === s.name} onClick={() => evaluate(s.name)}>
                <Sparkles className="h-3.5 w-3.5" /> {evaluating === s.name ? app.t("library.evaluating") : app.t("library.evaluate")}
              </button>
              <button className="btn" onClick={() => setDel(s)}>
                <Trash2 className="h-3.5 w-3.5" /> {app.t("common.delete")}
              </button>
            </div>

          </div>
        ))}
      </div>
      {lib && lib.skills.length === 0 && <Empty>{app.t("library.noSkills")}</Empty>}
      {lib && lib.problems.length > 0 && (
        <Section title={app.t("library.invalid")}>
          {lib.problems.map((p) => (
            <div key={p.dir} className="text-[11px]">
              <Mono className="text-rose-600 dark:text-rose-400">{p.dir}</Mono> — {p.error}
            </div>
          ))}
        </Section>
      )}
      <SkillPicker found={found} replace={replace} onClose={() => setFound(null)} onDone={() => (setFound(null), setPath(""), load())} />
      <Modal open={!!view} title={view?.name ?? ""} onClose={() => setView(null)}>
        <div className="max-h-[60vh] overflow-y-auto text-xs">{view && <Markdown text={view.body} />}</div>
        {view && view.files.length > 0 && <div className="mt-3 font-mono text-[11px] text-subtle">{view.files.join(" · ")}</div>}
      </Modal>
      <Modal open={!!evalResult} title={`${app.t("library.evaluate")}: ${evalResult?.name ?? ""}`} onClose={() => setEvalResult(null)}>
        {evalResult && (
          <div className="space-y-1.5 text-xs">
            {evalResult.shape_notes.length === 0 ? (
              <div className="flex items-center gap-1.5 text-emerald-600 dark:text-emerald-400">
                <CheckCircle2 className="h-4 w-4" /> {app.t("library.shapeOk")}
              </div>
            ) : (
              evalResult.shape_notes.map((n, i) => (
                <div key={i} className="flex items-start gap-1.5 text-amber-600 dark:text-amber-400">
                  <AlertTriangle className="mt-0.5 h-3.5 w-3.5 shrink-0" /> {n}
                </div>
              ))
            )}
            <div className="border-t border-line pt-2">
              {evalResult.quality === null ? (
                <div className="text-subtle">{app.t("library.qualityOff")}</div>
              ) : (
                evalResult.quality.map((q, i) => (
                  <div key={i} className="font-mono text-[11px] text-muted">
                    <Sparkles className="mr-1 inline h-3 w-3 text-cyan-500" />
                    {q}
                  </div>
                ))
              )}
            </div>
          </div>
        )}
      </Modal>
      {writing && <SkillWriter onClose={() => setWriting(false)} onSaved={() => (setWriting(false), load())} />}
      {edit && <SkillEditor initial={edit.initial} create={edit.create} name0={edit.name0} files0={edit.files0} onClose={() => setEdit(null)} onSaved={load} />}
      <ConfirmModal
        open={!!del}
        title={`${app.t("common.delete")} ${del?.name ?? ""}`}
        note={del?.users.length ? `${app.t("library.inUse")} ${del.users.join(", ")}.` : app.t("library.deleteNote")}
        confirm={del?.users.length ? app.t("library.deleteForce") : app.t("common.delete")}
        onClose={() => setDel(null)}
        onConfirm={() =>
          api
            .removeSkill(del!.name, del!.users.length > 0)
            .then(() => (setDel(null), load()))
            .catch((e) => (setDel(null), app.notify(String(e))))
        }
      />
    </div>
  );
}
