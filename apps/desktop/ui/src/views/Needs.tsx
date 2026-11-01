import { AlertTriangle, Ban, Check, CheckCircle2, FileText, GitMerge, HelpCircle, KeyRound, Lightbulb, Plug, RotateCcw, ShieldAlert, Terminal, UserPlus, X } from "lucide-react";
import { useEffect, useState, type ReactNode } from "react";
import { AnimatePresence } from "motion/react";
import { ListItem } from "../components/motion";
import { Mono, PageHeader, Section, ago, cn } from "../components/ui";
import { api } from "../lib/api";
import type { Key } from "../lib/i18n";
import { useApp } from "../lib/store";
import type { CommandApproval, NeedItem, NeedKind } from "../lib/types";
import { SkillProposalActions } from "./SkillProposalActions";

const icon: Record<NeedKind, ReactNode> = {
  AgentBlocked: <AlertTriangle className="h-4 w-4 text-rose-500" />,
  Escalation: <ShieldAlert className="h-4 w-4 text-rose-500" />,
  SpecApproval: <FileText className="h-4 w-4 text-amber-500" />,
  PlanApproval: <FileText className="h-4 w-4 text-amber-500" />,
  Permission: <KeyRound className="h-4 w-4 text-amber-500" />,
  Question: <HelpCircle className="h-4 w-4 text-blue-500" />,
  MergeReview: <GitMerge className="h-4 w-4 text-purple-500" />,
  RoleProposal: <UserPlus className="h-4 w-4 text-cyan-500" />,
  SkillProposal: <Lightbulb className="h-4 w-4 text-cyan-500" />,
  CancelledWork: <Ban className="h-4 w-4 text-amber-500" />,
};

/** The item in the interface's language: its template filled with its values (once, left to right), or its English. */
export function needText(n: NeedItem, t: (k: Key) => string): string {
  const key = `need.words.${n.words?.id}` as Key;
  const template = n.words ? t(key) : key;
  if (!n.words || template === key) return n.text;
  // A permission's kind (Network, Path, Command) is a word of the interface too.
  const word = (a: string) => {
    const k = `need.perm.${a}` as Key;
    return t(k) === k ? a : t(k);
  };
  const args = n.words.args.map((a, i) => (n.kind === "Permission" && i === 1 ? word(a) : a));
  return template.replace(/\{(\d+)\}/g, (whole, i) => args[Number(i)] ?? whole);
}

/** Stable identity of a "needs you" item across refreshes (for list animations). */
export function needKey(n: NeedItem): string {
  return [n.project, n.kind, n.ticket ?? "", n.comment ?? "", n.version ?? "", n.agent ?? "", n.proposal ?? ""].join("|");
}

/** One "needs you" item with the action that resolves it. */
export function NeedRow({ n, projectName, compact }: { n: NeedItem; projectName?: string; compact?: boolean }) {
  const app = useApp();
  const [answer, setAnswer] = useState("");
  const [busy, setBusy] = useState(false);
  const act = (p: Promise<unknown>, msg: string) => {
    setBusy(true);
    p.then((r) => (app.notify((r as { routed_to?: string } | null)?.routed_to === "lead" ? app.t("needs.toLead") : msg), app.refresh()))
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
          {needText(n, app.t)}
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
            {/* The item's own project first: the plan page shows the selected one, so Open showed another project's plan
                beside the Approve of this one (audit M5, 2026-10-31). */}
            <button className="btn" onClick={() => (app.setProject(n.project), app.go("tickets", "plan"))}>
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
        {n.kind === "RoleProposal" && n.proposal && (
          <>
            <button disabled={busy} className="btn-danger" title={app.t("need.declineRole")} onClick={() => act(api.decideRoleProposal(n.project, n.proposal!, false), app.t("common.done"))}>
              <X className="h-3.5 w-3.5" />
            </button>
            <button disabled={busy} className="btn-primary" title={app.t("need.approveRole")} onClick={() => act(api.decideRoleProposal(n.project, n.proposal!, true), app.t("need.roleAdded"))}>
              <UserPlus className="h-3.5 w-3.5" /> {app.t("common.approve")}
            </button>
          </>
        )}
        {n.kind === "MergeReview" && n.ticket && (
          <button disabled={busy} className="btn-primary" onClick={() => act(api.approve(n.project, "merge", n.ticket!), app.t("common.done"))}>
            <GitMerge className="h-3.5 w-3.5" /> Merge
          </button>
        )}
        {n.kind === "SkillProposal" && n.proposal && <SkillProposalActions id={n.proposal} />}
        {n.kind === "CancelledWork" && n.ticket && (
          <button disabled={busy} className="btn-primary" title={app.t("ticket.waiveHint")} onClick={() => act(api.ticketAction(n.project, n.ticket!, "waive"), app.t("common.done"))}>
            <Ban className="h-3.5 w-3.5" /> {app.t("ticket.waive")}
          </button>
        )}
        {n.ticket && (compact || n.kind !== "Question") && (
          <button className="btn" onClick={() => app.openTicket(n.ticket!, n.project)}>
            {n.ticket}
          </button>
        )}
        {n.kind === "AgentBlocked" && n.agent_id && (
          <>
            <button disabled={busy} className="btn-primary" title={app.t("agents.resumeHint")} onClick={() => act(api.setAgentIdle(n.agent_id!), app.t("common.done"))}>
              <RotateCcw className="h-3.5 w-3.5" /> {app.t("agents.resume")}
            </button>
            <button className="btn" onClick={() => app.go("agent", n.agent_id!)}>
              {n.agent}
            </button>
          </>
        )}
      </div>
    </div>
  );
}

