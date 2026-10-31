// The skill editor (author a new skill or edit one): its name, description and tags as fields, the SKILL.md
// instructions in Monaco, and its files grouped as references, scripts and assets — added, uploaded, edited or removed
// in place. Save writes everything in one step, validated together (a SKILL.md may name a reference added with it).
import { AlertTriangle, CheckCircle2, FileCode2, FileImage, FileText, FileUp, GitCompare, Loader2, Pencil, Plus, Save, Sparkles, Trash2, X } from "lucide-react";
import { Suspense, lazy, useMemo, useRef, useState } from "react";
import { ConfirmModal, Overlay } from "../../components/motion";
import { Mono, cn } from "../../components/ui";
import { api, type Scorecard } from "../../lib/api";
import { friendly } from "../../lib/errors";
import { useApp } from "../../lib/store";
import { ScorecardView } from "./Scorecard";

const MonacoEditor = lazy(() => import("../../components/MonacoEditor"));
const MonacoDiff = lazy(() => import("../../components/MonacoDiff"));

const SKILL = "SKILL.md";
const DIRS = ["references", "scripts", "assets"] as const;
type Dir = (typeof DIRS)[number];
/** A file uploaded from disk is read up to this size (the library caps a whole skill at 50 MB). */
const MAX_UPLOAD = 2 * 1024 * 1024;

/** One of the skill's own files as the editor holds it. */
type SkillFile = { path: string; status: "saved" | "new" | "changed"; binary?: { size: number; base64?: string; preview?: string } };

/** Splits a SKILL.md into its frontmatter fields (in order) and its body. */
function splitSkill(raw: string): { fields: [string, string][]; body: string } {
  const m = raw.match(/^---\r?\n([\s\S]*?)\r?\n---[ \t]*\r?\n?/);
  if (!m) return { fields: [], body: raw };
  const fields = m[1]
    .split(/\r?\n/)
    .map((l) => l.match(/^([A-Za-z0-9_-]+):[ \t]*(.*)$/))
    .filter((x): x is RegExpMatchArray => !!x)
    .map((x) => [x[1], x[2].trim()] as [string, string]);
  return { fields, body: raw.slice(m[0].length).replace(/^\r?\n/, "") };
}

/** The SKILL.md the fields and the body make; other frontmatter fields (version...) are kept as they were. */
function joinSkill(name: string, description: string, tags: string, extra: [string, string][], body: string): string {
  const one = (s: string) => s.replace(/\s*\r?\n\s*/g, " ").trim();
  const tagList = tags.split(/[\s,]+/).map((t) => t.trim().toLowerCase()).filter(Boolean);
  const lines = [`name: ${one(name)}`, `description: ${one(description)}`, ...(tagList.length ? [`tags: ${tagList.join(" ")}`] : []), ...extra.map(([k, v]) => `${k}: ${v}`)];
  return `---\n${lines.join("\n")}\n---\n\n${body.replace(/^\s+/, "")}`;
}

/** A skill name the library takes: lowercase letters, digits and single hyphens. */
const validName = (n: string) => /^[a-z0-9]+(-[a-z0-9]+)*$/.test(n) && n.length <= 64;
/** A file name made plain (letters, digits, . _ -). */
const plainName = (n: string) => n.trim().replace(/[^A-Za-z0-9._-]+/g, "-").replace(/^-+|-+$/g, "");
const kb = (n: number) => `${Math.max(1, Math.round(n / 1024))} KB`;

function language(path: string): string {
  const ext = path.split(".").pop()?.toLowerCase() ?? "";
  const map: Record<string, string> = { md: "markdown", py: "python", js: "javascript", mjs: "javascript", ts: "typescript", sh: "shell", bash: "shell", ps1: "powershell", json: "json", yaml: "yaml", yml: "yaml", toml: "ini", sql: "sql", html: "html", css: "css" };
  return map[ext] ?? "plaintext";
}

/** The media type of an image file the editor can show. */
function imageType(path: string): string | null {
  const ext = path.split(".").pop()?.toLowerCase() ?? "";
  return ({ png: "image/png", jpg: "image/jpeg", jpeg: "image/jpeg", gif: "image/gif", webp: "image/webp" } as Record<string, string>)[ext] ?? null;
}

function fileIcon(path: string) {
  if (path.startsWith("assets/") && /\.(png|jpe?g|gif|webp|svg)$/i.test(path)) return FileImage;
  return path.startsWith("scripts/") ? FileCode2 : FileText;
}

