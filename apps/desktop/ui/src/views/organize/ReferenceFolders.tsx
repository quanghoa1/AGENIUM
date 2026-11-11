import { BookOpen, FolderPlus, X } from "lucide-react";
import { useState } from "react";
import { api } from "../../lib/api";
import { friendly } from "../../lib/errors";
import { useApp } from "../../lib/store";
import type { Reference } from "../../lib/types";

/** Folders the project's agents read but never change (ADR-159): an SDK, docs, another codebase. */
export function ReferenceFolders({ value, onChange }: { value: Reference[]; onChange: (v: Reference[]) => void }) {
  const app = useApp();
  const add = () =>
    api
      .pickFolder()
      .then((p) => p && !value.some((r) => r.path === p) && onChange([...value, { path: p, note: "" }]))
      .catch((e) => app.notify(friendly(e, app.t)));
  return (
    <div className="space-y-1.5">
      <div className="flex items-center gap-2">
        <span className="label" title={app.t("projects.refsHint")}>
          {app.t("projects.refs")}
        </span>
        <button className="btn ml-auto" title={app.t("projects.refsHint")} onClick={add}>
          <FolderPlus className="h-3.5 w-3.5" /> {app.t("projects.refsAdd")}
        </button>
      </div>
      {value.map((r, i) => (
        <div key={r.path} className="flex items-center gap-2 text-xs">
          <BookOpen className="h-3.5 w-3.5 shrink-0 text-subtle" />
          <span className="min-w-0 flex-1 truncate font-mono" title={r.path}>
            {r.path}
          </span>
          <input
            className="input w-56 py-1"
            value={r.note}
            placeholder={app.t("projects.refsNote")}
            onChange={(e) => onChange(value.map((x, j) => (j === i ? { ...x, note: e.target.value } : x)))}
          />
          <button className="btn px-1.5" aria-label={app.t("projects.refsRemove")} title={app.t("projects.refsRemove")} onClick={() => onChange(value.filter((_, j) => j !== i))}>
            <X className="h-3.5 w-3.5" />
          </button>
        </div>
      ))}
    </div>
  );
}

/** A project's reference folders on its card: each change is saved at once (a note when the field is left). */
export function ProjectReferences({ project, refs }: { project: string; refs: Reference[] }) {
  const app = useApp();
  const [draft, setDraft] = useState<Reference[] | null>(null);
  const value = draft ?? refs;
  const save = (v: Reference[]) =>
    api
      .setReferences(project, v)
      .then(() => (setDraft(null), app.refresh()))
      .catch((e) => app.notify(friendly(e, app.t)));
  return (
    <div
      className="mt-3"
      onBlur={(e) => draft && !e.currentTarget.contains(e.relatedTarget as Node | null) && save(draft)}
    >
      <ReferenceFolders
        value={value}
        // A path added or removed is saved now; a note being typed waits for the field to be left.
        onChange={(v) => (v.length !== value.length ? save(v) : setDraft(v))}
      />
    </div>
  );
}