export function CommandRow({ a }: { a: CommandApproval }) {
  const app = useApp();
  // `false`: the request had already been answered or ran out of time (denied after 30 minutes): say so.
  const decide = (d: "allow" | "always" | "deny") =>
    api
      .decideCommand(a.id, d)
      .then((taken) => {
        app.setApprovals((xs) => xs.filter((x) => x.id !== a.id));
        if (!taken) app.notify(app.t("needs.commandGone"));
        app.refresh();
      })
      .catch((e) => app.notify(String(e)));
  return (
    <div className="flex items-start gap-3 rounded-lg border border-amber-500/30 bg-amber-500/5 p-3">
      {a.tool ? <Plug className="mt-0.5 h-4 w-4 text-amber-500" /> : <Terminal className="mt-0.5 h-4 w-4 text-amber-500" />}
      <div className="min-w-0 flex-1">
        <div className="flex items-center gap-2">
          <span className="label">{a.agent}</span>
          {a.ticket && <Mono className="text-subtle">{a.ticket}</Mono>}
          <Mono className="text-subtle">{ago(a.at)}</Mono>
        </div>
        {a.tool ? (
          <>
            <div className="mt-1 flex flex-wrap items-center gap-1.5 text-xs">
              <span>{app.t("needs.mcpTool")}</span>
              <Mono className="font-semibold">
                {a.tool.server}/{a.tool.tool}
              </Mono>
              {a.tool.destructive && <span className="chip border-rose-500/30 text-rose-600">destructive</span>}
            </div>
            <pre className="mt-1 max-h-48 overflow-auto whitespace-pre-wrap break-all rounded-md bg-background p-2 font-mono text-[11px]">{a.tool.arguments}</pre>
          </>
        ) : (
          <pre className="mt-1 whitespace-pre-wrap break-all rounded-md bg-background p-2 font-mono text-[11px]">{a.command}</pre>
        )}
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
  const [items, setItems] = useState<{ id: string; project: string; items: NeedItem[]; failed?: boolean }[]>([]);
  useEffect(() => {
    // One project that cannot be read does not empty the page for the others (audit L15, 2026-10-31): it says so.
    Promise.all(
      app.projects.map((p) =>
        api
          .needsYou(p.id)
          .then((r) => ({ id: p.id, project: p.name, items: r.needs, cmds: r.command_approvals, failed: false }))
          .catch(() => ({ id: p.id, project: p.name, items: [] as NeedItem[], cmds: [] as typeof app.approvals, failed: true })),
      ),
    ).then((rs) => {
      setItems(rs);
      const cmds = rs.flatMap((r) => r.cmds);
      app.setApprovals(cmds.filter((c, i) => cmds.findIndex((x) => x.id === c.id) === i));
    });
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [app.projects, app.tick]);
  const total = items.reduce((n, r) => n + r.items.length, 0) + app.approvals.length;
  return (
    <div className="space-y-4">
      <PageHeader title={app.t("nav.needs")} sub={total ? `${total} ${app.t("needs.waiting")}` : app.t("needs.sub")} />
      {items
        .map((r) => ({ ...r, cmds: app.approvals.filter((a) => a.project === r.id) }))
        .filter((r) => r.items.length || r.cmds.length || r.failed)
        .map((r) => (
          <Section key={r.id} title={r.project}>
            {r.failed && <div className="text-sm text-rose-600 dark:text-rose-400">{app.t("needs.unreadable")}</div>}
            <div className="space-y-2">
              <AnimatePresence initial={false} mode="popLayout">
                {r.cmds.map((a) => (
                  <ListItem key={`cmd-${a.id}`} id={`cmd-${a.id}`}>
                    <CommandRow a={a} />
                  </ListItem>
                ))}
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
    </div>
  );
}