/** Reads a file from disk: text when it is UTF-8, otherwise base64. */
async function readUpload(f: File): Promise<{ text?: string; base64?: string }> {
  const buf = new Uint8Array(await f.arrayBuffer());
  try {
    return { text: new TextDecoder("utf-8", { fatal: true }).decode(buf) };
  } catch {
    let bin = "";
    for (let i = 0; i < buf.length; i += 0x8000) bin += String.fromCharCode(...buf.subarray(i, i + 0x8000));
    return { base64: btoa(bin) };
  }
}

export function SkillEditor({
  initial,
  create: create0,
  name0,
  files0,
  newFiles0 = [],
  origin = "user",
  original,
  onClose,
  onSaved,
}: {
  initial: string;
  create: boolean;
  name0: string;
  files0: string[];
  /** Files not saved yet that the skill comes with (a skill the skill writer drafted, ADR-151). */
  newFiles0?: { path: string; text: string }[];
  /** Where the skill comes from, for its scorecard (ADR-152): `user`, `writer`, `writer-docs`, `proposal:<n>`. */
  origin?: string;
  /** The library skill's SKILL.md before this change (a merge drafted by the skill writer): compared side by side. */
  original?: string;
  onClose: () => void;
  onSaved: (name?: string) => void;
}) {
  const app = useApp();
  const parsed = useMemo(() => splitSkill(initial), [initial]);
  const field = (k: string) => parsed.fields.find(([f]) => f === k)?.[1] ?? "";
  const [create, setCreate] = useState(create0);
  const [name, setName] = useState(create0 ? field("name") : name0);
  const [description, setDescription] = useState(field("description"));
  const [tags, setTags] = useState(field("tags"));
  const extra = parsed.fields.filter(([k]) => !["name", "description", "tags"].includes(k));
  const [body, setBody] = useState(parsed.body);
  const [saved, setSaved] = useState(original ?? initial);
  const [card, setCard] = useState<Scorecard | null | undefined>(undefined);
  const [compare, setCompare] = useState(!!original);
  // A drafted file the skill already has (a merge) replaces it: listed once, as changed.
  const [files, setFiles] = useState<SkillFile[]>([
    ...files0.map((path) => ({ path, status: newFiles0.some((f) => f.path === path) ? ("changed" as const) : ("saved" as const) })),
    ...newFiles0.filter((f) => !files0.includes(f.path)).map((f) => ({ path: f.path, status: "new" as const })),
  ]);
  const [removed, setRemoved] = useState<string[]>([]);
  // Text of the files opened or added (a saved file is loaded on first open).
  const [texts, setTexts] = useState<Record<string, string>>(() => Object.fromEntries(newFiles0.map((f) => [f.path, f.text])));
  const [active, setActive] = useState(SKILL);
  const [adding, setAdding] = useState<{ dir: Dir; name: string } | null>(null);
  const [busy, setBusy] = useState(false);
  const [result, setResult] = useState<{ shape_notes: string[]; quality: string[] | null } | null>(null);
  const [leaving, setLeaving] = useState(false);
  const upload = useRef<HTMLInputElement>(null);
  const uploadDir = useRef<Dir>("references");

  const content = joinSkill(name, description, tags, extra, body);
  const changes = (content !== saved ? 1 : 0) + files.filter((f) => f.status !== "saved").length + removed.length;
  const nameOk = validName(name);
  const activeFile = files.find((f) => f.path === active);

  const open = (path: string) => {
    setActive(path);
    const f = files.find((x) => x.path === path);
    if (path === SKILL || !f || f.binary || texts[path] !== undefined || f.status === "new") return;
    api
      .skillFile(name0, path)
      .then((r) => {
        if (r.binary) setFiles((fs) => fs.map((x) => (x.path === path ? { ...x, binary: { size: r.size ?? 0, preview: r.base64 } } : x)));
        else setTexts((t) => ({ ...t, [path]: r.content ?? "" }));
      })
      .catch((e) => app.notify(friendly(e, app.t)));
  };
  const put = (path: string, data: { text?: string; base64?: string; size?: number }) => {
    setRemoved((r) => r.filter((x) => x !== path));
    setFiles((fs) => {
      const was = fs.find((x) => x.path === path);
      const status: SkillFile["status"] = was && was.status === "saved" ? "changed" : was?.status ?? (removed.includes(path) ? "changed" : "new");
      const entry: SkillFile = { path, status, binary: data.base64 !== undefined ? { size: data.size ?? 0, base64: data.base64 } : undefined };
      return was ? fs.map((x) => (x.path === path ? entry : x)) : [...fs, entry].sort((a, b) => a.path.localeCompare(b.path));
    });
    if (data.text !== undefined) setTexts((t) => ({ ...t, [path]: data.text! }));
  };
  const addFile = () => {
    if (!adding) return;
    const n = plainName(adding.name);
    if (!n) return;
    const path = `${adding.dir}/${n}`;
    if (!files.some((f) => f.path === path)) put(path, { text: "" });
    setAdding(null);
    setActive(path);
  };
  const remove = (path: string) => {
    const f = files.find((x) => x.path === path);
    setFiles((fs) => fs.filter((x) => x.path !== path));
    if (f && f.status !== "new") setRemoved((r) => [...r, path]);
    if (active === path) setActive(SKILL);
  };
  const onUpload = async (list: FileList | null) => {
    for (const f of Array.from(list ?? [])) {
      if (f.size > MAX_UPLOAD) {
        app.notify(`${f.name}: ${app.t("library.tooLarge")} (${kb(f.size)} > ${kb(MAX_UPLOAD)})`);
        continue;
      }
      const path = `${uploadDir.current}/${plainName(f.name)}`;
      put(path, { ...(await readUpload(f)), size: f.size });
      setActive(path);
    }
    if (upload.current) upload.current.value = "";
  };
  const edit = (v: string) => {
    if (active === SKILL) setBody(v);
    else {
      setTexts((t) => ({ ...t, [active]: v }));
      setFiles((fs) => fs.map((x) => (x.path === active && x.status === "saved" ? { ...x, status: "changed" } : x)));
    }
  };
  const evaluate = () => {
    setBusy(true);
    setCard(null);
    api
      .skillScorecard(name, description, body, origin, create ? null : name)
      .then(setCard, () => setCard(undefined));
    api
      .evaluateSkillText(content)
      .then(setResult)
      .catch((e) => app.notify(friendly(e, app.t)))
      .finally(() => setBusy(false));
  };
  const save = () => {
    if (!nameOk) return app.notify(app.t("library.badName"));
    setBusy(true);
    const sent = files
      .filter((f) => f.status !== "saved")
      .map((f) => (f.binary?.base64 !== undefined ? { path: f.path, base64: f.binary.base64 } : { path: f.path, text: texts[f.path] ?? "" }));
    api
      .saveSkill(name, content, create, sent, removed)
      .then((r) => {
        setResult({ shape_notes: r.shape_notes, quality: r.quality });
        setCreate(false);
        setSaved(content);
        setRemoved([]);
        setFiles((fs) => fs.map((f) => ({ path: f.path, status: "saved", binary: f.binary ? { size: f.binary.size, preview: f.binary.base64 ?? f.binary.preview } : undefined })));
        app.notify(`${app.t("library.saved")} ${r.name}`);
        onSaved(r.name);
      })
      .catch((e) => app.notify(friendly(e, app.t)))
      .finally(() => setBusy(false));
  };
  const close = () => (changes > 0 ? setLeaving(true) : onClose());

  const editorText = active === SKILL ? body : texts[active];
  const unmentioned = (p: string) => !body.includes(p);
  return (
    <Overlay open onClose={close} kind="dialog" top="pt-[3vh]">
      <div
        role="dialog"
        aria-modal="true"
        aria-label={create ? app.t("library.newSkill") : `${app.t("library.edit")}: ${name}`}
        className="card flex h-[92vh] w-[min(1240px,95vw)] flex-col gap-3 p-4"
        onKeyDown={(e) => e.key === "Escape" && close()}
      >
        {/* Header */}
        <div className="flex items-center gap-2">
          <Pencil className="h-4 w-4 text-accent" />
          <span className="text-sm font-semibold">{create ? app.t("library.newSkill") : `${app.t("library.edit")}: ${name}`}</span>
          <span className="ml-2 text-[11px] text-subtle">{app.t("library.editHint")}</span>
          {original !== undefined && (
            <button className={cn("btn ml-auto h-7 px-2 text-[11px]", compare && "border-accent text-accent")} onClick={() => (setCompare((c) => !c), setActive(SKILL))}>
              <GitCompare className="h-3.5 w-3.5" /> {app.t("library.compare")}
            </button>
          )}
          <button className={cn("btn px-2", original === undefined && "ml-auto")} onClick={close} title={app.t("common.cancel")}>
            <X className="h-3.5 w-3.5" />
          </button>
        </div>

        {/* What the skill is (its frontmatter) */}
        <div className="grid gap-3 md:grid-cols-[240px_1fr_280px]">
          <label className="space-y-1">
            <span className="label">{app.t("library.name")}</span>
            <input className={cn("input w-full font-mono", create && name && !nameOk && "border-rose-500")} value={name} disabled={!create} placeholder="my-skill" onChange={(e) => setName(e.target.value.toLowerCase())} />
            {create && name && !nameOk && <span className="block text-[11px] text-rose-600 dark:text-rose-400">{app.t("library.badName")}</span>}
          </label>
          <label className="space-y-1">
            <span className="label">{app.t("library.description")}</span>
            <input className="input w-full" value={description} placeholder={app.t("library.descriptionHint")} onChange={(e) => setDescription(e.target.value)} />
          </label>
          <label className="space-y-1">
            <span className="label">{app.t("library.tags")}</span>
            <input className="input w-full" value={tags} placeholder="coding testing" onChange={(e) => setTags(e.target.value)} />
          </label>
        </div>

        <div className="flex min-h-0 flex-1 gap-3">
          {/* The skill's files */}
          <div className="flex w-64 shrink-0 flex-col overflow-y-auto rounded-lg border border-line bg-elevated/30 p-2">
            <div className="label px-1.5 pb-1">{app.t("library.instructions")}</div>
            <button className={cn("flex items-center gap-2 rounded-md px-2 py-1.5 text-left text-xs", active === SKILL ? "bg-accent/15 text-accent" : "text-muted hover:bg-elevated")} onClick={() => open(SKILL)}>
              <FileText className="h-3.5 w-3.5 shrink-0" />
              <Mono>{SKILL}</Mono>
              {content !== saved && <span className="ml-auto h-1.5 w-1.5 rounded-full bg-amber-500" title={app.t("library.unsaved")} />}
            </button>
            {DIRS.map((dir) => {
              const inDir = files.filter((f) => f.path.startsWith(`${dir}/`));
              return (
                <div key={dir} className="mt-3">
                  <div className="flex items-center gap-1 px-1.5 pb-1">
                    <span className="label">{app.t(`library.dir.${dir}`)}</span>
                    <button className="ml-auto rounded p-1 text-subtle hover:bg-elevated hover:text-accent" title={app.t("library.newFile")} onClick={() => setAdding({ dir, name: "" })}>
                      <Plus className="h-3.5 w-3.5" />
                    </button>
                    <button
                      className="rounded p-1 text-subtle hover:bg-elevated hover:text-accent"
                      title={app.t("library.uploadFile")}
                      onClick={() => {
                        uploadDir.current = dir;
                        upload.current?.click();
                      }}
                    >
                      <FileUp className="h-3.5 w-3.5" />
                    </button>
                  </div>
                  {inDir.map((f) => {
                    const Icon = fileIcon(f.path);
                    return (
                      <div key={f.path} className={cn("group flex items-center gap-2 rounded-md px-2 py-1.5 text-xs", active === f.path ? "bg-accent/15 text-accent" : "text-muted hover:bg-elevated")}>
                        <button className="flex min-w-0 flex-1 items-center gap-2 text-left" onClick={() => open(f.path)} title={f.path}>
                          <Icon className="h-3.5 w-3.5 shrink-0" />
                          <span className="truncate">{f.path.slice(dir.length + 1)}</span>
                        </button>
                        {unmentioned(f.path) && (
                          <span title={app.t("library.unmentioned")}>
                            <AlertTriangle className="h-3 w-3 shrink-0 text-amber-500" />
                          </span>
                        )}
                        {f.status !== "saved" && <span className="h-1.5 w-1.5 shrink-0 rounded-full bg-amber-500" title={app.t("library.unsaved")} />}
                        <button className="hidden rounded p-0.5 text-subtle hover:text-rose-500 group-hover:block" title={app.t("common.delete")} onClick={() => remove(f.path)}>
                          <Trash2 className="h-3 w-3" />
                        </button>
                      </div>
                    );
                  })}
                  {adding?.dir === dir && (
                    <div className="flex items-center gap-1 px-1 py-1">
                      <span className="font-mono text-[11px] text-subtle">{dir}/</span>
                      <input
                        autoFocus
                        className="input h-7 min-w-0 flex-1 px-2 font-mono text-[11px]"
                        value={adding.name}
                        placeholder={dir === "scripts" ? "check.sh" : dir === "assets" ? "diagram.png" : "api.md"}
                        onChange={(e) => setAdding({ dir, name: e.target.value })}
                        onKeyDown={(e) => {
                          if (e.key === "Enter") addFile();
                          if (e.key === "Escape") {
                            e.stopPropagation();
                            setAdding(null);
                          }
                        }}
                        onBlur={() => (adding.name.trim() ? addFile() : setAdding(null))}
                      />
                    </div>
                  )}
                  {!inDir.length && adding?.dir !== dir && <div className="px-2 py-1 text-[11px] text-subtle">{app.t(`library.dirEmpty.${dir}`)}</div>}
                </div>
              );
            })}
            <div className="mt-auto px-1.5 pt-3 text-[11px] leading-relaxed text-subtle">{app.t("library.filesHint")}</div>
            <input ref={upload} type="file" multiple className="hidden" onChange={(e) => onUpload(e.target.files)} />
          </div>

          {/* The open file */}
          <div className="flex min-w-0 flex-1 flex-col gap-2">
            <div className="flex items-center gap-2">
              <Mono className="truncate text-subtle">{`${name || "…"}/${active}`}</Mono>
              {activeFile && unmentioned(active) && <span className="chip border-amber-500/40 text-amber-600 dark:text-amber-400">{app.t("library.unmentioned")}</span>}
            </div>
            {activeFile?.binary ? (
              <div className="flex min-h-0 flex-1 flex-col items-center justify-center gap-3 rounded-lg border border-dashed border-line p-4 text-xs text-subtle">
                {imageType(active) && (activeFile.binary.base64 ?? activeFile.binary.preview) ? (
                  <img alt={active} className="max-h-[70%] max-w-full rounded border border-line object-contain" src={`data:${imageType(active)};base64,${activeFile.binary.base64 ?? activeFile.binary.preview}`} />
                ) : (
                  <FileImage className="h-6 w-6" />
                )}
                <span>
                  {app.t("library.binary")} · {kb(activeFile.binary.size)}
                </span>
              </div>
            ) : editorText === undefined ? (
              <div className="flex flex-1 items-center justify-center rounded-lg border border-line">
                <Loader2 className="h-5 w-5 animate-spin text-subtle" />
              </div>
            ) : compare && original !== undefined && active === SKILL ? (
              <Suspense fallback={<div className="flex flex-1 items-center justify-center"><Loader2 className="h-5 w-5 animate-spin text-subtle" /></div>}>
                <div className="min-h-0 flex-1 overflow-hidden rounded-lg border border-line">
                  <MonacoDiff path="SKILL.md" original={original} modified={content} />
                </div>
              </Suspense>
            ) : (
              <Suspense fallback={<div className="flex flex-1 items-center justify-center"><Loader2 className="h-5 w-5 animate-spin text-subtle" /></div>}>
                <MonacoEditor key={active} value={editorText} language={active === SKILL ? "markdown" : language(active)} onChange={edit} className="min-h-0 flex-1" />
              </Suspense>
            )}
            {card !== undefined && (
              <div className="max-h-48 overflow-y-auto rounded-lg border border-line bg-elevated/40 p-2.5">
                <ScorecardView card={card} />
              </div>
            )}
            {result && (
              <div className="max-h-32 space-y-1 overflow-y-auto rounded-lg border border-line bg-elevated/40 p-2.5 text-[11px]">
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

        {/* Save */}
        <div className="flex items-center gap-2 border-t border-line pt-3">
          <span className={cn("mr-auto text-[11px]", changes ? "text-amber-600 dark:text-amber-400" : "text-subtle")}>
            {changes ? `${changes} ${app.t("library.changesUnsaved")}` : create ? app.t("library.notSavedYet") : app.t("library.allSaved")}
          </span>
          <button className="btn" onClick={close}>
            {app.t("common.cancel")}
          </button>
          <button className="btn" disabled={busy} onClick={evaluate} title={app.t("library.evalHint")}>
            <Sparkles className="h-3.5 w-3.5" /> {app.t("library.evaluate")}
          </button>
          <button className="btn-primary" disabled={busy || !nameOk || (!changes && !create)} onClick={save}>
            {busy ? <Loader2 className="h-3.5 w-3.5 animate-spin" /> : <Save className="h-3.5 w-3.5" />} {busy ? app.t("library.saving") : app.t("common.save")}
          </button>
        </div>
      </div>
      <ConfirmModal open={leaving} title={app.t("library.discardTitle")} note={app.t("library.discardNote")} confirm={app.t("library.discard")} onClose={() => setLeaving(false)} onConfirm={onClose} />
    </Overlay>
  );
}
