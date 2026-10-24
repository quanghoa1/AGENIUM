import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import { ReducedMotionConfig } from "@animate-ui/react/utils/motion";
import { PageEnter } from "./components/motion";
import { Shell } from "./components/Shell";
import "./index.css";
import { AppProvider, useApp } from "./lib/store";
import { AgentView, AgentsView, ProjectsView, TeamsView } from "./views/Organize";
import { AuditView, ProvidersView, SettingsView, SetupView, TemplatesView, UsageView } from "./views/Admin";
import { ChatView } from "./views/Chat";
import { NeedsView } from "./views/Needs";
import { OverviewView } from "./views/Overview";
import { ActivityView, RunView, RunsView } from "./views/Runs";
import { TicketView } from "./views/Ticket";
import { TicketsView } from "./views/Tickets";

function Router() {
  const app = useApp();
  if (!app.status) return <div className="p-6 text-xs text-subtle">…</div>;
  if (!app.status.ready) return <SetupView />;
  const { view, id } = app.route;
  const page = (() => {
    switch (view) {
      case "needs":
        return <NeedsView />;
      case "tickets":
        return <TicketsView />;
      case "ticket":
        return id ? <TicketView id={id} /> : <TicketsView />;
      case "runs":
        return <RunsView />;
      case "run":
        return id ? <RunView id={id} /> : <RunsView />;
      case "activity":
        return <ActivityView />;
      case "projects":
        return <ProjectsView />;
      case "agents":
        return <AgentsView />;
      case "agent":
        return id ? <AgentView id={id} /> : <AgentsView />;
      case "teams":
        return <TeamsView />;
      case "providers":
        return <ProvidersView />;
      case "templates":
        return <TemplatesView />;
      case "audit":
        return <AuditView />;
      case "usage":
        return <UsageView />;
      case "settings":
        return <SettingsView />;
      case "chat":
        return <ChatView />;
      default:
        return <OverviewView />;
    }
  })();
  return (
    <Shell>
      <PageEnter id={`${view}/${id ?? ""}`}>{page}</PageEnter>
    </Shell>
  );
}

createRoot(document.getElementById("root")!).render(
  <StrictMode>
    <AppProvider>
      {/* Under the OS "reduce motion" setting, fades and blurs stay and movement goes (animate-ui rules). */}
      <ReducedMotionConfig>
        <Router />
      </ReducedMotionConfig>
    </AppProvider>
  </StrictMode>,
);
