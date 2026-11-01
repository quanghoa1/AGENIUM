// How full a run's context is (ADR-155): the tokens the model read at the run's last request against its context
// window. From 60 % old tool results are cleared, from 85 % the older conversation is compacted into notes, and a
// conversation resumed on a model with a smaller window is folded to fit before the run starts.
import { useEffect, useState } from "react";
import { api } from "../lib/api";
import { useApp, useLive } from "../lib/store";
import type { Usage } from "../lib/types";
import { Mono, cn, tokens } from "./ui";

export function ContextGauge({ usage, className, note }: { usage?: Usage | null; className?: string; note?: string }) {
  const app = useApp();
  const used = usage?.context_used ?? 0;
  if (!used) return null;
  const window = usage?.context_window ?? 0;
  // Over 100 % after a change to a smaller model, until the next run folds the conversation; the bar stops full.
  const pct = window > 0 ? Math.round((used / window) * 100) : null;
  const tone = pct == null ? "bg-blue-500" : pct >= 85 ? "bg-rose-500" : pct >= 60 ? "bg-amber-500" : "bg-emerald-500";
  return (
    <span className={cn("inline-flex items-center gap-1.5 text-[11px]", className)} title={app.t("context.hint")}>
      <span className="label">{app.t("context.label")}</span>
      {pct != null && (
        <span className="h-1.5 w-16 overflow-hidden rounded-full bg-elevated ring-1 ring-line">
          <span className={cn("block h-full rounded-full", tone)} style={{ width: `${Math.min(100, Math.max(pct, 2))}%` }} />
        </span>
      )}
      <Mono className="text-subtle">
        {tokens(used)}
        {window > 0 ? ` / ${tokens(window)}` : ""}
        {pct != null ? ` · ${pct}%` : ""}
      </Mono>
      {note && <span className="text-[11px] font-medium text-rose-600 dark:text-rose-300">{note}</span>}
    </span>
  );
}

/** The agent's context against the window of the model it has now (the chat page's Lead): after a change to a
 *  smaller model the last run's context shows against the new window, and that the next run folds it to fit. Read
 *  again when one of its runs ends or the app refreshes (a model saved). */
export function AgentContext({ agent }: { agent?: string | null }) {
  const app = useApp();
  const live = useLive();
  const [usage, setUsage] = useState<Usage | null>(null);
  const [over, setOver] = useState(false);
  const running = agent ? Object.values(live).filter((l) => l.agent === agent && !l.done).length : 0;
  const ended = agent ? Object.values(live).filter((l) => l.agent === agent && l.done).length : 0;
  useEffect(() => {
    if (!agent) return;
    const read = () =>
      api
        .agentContext(agent)
        .then((c) => {
          setUsage({ tokens_in: 0, tokens_out: 0, cache_read: 0, cache_write: 0, estimated_cost_usd: 0, elapsed_ms: 0, context_used: c.used, context_window: c.window });
          setOver(c.over);
        })
        .catch(() => undefined);
    read();
    // While a run goes on, its context is read again every few seconds.
    if (running === 0) return;
    const t = window.setInterval(read, 4000);
    return () => window.clearInterval(t);
  }, [agent, running, ended, app.tick]);
  return <ContextGauge usage={usage} note={over ? app.t("context.foldNext") : undefined} />;
}
