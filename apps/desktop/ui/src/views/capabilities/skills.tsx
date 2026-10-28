import { AlertTriangle, CheckCircle2, Eye, FilePlus2, FileUp, Loader2, Pencil, Plus, Save, Sparkles, Tag, Trash2, Wrench } from "lucide-react";
import { Suspense, lazy, useEffect, useState } from "react";
import { Markdown } from "../../components/Markdown";
import { ConfirmModal, Modal, Overlay } from "../../components/motion";
import { Empty, Mono, Section, cn } from "../../components/ui";
import { api, inTauri } from "../../lib/api";
import { friendly } from "../../lib/errors";
import { useApp } from "../../lib/store";
import type { LibrarySkill } from "../../lib/types";

const MonacoEditor = lazy(() => import("../../components/MonacoEditor"));

/** The frontmatter `name:` of a SKILL.md (the folder it writes to). */
function frontmatterName(content: string): string {
  // The first `name:` line, which lives in the frontmatter (the body uses `## ` headings, never a `name:` line).
  const m = content.match(/^name:[ \t]*(.+?)[ \t]*$/m);
  return (m?.[1] ?? "").trim();
}

/** A new skill's starting SKILL.md: frontmatter (name, description, tags) + the section skeleton. */
function newSkillTemplate(skeleton: string): string {
  return `---
name: my-skill
description: One line: what it does and when to use it.
tags: 
---

${skeleton}`;
}

/** Author a skill in the app (Monaco), tiers 2 and 3: edit SKILL.md and its references/ and scripts/ files. Evaluate
 *  runs the checks without saving; Save writes the active file. A new skill's folder is the SKILL.md frontmatter name;
 *  reference files can be added once it is saved. */
