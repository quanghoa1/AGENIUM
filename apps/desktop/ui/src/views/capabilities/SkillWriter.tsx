// "Write a skill with an agent" (ADR-151), beside writing one by hand: the user says what the skill should do and adds
// documents (text files: a guide, a checklist, a standard); a model of their choosing writes it to Agenium's skill
// contract. The draft opens in the skill editor, where the user evaluates and saves it like a skill they wrote.
import { AlertTriangle, ArrowRight, FileText, Paperclip, Trash2, Wand2 } from "lucide-react";
import { useRef, useState } from "react";
import { HelperChat } from "../../components/HelperChat";
import { Markdown } from "../../components/Markdown";
import { Mono } from "../../components/ui";
import { api, type NewSkill, type SkillDoc, type SkillWrite } from "../../lib/api";
import { useApp } from "../../lib/store";
import { SkillEditor } from "./SkillEditor";

/** A document is read up to this size; all of them are cut to 120 000 characters when sent. */
const MAX_DOC = 1024 * 1024;

export function SkillWriter({ seed, onClose, onSaved }: { seed?: NewSkill; onClose: () => void; onSaved: (name: string) => void }) {
  const app = useApp();
  const [docs, setDocs] = useState<SkillDoc[]>([]);
  const [write, setWrite] = useState<SkillWrite | null>(null);
  const [editing, setEditing] = useState(false);
  const pick = useRef<HTMLInputElement>(null);
  const d = write?.draft;
  const first = seed
    ? app
        .t("skillWriter.seedMessage")
        .replace("{name}", seed.name)
        .replace("{does}", seed.does.trim().replace(/\.+$/, "") || "–")
        // The designer often writes "Use when …" itself.
        .replace("{when}", seed.when.trim().replace(/^use (it )?when:?\s*/i, "").replace(/\.+$/, "") || "–")
    : undefined;
  const addDocs = async (list: FileList | null) => {
    for (const f of Array.from(list ?? [])) {
      if (f.size > MAX_DOC) {
        app.notify(`${f.name}: ${app.t("library.tooLarge")}`);
        continue;
      }
      try {
        const text = new TextDecoder("utf-8", { fatal: true }).decode(await f.arrayBuffer());
        setDocs((ds) => [...ds.filter((x) => x.name !== f.name), { name: f.name, text }]);
      } catch {
        app.notify(`${f.name}: ${app.t("skillWriter.textOnly")}`);
      }
    }
    if (pick.current) pick.current.value = "";
  };
  const docsBox = (
    <div className="rounded-lg border border-line p-2">
      <div className="flex items-center gap-2">
        <span className="label">{app.t("skillWriter.docs")}</span>
        <button className="btn ml-auto h-7 px-2 text-[11px]" onClick={() => pick.current?.click()}>
          <Paperclip className="h-3.5 w-3.5" /> {app.t("skillWriter.addDoc")}
        </button>
        <input ref={pick} type="file" multiple className="hidden" accept=".md,.txt,.markdown,.rst,.adoc,.html,.json,.yaml,.yml,.toml,.csv,.py,.js,.ts,.sh,.ps1,.sql,.xml" onChange={(e) => addDocs(e.target.files)} />
      </div>
      {docs.length === 0 ? (
        <div className="mt-1 text-[11px] text-subtle">{app.t("skillWriter.docsNote")}</div>
      ) : (
        <div className="mt-1.5 flex flex-wrap gap-1.5">
          {docs.map((x) => (
            <span key={x.name} className="chip gap-1 border-line-strong text-muted">
              <FileText className="h-3 w-3" /> {x.name} <span className="text-subtle">{Math.max(1, Math.round(x.text.length / 1000))}k</span>
              <button className="text-subtle hover:text-rose-500" title={app.t("common.delete")} onClick={() => setDocs((ds) => ds.filter((y) => y.name !== x.name))}>
                <Trash2 className="h-3 w-3" />
              </button>
            </span>
          ))}
        </div>
      )}
    </div>
  );
  return (
    <>
      <HelperChat<SkillWrite>
        title={app.t("skillWriter.title")}
        intro={app.t("skillWriter.intro")}
        placeholder={app.t("skillWriter.placeholder")}
        first={first}
        ask={(m, turns) => api.writeSkill(m, turns, docs, seed ?? null)}
        onAnswer={(a) => a.draft && setWrite(a)}
        extra={docsBox}
        onClose={onClose}
      >
        {!d ? (
          <div className="flex h-full flex-col items-center justify-center gap-2 text-center text-xs text-subtle">
            <Wand2 className="h-6 w-6" />
            <div className="max-w-md leading-relaxed">{app.t("skillWriter.empty")}</div>
          </div>
        ) : (
          <div className="space-y-3">
            <div className="flex items-start gap-3">
              <div className="min-w-0 flex-1">
                <div className="flex items-center gap-2">
                  <Mono className="text-sm font-semibold">{d.name}</Mono>
                  {d.tags.map((t) => (
                    <span key={t} className="chip border-indigo-500/30 text-indigo-600 dark:text-indigo-300">
                      {t}
                    </span>
                  ))}
                </div>
                <div className="mt-1 text-xs text-muted">{d.description}</div>
              </div>
              <button className="btn-primary shrink-0" onClick={() => setEditing(true)}>
                {app.t("skillWriter.open")} <ArrowRight className="h-3.5 w-3.5" />
              </button>
            </div>
            {d.exists && (
              <div className="flex items-start gap-1.5 text-[11px] text-amber-600 dark:text-amber-400">
                <AlertTriangle className="mt-0.5 h-3 w-3 shrink-0" /> {app.t("skillWriter.exists")}
              </div>
            )}
            {d.notes.map((n, i) => (
              <div key={i} className="flex items-start gap-1.5 text-[11px] text-amber-600 dark:text-amber-400">
                <AlertTriangle className="mt-0.5 h-3 w-3 shrink-0" /> {n}
              </div>
            ))}
            {d.files.length > 0 && (
              <div className="font-mono text-[11px] text-muted">
                {app.t("skillWriter.files")}: {d.files.map((f) => f.path).join(" · ")}
              </div>
            )}
            <div className="rounded-lg border border-line bg-elevated/40 p-3 text-xs">
              <Markdown text={d.body} />
            </div>
          </div>
        )}
      </HelperChat>
      {editing && d && (
        <SkillEditor
          initial={d.skill_md}
          create
          name0=""
          files0={[]}
          newFiles0={d.files}
          onClose={() => setEditing(false)}
          onSaved={(name) => name && onSaved(name)}
        />
      )}
    </>
  );
}
