// The earlier chats of one helper (ADR-152): each a session kept in the root, reopened where it stopped.
import { History, Loader2, Trash2 } from "lucide-react";
import { useEffect, useState } from "react";
import { api, type HelperSession } from "../lib/api";
import { friendly } from "../lib/errors";
import { useApp } from "../lib/store";
import { ago, cn } from "./ui";

export function HelperHistory({ kind, current, onOpen }: { kind: "design" | "skill"; current: string | null; onOpen: (s: HelperSession) => void }) {
  const app = useApp();
  const [list, setList] = useState<HelperSession[] | null>(null);
  const [cost, setCost] = useState<Record<string, number>>({});
  const load = () =>
    Promise.all([api.sessions(kind), api.helperCalls(2000).catch(() => [])])
      .then(([s, calls]) => {
        setList(s);
        const c: Record<string, number> = {};
        for (const h of calls as { session?: string | null; usage: { estimated_cost_usd: number } }[]) {
          if (h.session) c[h.session] = (c[h.session] ?? 0) + h.usage.estimated_cost_usd;
        }
        setCost(c);
      })
      .catch((e) => app.notify(friendly(e, app.t)));
  useEffect(() => {
    load();
  }, [kind]);
  if (!list) return <Loader2 className="m-4 h-4 w-4 animate-spin text-subtle" />;
  if (!list.length) return <div className="p-3 text-xs text-subtle">{app.t("helper.noHistory")}</div>;
  return (
    <div className="space-y-1.5">
      {list.map((s) => (
        <div key={s.id} className={cn("group flex items-center gap-2 rounded-lg border border-line p-2 text-xs", s.id === current && "border-accent bg-accent/5")}>
          <History className="h-3.5 w-3.5 shrink-0 text-subtle" />
          <button className="min-w-0 flex-1 text-left" onClick={() => api.session(s.id).then(onOpen, (e) => app.notify(friendly(e, app.t)))}>
            <div className="truncate font-medium">{s.title || app.t("helper.untitled")}</div>
            <div className="text-[11px] text-subtle">
              {ago(s.updated_at)} · {Math.ceil((s.turns?.length ?? 0) / 2)} {app.t("helper.turns")}
              {s.model?.model && ` · ${s.model.model}`}
              {cost[s.id] ? ` · $${cost[s.id].toFixed(4)}` : ""}
            </div>
          </button>
          <span className={cn("chip", s.status === "saved" ? "border-emerald-500/40 text-emerald-600 dark:text-emerald-400" : "border-line-strong text-muted")}>
            {s.status === "saved" ? `${app.t("helper.saved")}${s.result ? `: ${s.result}` : ""}` : app.t("helper.open")}
          </span>
          {s.id !== current && (
            <button className="hidden text-subtle hover:text-rose-500 group-hover:block" title={app.t("common.delete")} onClick={() => api.removeSession(s.id).then(load)}>
              <Trash2 className="h-3.5 w-3.5" />
            </button>
          )}
        </div>
      ))}
    </div>
  );
}
