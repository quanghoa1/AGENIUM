import { CheckCheck, KanbanSquare, Radio, Terminal, Users } from "lucide-react";
import { CloseProject } from "../components/CloseProject";
import { ReopenProject } from "../components/ReopenProject";
import { ProjectPicker } from "../components/ProjectPicker";
import { ChatPane } from "../components/ChatPane";
import { NeedsProject } from "../components/GettingStarted";
import { PageHeader } from "../components/ui";
import { useState } from "react";
import { useApp } from "../lib/store";
import type { Prefill } from "../components/ChatPane";

const DRAFT = "agenium-chat-draft";

/** Opens the chat with `text` ready to send (a follow-up on a closed ticket, more work on a done project); `note`
 *  says above the input what the message is for. */
export function askLead(go: (v: "chat") => void, text: string, note?: string) {
  try {
    sessionStorage.setItem(DRAFT, JSON.stringify({ text, note }));
  } catch {
    /* the chat opens empty */
  }
  go("chat");
}

function takeDraft(): Prefill | null {
  try {
    const t = sessionStorage.getItem(DRAFT);
    sessionStorage.removeItem(DRAFT);
    if (!t) return null;
    const d = JSON.parse(t) as { text: string; note?: string };
    return { text: d.text, note: d.note, nonce: Date.now() };
  } catch {
    return null;
  }
}

/** Talk to the project's Lead (or Solo agent) in your language: discovery, spec, plan, steering. */
export function ChatView() {
  const app = useApp();
  const [prefill] = useState(takeDraft);
  const p = app.projects.find((x) => x.id === app.project);
  if (!p) return <NeedsProject title={app.t("chat.title")} />;
  const lead = p.team ? (p.team.solo ? "Solo" : "Lead") : "–";
  const open = Object.entries(p.by_status ?? {})
    .filter(([s]) => s !== "Closed" && s !== "Cancelled")
    .reduce((n, [, c]) => n + c, 0);
  return (
    <div className="flex h-full flex-col">
      <PageHeader title={app.t("chat.title")} sub={p.team ? `${p.name} · ${p.team.name} (${lead})` : p.name}>
        <ProjectPicker />
      </PageHeader>
      {!p.team && (
        // The chat needs someone to answer: the project's Lead (or Solo agent).
        <div className="mb-3 flex flex-wrap items-center gap-3 rounded-xl border border-amber-500/40 bg-amber-500/10 p-3 text-xs">
          <span>{app.t(p.run_state === "Closed" ? "run.Closed.hint" : "chat.noTeam")}</span>
          {p.run_state === "Closed" ? (
            <span className="ml-auto">
              <ReopenProject p={p} />
            </span>
          ) : (
            <button className="btn-primary ml-auto" onClick={() => app.go("teams")}>
              <Users className="h-3.5 w-3.5" /> {app.t("projects.nextTeam")}
            </button>
          )}
        </div>
      )}
      {/* Where the agreed work goes on, and the way there: the office, the board, the runs. */}
      {p.team && p.run_state === "Running" && (
        <div className="mb-3 flex flex-wrap items-center gap-2 rounded-xl border border-emerald-500/40 bg-emerald-500/10 p-3 text-xs">
          <span className="relative mr-1 inline-flex h-2 w-2">
            <span className="absolute inset-0 animate-ping rounded-full bg-emerald-500/60" />
            <span className="relative h-2 w-2 rounded-full bg-emerald-500" />
          </span>
          <span className="flex-1">{open ? app.t("chat.working").replace("{n}", String(open)) : app.t("chat.planning")}</span>
          <button className="btn-primary" onClick={() => app.go("live")}>
            <Radio className="h-3.5 w-3.5" /> {app.t("chat.watch")}
          </button>
          <button className="btn" onClick={() => app.go("tickets")}>
            <KanbanSquare className="h-3.5 w-3.5" /> {app.t("nav.tickets")}
          </button>
          <button className="btn" onClick={() => app.go("runs")}>
            <Terminal className="h-3.5 w-3.5" /> {app.t("nav.runs")}
          </button>
        </div>
      )}
      {p.team && p.run_state === "Done" && (
        <div className="mb-3 flex flex-wrap items-center gap-2 rounded-xl border border-blue-500/40 bg-blue-500/10 p-3 text-xs">
          <CheckCheck className="h-4 w-4 text-blue-500" />
          <span className="flex-1">{app.t("chat.done")}</span>
          <button className="btn" onClick={() => app.go("tickets")}>
            <KanbanSquare className="h-3.5 w-3.5" /> {app.t("nav.tickets")}
          </button>
          <CloseProject p={p} />
        </div>
      )}
      <div className="card flex min-h-0 flex-1 flex-col">
        <ChatPane key={p.id} project={p.id} agentName={p.team?.name ?? "lead"} agentId={p.team?.lead} enabled={!!p.team} prefill={prefill} />
      </div>
    </div>
  );
}
