import { AlertTriangle, Check, CheckCircle2, FileText, GitMerge, HelpCircle, KeyRound, ShieldAlert, Terminal, X } from "lucide-react";
import { useEffect, useState, type ReactNode } from "react";
import { AnimatePresence } from "motion/react";
import { ListItem } from "../components/motion";
import { Empty, Mono, PageHeader, Section, ago, cn } from "../components/ui";
import { api } from "../lib/api";
import type { Key } from "../lib/i18n";
import { useApp } from "../lib/store";
import type { CommandApproval, NeedItem, NeedKind } from "../lib/types";

const icon: Record<NeedKind, ReactNode> = {
  AgentBlocked: <AlertTriangle className="h-4 w-4 text-rose-500" />,
  Escalation: <ShieldAlert className="h-4 w-4 text-rose-500" />,
  SpecApproval: <FileText className="h-4 w-4 text-amber-500" />,
  PlanApproval: <FileText className="h-4 w-4 text-amber-500" />,
  Permission: <KeyRound className="h-4 w-4 text-amber-500" />,
  Question: <HelpCircle className="h-4 w-4 text-blue-500" />,
  MergeReview: <GitMerge className="h-4 w-4 text-purple-500" />,
};

/** Stable identity of a "needs you" item across refreshes (for list animations). */
export function needKey(n: NeedItem): string {
  return [n.project, n.kind, n.ticket ?? "", n.comment ?? "", n.version ?? "", n.agent ?? ""].join("|");
}

/** One "needs you" item with the action that resolves it. */
export function NeedRow({ n, projectName, compact }: { n: NeedItem; projectName?: string; compact?: boolean }) {
  const app = useApp();
  const [answer, setAnswer] = useState("");
  const [busy, setBusy] = useState(false);
  const act = (p: Promise<unknown>, msg: string) => {
    setBusy(true);
    p.then(() => (app.notify(msg), app.refresh()))
      .catch((e) => app.notify(String(e)))
      .finally(() => setBusy(false));
  };
  return (
    <div className="flex items-start gap-3 rounded-lg border border-line bg-elevated/50 p-3">
      <div className="mt-0.5">{icon[n.kind]}</div>
      <div className="min-w-0 flex-1">
        <div className="flex items-center gap-2">
          <span className="label">{app.t(`need.${n.kind}` as Key)}</span>
          {projectName && <Mono className="text-subtle">{projectName}</Mono>}
          {n.at && <Mono className="text-subtle">{ago(n.at)}</Mono>}
        </div>
        <div className="mt-1 text-xs">
          {n.kind === "SpecApproval" || n.kind === "PlanApproval"
            ? app.t(n.kind === "SpecApproval" ? "need.specText" : "need.planText").replace("{v}", String(n.version ?? ""))
            : n.text}
        </div>
        {!compact && n.kind === "Question" && n.ticket && n.comment && (
          <div className="mt-2 flex gap-2">
            <input className="input" placeholder={app.t("needs.answer")} value={answer} onChange={(e) => setAnswer(e.target.value)} />
            <button disabled={busy || !answer.trim()} className="btn-primary" onClick={() => act(api.answer(n.project, n.ticket!, n.comment!, answer), app.t("common.done"))}>
              {app.t("common.send")}
            </button>
          </div>
        )}
      </div>
      <div className="flex shrink-0 flex-wrap justify-end gap-1.5">
        {(n.kind === "SpecApproval" || n.kind === "PlanApproval") && (
          <>
            <button className="btn" onClick={() => app.go("tickets", "plan")}>
              {app.t("common.open")}
            </button>
            <button disabled={busy} className="btn-primary" onClick={() => act(api.approve(n.project, n.kind === "SpecApproval" ? "spec" : "plan", String(n.version)), app.t("common.done"))}>
              <Check className="h-3.5 w-3.5" /> {app.t("common.approve")}
            </button>
          </>
        )}
        {n.kind === "Permission" && n.ticket && n.comment && (
          <>
            <button disabled={busy} className="btn-danger" onClick={() => act(api.decidePermission(n.project, n.ticket!, n.comment!, false), app.t("common.done"))}>
              <X className="h-3.5 w-3.5" />
            </button>
            <button disabled={busy} className="btn" onClick={() => act(api.decidePermission(n.project, n.ticket!, n.comment!, true, true), app.t("common.done"))}>
              {app.t("needs.always")}
            </button>
            <button disabled={busy} className="btn-primary" onClick={() => act(api.decidePermission(n.project, n.ticket!, n.comment!, true), app.t("common.done"))}>
              <Check className="h-3.5 w-3.5" /> {app.t("common.approve")}
            </button>
          </>
        )}
        {n.kind === "MergeReview" && n.ticket && (
          <button disabled={busy} className="btn-primary" onClick={() => act(api.approve(n.project, "merge", n.ticket!), app.t("common.done"))}>
            <GitMerge className="h-3.5 w-3.5" /> Merge
          </button>
        )}
        {n.ticket && (compact || n.kind !== "Question") && (
          <button className="btn" onClick={() => app.go("ticket", n.ticket!)}>
            {n.ticket}
          </button>
        )}
        {n.kind === "AgentBlocked" && (
          <button className="btn" onClick={() => app.go("agents")}>
            {app.t("nav.agents")}
          </button>
        )}
      </div>
    </div>
  );
}

