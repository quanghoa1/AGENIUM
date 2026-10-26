import { ChatPane } from "../components/ChatPane";
import { Empty, PageHeader } from "../components/ui";
import { useApp } from "../lib/store";

/** Talk to the project's Lead (or Solo agent) in your language: discovery, spec, plan, steering. */
export function ChatView() {
  const app = useApp();
  const p = app.projects.find((x) => x.id === app.project);
  if (!p) return <Empty>{app.t("common.none")}</Empty>;
  const lead = p.team ? (p.team.solo ? "Solo" : "Lead") : "–";
  return (
    <div className="flex h-full flex-col">
      <PageHeader title={app.t("chat.title")} sub={`${p.key_prefix} — ${p.name} · ${p.team?.name ?? "no team"} (${lead})`} />
      <div className="card flex min-h-0 flex-1 flex-col">
        <ChatPane key={p.id} project={p.id} agentName={p.team?.name ?? "lead"} agentId={p.team?.lead} enabled={!!p.team} />
      </div>
    </div>
  );
}
