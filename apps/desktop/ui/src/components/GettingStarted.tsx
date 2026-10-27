import { ArrowRight, Check } from "lucide-react";
import { useEffect, useState } from "react";
import { api } from "../lib/api";
import type { Key } from "../lib/i18n";
import { useApp, type View } from "../lib/store";
import { PageHeader, cn } from "./ui";

/** The four steps from an empty root to a working team, each ticked once done. Shown on the overview until they
 *  are all done, and in place of an empty screen that needs a project (a review found "Nothing yet." alone there
 *  read as a broken page). */
export function GettingStarted({ className }: { className?: string }) {
  const app = useApp();
  const [agents, setAgents] = useState<number | null>(null);
  useEffect(() => {
    api.agents().then((a) => setAgents(a.length)).catch(() => setAgents(0));
  }, [app.tick]);
  const project = app.projects.length > 0;
  const team = app.projects.some((p) => p.team);
  const talked = app.projects.some((p) => p.team && (p.tickets > 0 || (p.run_state && p.run_state !== "Agreeing")));
  const steps: { done: boolean; title: Key; hint: Key; go: View }[] = [
    { done: project, title: "start.project", hint: "start.projectHint", go: "projects" },
    { done: (agents ?? 0) > 0 || team, title: "start.agent", hint: "start.agentHint", go: "agents" },
    { done: team, title: "start.team", hint: "start.teamHint", go: "teams" },
    { done: talked, title: "start.talk", hint: "start.talkHint", go: "chat" },
  ];
  const next = steps.findIndex((s) => !s.done);
  return (
    <div className={cn("card p-4", className)}>
      <div className="text-sm font-semibold">{app.t("start.title")}</div>
      <div className="mt-0.5 text-xs text-muted">{app.t("start.sub")}</div>
      <ol className="mt-3 space-y-2">
        {steps.map((s, i) => (
          <li key={s.title} className={cn("flex items-center gap-3 rounded-lg border p-2.5", i === next ? "border-accent/50 bg-blue-600/5" : "border-line")}>
            <span
              className={cn(
                "flex h-6 w-6 shrink-0 items-center justify-center rounded-full border font-mono text-[11px]",
                s.done ? "border-emerald-500/50 bg-emerald-500/15 text-emerald-600 dark:text-emerald-300" : "border-line-strong text-subtle",
              )}
            >
              {s.done ? <Check className="h-3.5 w-3.5" /> : i + 1}
            </span>
            <span className="min-w-0 flex-1">
              <span className={cn("block text-xs font-medium", s.done && "text-muted line-through decoration-line-strong")}>{app.t(s.title)}</span>
              <span className="block text-[11px] text-subtle">{app.t(s.hint)}</span>
            </span>
            {!s.done && (
              <button className={i === next ? "btn-primary" : "btn"} onClick={() => app.go(s.go)}>
                {app.t("start.go")} <ArrowRight className="h-3 w-3" />
              </button>
            )}
          </li>
        ))}
      </ol>
    </div>
  );
}

/** A screen that works on the selected project, when there is none (yet): its title, why it is empty, the steps. */
export function NeedsProject({ title }: { title: string }) {
  const app = useApp();
  return (
    <div className="mx-auto max-w-2xl">
      <PageHeader title={title} sub={app.t(app.projects.length ? "start.pickProject" : "start.noProject")} />
      <GettingStarted />
    </div>
  );
}
