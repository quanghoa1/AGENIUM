import { AlertTriangle, FileUp } from "lucide-react";
import { useEffect, useState } from "react";
import { Modal } from "../../components/motion";
import { cn } from "../../components/ui";
import { api, type FoundSkill } from "../../lib/api";
import { useApp } from "../../lib/store";

/** The skills a chosen folder holds: tick the ones to import (those that can be, and are not in the library yet, start
 *  ticked; one of a name the library has is imported only with "Replace"). */
export function SkillPicker({
  found,
  replace,
  onClose,
  onDone,
}: {
  found: FoundSkill[] | null;
  replace: boolean;
  onClose: () => void;
  onDone: () => void;
}) {
  const app = useApp();
  const [picked, setPicked] = useState<Set<string>>(new Set());
  const [busy, setBusy] = useState(false);
  useEffect(() => {
    setPicked(new Set((found ?? []).filter((f) => !f.error && !f.exists).map((f) => f.path)));
  }, [found]);
  const can = (f: FoundSkill) => !f.error && (!f.exists || replace);
  const all = (found ?? []).filter(can);
  const go = () => {
    setBusy(true);
    api
      .importSkills([...picked], replace)
      .then((rs) => {
        const ok = rs.filter((r) => r.name).map((r) => r.name);
        const bad = rs.filter((r) => r.error).map((r) => r.error);
        if (ok.length) app.notify(app.t("library.importedMany").replace("{n}", String(ok.length)).replace("{x}", ok.join(", ")));
        for (const e of bad) app.notify(String(e));
        onDone();
      })
      .catch((e) => app.notify(String(e)))
      .finally(() => setBusy(false));
  };
  return (
    <Modal open={!!found} wide title={app.t("library.pickTitle").replace("{n}", String(found?.length ?? 0))} onClose={onClose}>
      <div className="space-y-2">
        <div className="flex items-center gap-2 text-xs">
          <label className="flex items-center gap-1.5">
            <input
              type="checkbox"
              checked={all.length > 0 && all.every((f) => picked.has(f.path))}
              onChange={(e) => setPicked(new Set(e.target.checked ? all.map((f) => f.path) : []))}
            />
            {app.t("library.pickAll")}
          </label>
          <span className="ml-auto text-subtle">{app.t("library.pickCount").replace("{n}", String(picked.size))}</span>
        </div>
        <div className="max-h-[50vh] space-y-1 overflow-y-auto pr-1">
          {(found ?? []).map((f) => (
            <label key={f.path} className={cn("flex items-start gap-2 rounded-lg border border-line p-2 text-xs", !can(f) && "opacity-60")} title={f.path}>
              <input
                type="checkbox"
                className="mt-0.5"
                disabled={!can(f)}
                checked={picked.has(f.path)}
                onChange={(e) => {
                  const n = new Set(picked);
                  if (e.target.checked) n.add(f.path);
                  else n.delete(f.path);
                  setPicked(n);
                }}
              />
              <span className="min-w-0 flex-1">
                <span className="font-semibold">{f.name}</span>
                {f.exists && <span className="chip ml-2 border-amber-500/40 text-amber-600 dark:text-amber-300">{app.t(replace ? "library.pickReplaces" : "library.pickExists")}</span>}
                <span className="block truncate text-subtle">{f.description}</span>
                {f.error && (
                  <span className="mt-0.5 flex items-center gap-1 text-red-600 dark:text-red-400">
                    <AlertTriangle className="h-3 w-3 shrink-0" /> {f.error}
                  </span>
                )}
              </span>
            </label>
          ))}
        </div>
        <div className="flex justify-end gap-2 pt-1">
          <button className="btn" onClick={onClose}>
            {app.t("common.cancel")}
          </button>
          <button className="btn-primary" disabled={!picked.size || busy} onClick={go}>
            <FileUp className="h-3.5 w-3.5" /> {app.t("library.pickImport").replace("{n}", String(picked.size))}
          </button>
        </div>
      </div>
    </Modal>
  );
}
