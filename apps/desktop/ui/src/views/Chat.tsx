import { Users } from "lucide-react";
import { ChatPane } from "../components/ChatPane";
import { NeedsProject } from "../components/GettingStarted";
import { PageHeader } from "../components/ui";
import { useApp } from "../lib/store";

/** Talk to the project's Lead (or Solo agent) in your language: discovery, spec, plan, steering. */
export function ChatView() {
  const app = useApp();
  const p = app.projects.find((x) => x.id === app.project);
  if (!p) return <NeedsProject title={app.t("chat.title")} />;
  const lead = p.team ? (p.team.solo ? "Solo" : "Lead") : "–";
  return (
    <div className="flex h-full flex-col">
      <PageHeader title={app.t("chat.title")} sub={p.team ? `${p.key_prefix} — ${p.name} · ${p.team.name} (${lead})` : `${p.key_prefix} — ${p.name}`} />
      {!p.team && (
        // The chat needs someone to answer: the project's Lead (or Solo agent).
        <div className="mb-3 flex flex-wrap items-center gap-3 rounded-xl border border-amber-500/40 bg-amber-500/10 p-3 text-xs">
          <span>{app.t("chat.noTeam")}</span>
          <button className="btn-primary ml-auto" onClick={() => app.go("teams")}>
            <Users className="h-3.5 w-3.5" /> {app.t("projects.nextTeam")}
          </button>
        </div>
      )}
      <div className="card flex min-h-0 flex-1 flex-col">
        <ChatPane key={p.id} project={p.id} agentName={p.team?.name ?? "lead"} agentId={p.team?.lead} enabled={!!p.team} />
      </div>
    </div>
  );
}
