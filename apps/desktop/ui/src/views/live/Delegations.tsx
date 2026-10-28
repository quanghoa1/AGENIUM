import { ChevronDown, ChevronRight, Eye, GitBranch } from "lucide-react";
import { useEffect, useState } from "react";
import { ProgressBar } from "../../components/motion";
import { Avatar, Mono, TicketStatusBadge } from "../../components/ui";
import { api } from "../../lib/api";
import { useApp } from "../../lib/store";
import type { Column, LiveData, TicketStatus } from "../../lib/types";

const PROGRESS: Record<TicketStatus, number> = { New: 0.05, Assigned: 0.15, InProgress: 0.45, Pending: 0.55, Reopened: 0.35, Resolved: 0.85, Closed: 1, Cancelled: 0 };

/** The Lead's delegations: every ticket of the project with its worker, progress and "inspect". A one-line bar
 *  over the Lead's chat; opened, the list covers the chat until it is folded again (or Esc). */
export function Delegations({ project, data, onWatch }: { project: string; data: LiveData; onWatch: (id: string) => void }) {
  const app = useApp();
  const [cols, setCols] = useState<Column[]>([]);
  const [open, setOpen] = useState(false);
  useEffect(() => {
    api.board(project).then((c) => setCols(c ?? []));
  }, [project, app.tick]);
  useEffect(() => {
    if (!open) return;
    const onKey = (e: KeyboardEvent) => e.key === "Escape" && setOpen(false);
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [open]);
  const byName = new Map(data.agents.map((a) => [a.agent.name, a]));
  const cards = cols.flatMap((c) => c.cards.map((card) => ({ ...card, status: c.status }))).filter((c) => c.assignee && c.status !== "Cancelled");
  if (!cards.length) return null;
  const working = cards.filter((c) => c.status === "InProgress" || c.status === "Pending" || c.status === "Resolved").length;
  const closed = cards.filter((c) => c.status === "Closed").length;
  return (
    <>
      <button
        className="flex h-10 w-full shrink-0 items-center gap-2 border-t border-line bg-purple-500/5 px-4 text-left text-[11px] hover:bg-purple-500/10"
        onClick={() => setOpen(!open)}
        aria-expanded={open}
      >
        {open ? <ChevronDown className="h-3.5 w-3.5 text-purple-500" /> : <ChevronRight className="h-3.5 w-3.5 text-purple-500" />}
        <GitBranch className="h-3.5 w-3.5 text-purple-500" />
        <span className="font-semibold text-purple-600 dark:text-purple-300">{app.t("live.delegations")}</span>
        <span className="text-subtle">
          · {cards.length} ticket · {working} {app.t("live.delegWorking")} · {closed} {app.t("live.delegClosed")}
        </span>
      </button>
      {open && (
        <div className="absolute inset-x-0 bottom-0 top-10 z-20 space-y-1.5 overflow-y-auto border-t border-purple-500/30 bg-surface/95 p-3 backdrop-blur-sm">
          {cards.map((c) => {
            const w = c.assignee ? byName.get(c.assignee) : undefined;
            return (
              <div key={c.key} className="flex items-center gap-2 rounded-lg border border-line bg-surface p-2">
                <Avatar name={c.assignee ?? "?"} className="h-6 w-6 text-[9px]" />
                <div className="min-w-0 flex-1">
                  <div className="flex items-center gap-1.5">
                    <Mono className="text-cyan-600 dark:text-cyan-400">{c.key}</Mono>
                    <span className="truncate text-[11px] font-medium">{c.title}</span>
                  </div>
                  <div className="mt-1 flex items-center gap-2">
                    <div className="h-1 flex-1 overflow-hidden rounded-full bg-line">
                      <ProgressBar value={PROGRESS[c.status]} className={c.status === "Pending" ? "bg-amber-500" : c.status === "Reopened" ? "bg-rose-500" : undefined} />
                    </div>
                    <span className="text-[11px] text-subtle">@{c.assignee}</span>
                    <TicketStatusBadge status={c.status} />
                  </div>
                </div>
                {w && (
                  <button className="btn shrink-0 px-2 py-1 text-[11px]" onClick={() => onWatch(w.agent.id)}>
                    <Eye className="h-3 w-3" /> {app.t("live.watch")}
                  </button>
                )}
              </div>
            );
          })}
        </div>
      )}
    </>
  );
}