function SkillEditor({ initial, create: create0, name0, files0, onClose, onSaved }: { initial: string; create: boolean; name0: string; files0: string[]; onClose: () => void; onSaved: () => void }) {
  const app = useApp();
  const SKILL = "SKILL.md";
  const [create, setCreate] = useState(create0);
  const [files, setFiles] = useState<string[]>([SKILL, ...files0]);
  const [active, setActive] = useState(SKILL);
  // Content per file; SKILL.md seeded, reference files loaded on first open.
  const [cache, setCache] = useState<Record<string, string>>({ [SKILL]: initial });
  const [busy, setBusy] = useState(false);
  const [result, setResult] = useState<{ shape_notes: string[]; quality: string[] | null } | null>(null);
  // For a new skill the folder is the frontmatter name; for an existing one it is fixed.
  const name = create ? frontmatterName(cache[SKILL] ?? "") : name0;
  const content = cache[active] ?? "";
  const setContent = (v: string) => setCache((c) => ({ ...c, [active]: v }));
  const open = (path: string) => {
    setResult(null);
    setActive(path);
    if (cache[path] === undefined) {
      api.skillFile(name, path).then((r) => setCache((c) => ({ ...c, [path]: r.content }))).catch((e) => app.notify(friendly(e, app.t)));
    }
  };
  const addFile = () => {
    const path = window.prompt(app.t("library.addFilePrompt"), "references/notes.md")?.trim();
    if (!path) return;
    if (!/^(references|scripts)\/[\w.-]+(\/[\w.-]+)*$/.test(path)) {
      app.notify(app.t("library.addFileBad"));
      return;
    }
    setFiles((f) => (f.includes(path) ? f : [...f, path]));
    setCache((c) => ({ ...c, [path]: c[path] ?? "" }));
    setActive(path);
    setResult(null);
  };
  const evaluate = () => {
    setBusy(true);
    api
      .evaluateSkillText(cache[SKILL] ?? "")
      .then(setResult)
      .catch((e) => app.notify(friendly(e, app.t)))
      .finally(() => setBusy(false));
  };
  const save = () => {
    if (!name) {
      app.notify(app.t("library.needName"));
      return;
    }
    setBusy(true);
    const done = () => (onSaved(), setBusy(false));
    if (active === SKILL) {
      api
        .saveSkill(name, cache[SKILL] ?? "", create)
        .then((r) => {
          setResult({ shape_notes: r.shape_notes, quality: r.quality });
          setCreate(false); // it exists now: reference files can be added
          app.notify(`${app.t("library.saved")} ${r.name}`);
        })
        .catch((e) => app.notify(friendly(e, app.t)))
        .finally(done);
    } else {
      api
        .saveSkillFile(name, active, content)
        .then((r) => {
          setResult({ shape_notes: r.shape_notes, quality: null });
          app.notify(`${app.t("library.saved")} ${active}`);
        })
        .catch((e) => app.notify(friendly(e, app.t)))
        .finally(done);
    }
  };
  const ext = active.endsWith(".md") ? "markdown" : active.endsWith(".py") ? "python" : active.endsWith(".mjs") || active.endsWith(".js") ? "javascript" : active.endsWith(".sh") ? "shell" : active.endsWith(".json") ? "json" : "plaintext";
  return (
    <Overlay open onClose={onClose} kind="dialog" top="pt-[4vh]">
      <div className="card flex h-[88vh] w-[min(1040px,94vw)] flex-col gap-3 p-4" onKeyDown={(e) => e.key === "Escape" && onClose()}>
        <div className="flex items-center gap-2">
          <Pencil className="h-4 w-4 text-accent" />
          <span className="text-sm font-semibold">{create ? app.t("library.newSkill") : `${app.t("library.edit")}: ${name}`}</span>
        </div>
        <div className="text-[11px] text-subtle">{app.t("library.editHint")}</div>
        <div className="flex min-h-0 flex-1 gap-3">
          {/* Tier 2 (SKILL.md) and tier 3 (references/scripts) files of this skill. */}
          <div className="flex w-48 shrink-0 flex-col gap-1 overflow-y-auto">
            {files.map((f) => (
              <button key={f} className={cn("truncate rounded-md px-2 py-1.5 text-left text-[11px]", active === f ? "bg-accent/15 text-accent" : "text-muted hover:bg-elevated")} onClick={() => open(f)} title={f}>
                {f === SKILL ? <Mono>{f}</Mono> : f}
              </button>
            ))}
            <button className={cn("mt-1 flex items-center gap-1 rounded-md px-2 py-1.5 text-left text-[11px]", create ? "text-subtle" : "text-accent hover:bg-elevated")} disabled={create} title={create ? app.t("library.saveFirst") : undefined} onClick={addFile}>
              <Plus className="h-3 w-3" /> {app.t("library.addFile")}
            </button>
          </div>
          <div className="flex min-w-0 flex-1 flex-col gap-2">
            <Suspense fallback={<div className="flex flex-1 items-center justify-center"><Loader2 className="h-5 w-5 animate-spin text-subtle" /></div>}>
              <MonacoEditor key={active} value={content} language={ext} onChange={setContent} />
            </Suspense>
            {result && (
              <div className="max-h-28 space-y-1 overflow-y-auto rounded-lg border border-line bg-elevated/40 p-2.5 text-[11px]">
                {result.shape_notes.length === 0 ? (
                  <div className="flex items-center gap-1.5 text-emerald-600 dark:text-emerald-400">
                    <CheckCircle2 className="h-3.5 w-3.5" /> {app.t("library.shapeOk")}
                  </div>
                ) : (
                  result.shape_notes.map((n, i) => (
                    <div key={i} className="flex items-start gap-1.5 text-amber-600 dark:text-amber-400">
                      <AlertTriangle className="mt-0.5 h-3 w-3 shrink-0" /> {n}
                    </div>
                  ))
                )}
                {(result.quality ?? []).map((q, i) => (
                  <div key={i} className="font-mono text-subtle">
                    <Sparkles className="mr-1 inline h-3 w-3 text-cyan-500" />
                    {q}
                  </div>
                ))}
              </div>
            )}
          </div>
        </div>
        <div className="flex items-center gap-2">
          <Mono className="mr-auto truncate text-subtle">{name ? `${name}/${active}` : app.t("library.needName")}</Mono>
          <button className="btn" onClick={onClose}>
            {app.t("common.cancel")}
          </button>
          <button className="btn" disabled={busy} onClick={evaluate} title={app.t("library.evalHint")}>
            <Sparkles className="h-3.5 w-3.5" /> {app.t("library.evaluate")}
          </button>
          <button className="btn-primary" disabled={busy || !name} onClick={save}>
            <Save className="h-3.5 w-3.5" /> {busy ? app.t("library.saving") : app.t("common.save")}
          </button>
        </div>
      </div>
    </Overlay>
  );
}

