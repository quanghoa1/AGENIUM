import { ArrowRight, CheckCircle2, FileText, MessageSquare } from "lucide-react";
import { useEffect, useState } from "react";
import { AgentStatusBadge, Avatar, Empty, Metric, Mono, OutcomeBadge, PageHeader, RuntimeBadge, Section, ago, tokens } from "../components/ui";
import { api } from "../lib/api";
import { useApp } from "../lib/store";
import type { Overview } from "../lib/types";
import { AnimatePresence } from "motion/react";
import { CountUp, ListItem, ProgressBar } from "../components/motion";
import { CommandRow, NeedRow, needKey } from "./Needs";
import { GettingStarted } from "../components/GettingStarted";

export function OverviewView() {
  const app = useApp();
  const [o, setO] = useState<Overview | null>(null);
  useEffect(() => {
    api.overview().then((x) => {
      setO(x);
      // The backend's list is the truth: answered or expired approvals leave it.
      app.setApprovals(x.command_approvals);
    });
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [app.tick]);
  if (!o) return <div className="text-xs text-subtle">{app.t("common.loading")}</div>;
  const working = o.agents.filter((a) => a.status === "Working").length;
  const blocked = o.agents.filter((a) => a.status === "QuotaExhausted" || a.status === "Offline").length;
  const needCount = o.needs.length + app.approvals.length;
  return (
    <div className="space-y-5">
      <PageHeader title={app.t("overview.title")} sub={app.t("overview.sub")}>
        <button
          className="btn"
          disabled={!app.project}
          title={app.project ? app.t("overview.reportHint") : app.t("overview.reportNeedsProject")}
          onClick={() =>
            app.project &&
            api
              .projectReport(app.project)
              .then((md) => navigator.clipboard.writeText(String(md)))
              .then(() => app.notify(app.t("overview.reportCopied")))
              .catch((e) => app.notify(String(e)))
          }
        >
          <FileText className="h-3.5 w-3.5" /> {app.t("overview.report")}
        </button>
        <button className="btn-primary" onClick={() => app.go("chat")}>
          <MessageSquare className="h-3.5 w-3.5" /> {app.t("top.talk")}
        </button>
      </PageHeader>

      {/* Until a team has been talked to, the way there. */}
      {!app.projects.some((p) => p.team && (p.tickets > 0 || (p.run_state && p.run_state !== "Agreeing"))) && <GettingStarted />}

      <div className="grid grid-cols-2 gap-3 lg:grid-cols-5">
        <Metric label={app.t("overview.agents")} value={<CountUp value={o.agents.length} />} sub={`${working} ${app.t("overview.working")}${blocked ? ` · ${blocked} ${app.t("overview.blocked")}` : ""}`} tone={blocked ? "rose" : "neutral"} onClick={() => app.go("agents")} />
        <Metric label={app.t("overview.live")} value={<CountUp value={o.live_runs.length} />} tone="blue" onClick={() => app.go("runs")} />
        <Metric label={app.t("overview.needs")} value={<CountUp value={needCount} />} tone={needCount ? "amber" : "emerald"} onClick={() => app.go("needs")} />
        <Metric label={app.t("overview.tokens")} value={tokens(o.usage_today.tokens_in + o.usage_today.tokens_out)} sub={`${o.runs_today} ${app.t(o.runs_today === 1 ? "overview.run" : "overview.runs")}`} />
        <Metric label={app.t("overview.cost")} value={`$${o.usage_today.estimated_cost_usd.toFixed(3)}`} sub={app.t("overview.costNote")} />
      </div>

      <div className="grid gap-5 xl:grid-cols-5">
        <Section
          className="xl:col-span-3"
          title={app.t("overview.needs")}
          right={
            <button className="btn" onClick={() => app.go("needs")}>
              {app.t("common.open")} <ArrowRight className="h-3 w-3" />
            </button>
          }
        >
          <div className="space-y-2">
            {app.approvals.slice(0, 2).map((a) => (
              <CommandRow key={a.id} a={a} />
            ))}
            <AnimatePresence initial={false} mode="popLayout">
              {o.needs.slice(0, 5).map((n) => (
                <ListItem key={needKey(n.item)} id={needKey(n.item)}>
                  <NeedRow n={n.item} projectName={n.project_name} compact />
                </ListItem>
              ))}
            </AnimatePresence>
            {needCount === 0 && (
              <div className="flex items-center gap-2 text-xs text-muted">
                <CheckCircle2 className="h-4 w-4 text-emerald-500" /> {app.t("needs.empty")}
              </div>
            )}
          </div>
        </Section>

        <Section className="xl:col-span-2" title={app.t("overview.projects")}>
          <div className="space-y-2">
            {app.projects.map((p) => (
              <button key={p.id} onClick={() => (app.setProject(p.id), app.go("tickets"))} className="press w-full rounded-lg border border-line bg-elevated/50 p-3 text-left hover:border-accent/40 hover:shadow-md">
                <div className="flex items-center justify-between">
                  <span className="text-xs font-semibold">
                    <Mono className="text-cyan-600 dark:text-cyan-400">{p.key_prefix}</Mono> {p.name}
                  </span>
                  <span className="text-[11px] text-subtle">{p.team ? `${p.team.name}${p.team.solo ? "" : ` · ${p.team.members}`}` : app.t("overview.noTeam")}</span>
                </div>
                <div className="mt-2 h-1.5 overflow-hidden rounded-full bg-background">
                  <ProgressBar value={p.plan_items ? p.plan_done / p.plan_items : 0} />
                </div>
                <div className="mt-1.5 flex justify-between text-[11px] text-subtle">
                  <span>
                    plan {p.plan_done}/{p.plan_items} · {p.tickets} {app.t("nav.tickets").toLowerCase()}
                  </span>
                  <span className="flex gap-3">
                    {p.inbox > 0 && <span className="text-cyan-600 dark:text-cyan-400">{p.inbox} {app.t("overview.inbox")}</span>}
                    {p.needs_you > 0 && <span className="text-amber-600 dark:text-amber-400">{p.needs_you} {app.t("overview.needYou")}</span>}
                  </span>
                </div>
              </button>
            ))}
            {app.projects.length === 0 && <Empty>{app.t("common.none")}</Empty>}
          </div>
        </Section>
      </div>

      <Section title={app.t("overview.matrix")}>
        <div className="divide-y divide-line">
          {o.agents.map((a) => (
            <div key={a.id} className="flex items-center gap-3 py-2.5">
              <Avatar name={a.name} />
              <div className="w-44 min-w-0">
                <div className="truncate text-xs font-semibold">{a.name}</div>
                <div className="truncate text-[11px] text-subtle">{a.role}</div>
              </div>
              <div className="w-36">
                <AgentStatusBadge status={a.status} />
              </div>
              <div className="min-w-0 flex-1 truncate text-xs text-muted">
                {a.current ? (
                  <button className="hover:text-accent" onClick={() => app.go("ticket", a.current!.ticket)}>
                    <Mono className="text-cyan-600 dark:text-cyan-400">{a.current.ticket}</Mono> {a.current.title}
                  </button>
                ) : (
                  <span className="text-subtle">–</span>
                )}
              </div>
              <RuntimeBadge backend={a.backend} model={a.model} />
            </div>
          ))}
          {o.agents.length === 0 && <Empty>{app.t("common.none")}</Empty>}
        </div>
      </Section>

      <Section title={app.t("overview.recent")}>
        <table className="w-full text-xs">
          <thead>
            <tr className="label border-b border-line text-left">
              <th className="py-2 pr-3">{app.t("runs.outcome")}</th>
              <th className="pr-3">{app.t("runs.agent")}</th>
              <th className="pr-3">{app.t("common.model")}</th>
              <th className="pr-3">{app.t("runs.steps")}</th>
              <th className="pr-3">{app.t("ticket.tokens")}</th>
              <th className="text-right">{app.t("runs.started")}</th>
            </tr>
          </thead>
          <tbody className="divide-y divide-line">
            {o.recent_runs.map((r) => (
              <tr key={r.id} className="cursor-pointer hover:bg-elevated/60" onClick={() => app.go("run", r.id)}>
                <td className="py-2 pr-3">
                  <OutcomeBadge outcome={r.outcome} />
                </td>
                <td className="pr-3 font-medium">{r.agent_name}</td>
                <td className="pr-3 text-muted">{r.model}</td>
                <td className="pr-3 font-mono text-subtle">{r.steps}</td>
                <td className="pr-3 font-mono text-subtle">{tokens(r.usage.tokens_in + r.usage.tokens_out)}</td>
                <td className="text-right font-mono text-subtle">{ago(r.started_at)}</td>
              </tr>
            ))}
          </tbody>
        </table>
        {o.recent_runs.length === 0 && <Empty>{app.t("common.none")}</Empty>}
      </Section>
    </div>
  );
}