export function CommandRow({ a }: { a: CommandApproval }) {
  const app = useApp();
  const decide = (d: "allow" | "always" | "deny") =>
    api.decideCommand(a.id, d).then(() => {
      app.setApprovals(app.approvals.filter((x) => x.id !== a.id));
      app.refresh();
    });
  return (
    <div className="flex items-start gap-3 rounded-lg border border-amber-500/30 bg-amber-500/5 p-3">
      <Terminal className="mt-0.5 h-4 w-4 text-amber-500" />
      <div className="min-w-0 flex-1">
        <div className="flex items-center gap-2">
          <span className="label">{a.agent}</span>
          {a.ticket && <Mono className="text-subtle">{a.ticket}</Mono>}
          <Mono className="text-subtle">{ago(a.at)}</Mono>
        </div>
        <pre className="mt-1 whitespace-pre-wrap break-all rounded-md bg-background p-2 font-mono text-[11px]">{a.command}</pre>
      </div>
      <div className="flex shrink-0 gap-1.5">
        <button className="btn-danger" onClick={() => decide("deny")}>
          <X className="h-3.5 w-3.5" />
        </button>
        <button className="btn" onClick={() => decide("always")}>
          {app.t("needs.always")}
        </button>
        <button className="btn-primary" onClick={() => decide("allow")}>
          <Check className="h-3.5 w-3.5" /> {app.t("needs.allow")}
        </button>
      </div>
    </div>
  );
}

export function NeedsView() {
  const app = useApp();
  const [items, setItems] = useState<{ project: string; items: NeedItem[] }[]>([]);
  useEffect(() => {
    Promise.all(app.projects.map((p) => api.needsYou(p.id).then((r) => ({ project: p.name, items: r.needs, cmds: r.command_approvals })))).then((rs) => {
      setItems(rs);
      const cmds = rs.flatMap((r) => r.cmds);
      if (cmds.length) app.setApprovals(cmds.filter((c, i) => cmds.findIndex((x) => x.id === c.id) === i));
    });
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [app.projects, app.tick]);
  const total = items.reduce((n, r) => n + r.items.length, 0) + app.approvals.length;
  return (
    <div className="space-y-4">
      <PageHeader title={app.t("nav.needs")} sub={`${total}`} />
      {app.approvals.length > 0 && (
        <Section title={app.t("needs.commands")}>
          <div className="space-y-2">
            <AnimatePresence initial={false} mode="popLayout">
              {app.approvals.map((a) => (
                <ListItem key={String(a.id)} id={String(a.id)}>
                  <CommandRow a={a} />
                </ListItem>
              ))}
            </AnimatePresence>
          </div>
        </Section>
      )}
      {items
        .filter((r) => r.items.length)
        .map((r) => (
          <Section key={r.project} title={r.project}>
            <div className="space-y-2">
              <AnimatePresence initial={false} mode="popLayout">
                {r.items.map((n) => (
                  <ListItem key={needKey(n)} id={needKey(n)}>
                    <NeedRow n={n} />
                  </ListItem>
                ))}
              </AnimatePresence>
            </div>
          </Section>
        ))}
      {total === 0 && (
        <div className={cn("card flex items-center gap-3 p-6 text-sm text-muted")}>
          <CheckCircle2 className="h-5 w-5 text-emerald-500" /> {app.t("needs.empty")}
        </div>
      )}
      {items.length === 0 && app.projects.length === 0 && <Empty>{app.t("common.none")}</Empty>}
    </div>
  );
}
