import { FileText, X } from "lucide-react";
import { useState } from "react";
import { api } from "../lib/api";
import { friendly } from "../lib/errors";
import { useApp } from "../lib/store";
import type { ProjectSummary } from "../lib/types";
import { Markdown } from "./Markdown";
import { Overlay } from "./motion";

/** The project's latest handover report (ADR-147: written each time the project completes), read in the app: the
 *  requirements and how they are covered, per ticket the results, checks, reviews and commits, risks and follow-ups. */
export function HandoverReport({ p }: { p: ProjectSummary }) {
  const app = useApp();
  const [report, setReport] = useState<{ path: string; version: number; markdown: string } | null>(null);
  if (!p.handovers) return null;
  const open = () =>
    api
      .handover(p.id)
      .then((r) => (r ? setReport(r) : app.notify(app.t("handover.none"))))
      .catch((e) => app.notify(friendly(e, app.t)));
  return (
    <>
      <button className="btn" title={app.t("handover.hint")} onClick={open}>
        <FileText className="h-3.5 w-3.5" /> {app.t("handover.open")}
      </button>
      <Overlay open={!!report} onClose={() => setReport(null)} kind="dialog" top="pt-10">
        <div className="card flex max-h-[86vh] w-[980px] max-w-[94vw] flex-col p-5 shadow-2xl" onKeyDown={(e) => e.key === "Escape" && setReport(null)}>
          <div className="mb-2 flex items-center gap-2">
            <FileText className="h-4 w-4 text-subtle" />
            <div className="text-sm font-semibold">
              {app.t("handover.title").replace("{p}", p.name).replace("{v}", String(report?.version ?? ""))}
            </div>
            <button className="btn ml-auto" aria-label={app.t("common.close")} onClick={() => setReport(null)}>
              <X className="h-3.5 w-3.5" />
            </button>
          </div>
          <div className="mb-3 break-all font-mono text-[11px] text-subtle">{report?.path}</div>
          <div className="min-h-0 overflow-auto pr-2">{report && <Markdown text={report.markdown} />}</div>
        </div>
      </Overlay>
    </>
  );
}