export function SkillsPanel() {
  const app = useApp();
  const [lib, setLib] = useState<{ skills: LibrarySkill[]; problems: { dir: string; error: string }[] } | null>(null);
  const [path, setPath] = useState("");
  const [replace, setReplace] = useState(false);
  const [view, setView] = useState<{ name: string; body: string; files: string[] } | null>(null);
  const [del, setDel] = useState<LibrarySkill | null>(null);
  const [drop, setDrop] = useState(false);
  const [tag, setTag] = useState<string | null>(null);
  // The in-app SKILL.md editor (author a new skill or edit one); `create` tells the save to make a new folder.
  const [edit, setEdit] = useState<{ initial: string; create: boolean; name0: string; files0: string[] } | null>(null);
  const openNew = () => api.skillSkeleton().then((r) => setEdit({ initial: newSkillTemplate(r.body), create: true, name0: "", files0: [] }));
  const openEdit = (name: string) => api.skillText(name).then((r) => setEdit({ initial: r.raw || r.body, create: false, name0: name, files0: r.files }));
  // On-demand evaluation (ADR-88/90/92): which skill is running, and the result shown in a popup (keeps the cards the
  // same size — it does not grow the card).
  const [evaluating, setEvaluating] = useState<string | null>(null);
  const [evalResult, setEvalResult] = useState<{ name: string; shape_notes: string[]; quality: string[] | null } | null>(null);
  const load = () => api.skillLibrary().then(setLib);
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
  const doImport = (p: string) =>
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
  // Drop a skill folder or .zip on the window (Tauri gives the paths).
  useEffect(() => {
    if (!inTauri) return;
    // The listener arrives asynchronously: one that lands after cleanup is removed at once (no stacked listeners).
    let un: (() => void) | undefined;
    let gone = false;
    import("@tauri-apps/api/webview").then(({ getCurrentWebview }) =>
      getCurrentWebview()
        .onDragDropEvent((e) => {
          const p = e.payload;
          if (p.type === "enter" || p.type === "over") setDrop(true);
          else if (p.type === "leave") setDrop(false);
          else if (p.type === "drop") {
            setDrop(false);
            if (p.paths[0]) doImport(p.paths[0]);
          }
        })
        .then((f) => (gone ? f() : (un = f))),
    );
    return () => {
      gone = true;
      un?.();
    };
  }, [replace]);
  return (
    <div className="space-y-4">
      <div className={cn("card space-y-3 border-dashed p-4 transition-colors", drop && "border-accent bg-blue-600/10")}>
        <div className="flex items-center gap-2 text-xs font-semibold">
          <FileUp className="h-4 w-4 text-accent" /> {app.t("library.importTitle")}
          <button className="btn ml-auto" onClick={openNew}>
            <FilePlus2 className="h-3.5 w-3.5" /> {app.t("library.newSkill")}
          </button>
        </div>
        <div className="text-[11px] text-subtle">{app.t("library.importNote")}</div>
        <div className="flex flex-wrap gap-2">
          <input className="input min-w-[280px] flex-1 font-mono" value={path} placeholder="C:\skills\my-skill  ·  my-skill.zip" onChange={(e) => setPath(e.target.value)} />
          <label className="flex items-center gap-1.5 text-xs text-muted">
            <input type="checkbox" checked={replace} onChange={(e) => setReplace(e.target.checked)} /> {app.t("library.replace")}
          </label>
          <button className="btn-primary" disabled={!path.trim()} onClick={() => doImport(path.trim())}>
            <Plus className="h-3.5 w-3.5" /> {app.t("library.import")}
          </button>
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
