import {
  Activity,
  BarChart3,
  Bell,
  Bot,
  ChevronLeft,
  ChevronRight,
  Cpu,
  FileClock,
  FolderKanban,
  KanbanSquare,
  LayoutDashboard,
  Layers,
  MessageSquare,
  Monitor,
  Moon,
  OctagonX,
  Play,
  Search,
  Settings,
  Sun,
  Terminal,
  Users,
} from "lucide-react";
import { useEffect, useState, type ReactNode } from "react";
import { api } from "../lib/api";
import type { Key } from "../lib/i18n";
import { inTauri, useApp, type View } from "../lib/store";
import { CommandPalette } from "./CommandPalette";
import { cn } from "./ui";

interface NavItem {
  id: View;
  label: Key;
  icon: ReactNode;
  badge?: number;
  tone?: "amber" | "cyan" | "blue";
}

export function Shell({ children }: { children: ReactNode }) {
  const app = useApp();
  const { t, route, go } = app;
  const [collapsed, setCollapsed] = useState(false);
  const [palette, setPalette] = useState(false);
  const current = app.projects.find((p) => p.id === app.project);
  const needs = app.projects.reduce((n, p) => n + p.needs_you, 0) + app.approvals.length;
  useEffect(() => {
    const f = (e: KeyboardEvent) => {
      if ((e.ctrlKey || e.metaKey) && e.key.toLowerCase() === "k") {
        e.preventDefault();
        setPalette((v) => !v);
      }
    };
    window.addEventListener("keydown", f);
    return () => window.removeEventListener("keydown", f);
  }, []);

  const groups: { label: Key; items: NavItem[] }[] = [
    {
      label: "nav.monitor",
      items: [
        { id: "overview", label: "nav.overview", icon: <LayoutDashboard className="h-4 w-4" /> },
        { id: "needs", label: "nav.needs", icon: <Bell className="h-4 w-4" />, badge: needs || undefined, tone: "amber" },
        { id: "tickets", label: "nav.tickets", icon: <KanbanSquare className="h-4 w-4" />, badge: current?.tickets || undefined, tone: "cyan" },
        { id: "runs", label: "nav.runs", icon: <Terminal className="h-4 w-4" /> },
        { id: "activity", label: "nav.activity", icon: <Activity className="h-4 w-4" /> },
      ],
    },
    {
      label: "nav.organize",
      items: [
        { id: "projects", label: "nav.projects", icon: <FolderKanban className="h-4 w-4" />, badge: app.projects.length || undefined },
        { id: "agents", label: "nav.agents", icon: <Bot className="h-4 w-4" /> },
        { id: "teams", label: "nav.teams", icon: <Users className="h-4 w-4" /> },
      ],
    },
    {
      label: "nav.capabilities",
      items: [
        { id: "providers", label: "nav.providers", icon: <Cpu className="h-4 w-4" /> },
        { id: "templates", label: "nav.templates", icon: <Layers className="h-4 w-4" /> },
      ],
    },
    {
      label: "nav.governance",
      items: [
        { id: "audit", label: "nav.audit", icon: <FileClock className="h-4 w-4" /> },
        { id: "usage", label: "nav.usage", icon: <BarChart3 className="h-4 w-4" /> },
        { id: "settings", label: "nav.settings", icon: <Settings className="h-4 w-4" /> },
      ],
    },
  ];
  const active = (id: View) =>
    route.view === id || (id === "tickets" && route.view === "ticket") || (id === "runs" && route.view === "run") || (id === "agents" && route.view === "agent");
  const badgeTone = { amber: "border-amber-500/40 bg-amber-500/15 text-amber-600 dark:text-amber-300", cyan: "border-cyan-500/30 bg-cyan-500/10 text-cyan-600 dark:text-cyan-300", blue: "border-blue-500/30 bg-blue-500/10 text-blue-600 dark:text-blue-300" };

  return (
    <div className="flex h-full w-full overflow-hidden bg-background text-fg">
      <aside className={cn("flex shrink-0 select-none flex-col border-r border-line bg-surface transition-all duration-200", collapsed ? "w-16" : "w-60")}>
        <div className="flex h-14 items-center justify-between border-b border-line px-3.5">
          <button onClick={() => go("overview")} className="flex items-center gap-2.5 text-left">
            <div className="flex h-8 w-8 items-center justify-center rounded-lg bg-gradient-to-br from-blue-600 to-cyan-500 font-mono text-sm font-bold text-white shadow-sm">AG</div>
            {!collapsed && (
              <div>
                <div className="flex items-center gap-1.5">
                  <span className="text-sm font-bold tracking-tight">AGENIUM</span>
                  <span className="rounded border border-blue-500/30 bg-blue-500/15 px-1.5 font-mono text-[10px] text-blue-500">v{app.status?.version ?? "0.1"}</span>
                </div>
                <p className="truncate text-[10px] text-subtle">{t("brand.sub")}</p>
              </div>
            )}
          </button>
          <button onClick={() => setCollapsed(!collapsed)} className="rounded-lg p-1.5 text-subtle hover:bg-elevated" aria-label="Toggle sidebar">
            {collapsed ? <ChevronRight className="h-4 w-4" /> : <ChevronLeft className="h-4 w-4" />}
          </button>
        </div>
        {!collapsed && (
          <div className="border-b border-line p-3">
            <button onClick={() => go("chat")} className="btn-primary w-full justify-center">
              <MessageSquare className="h-3.5 w-3.5" />
              {t("top.talk")}
            </button>
          </div>
        )}
        <nav className="flex-1 space-y-5 overflow-y-auto p-2.5">
          {groups.map((g) => (
            <div key={g.label}>
              {!collapsed && <div className="label mb-1.5 px-2.5">{t(g.label)}</div>}
              <div className="space-y-0.5">
                {g.items.map((it) => (
                  <button
                    key={it.id}
                    onClick={() => go(it.id)}
                    title={collapsed ? t(it.label) : undefined}
                    className={cn(
                      "flex w-full items-center justify-between rounded-lg border px-2.5 py-2 text-xs font-medium transition-colors",
                      active(it.id) ? "border-blue-500/30 bg-blue-600/15 text-blue-600 dark:text-blue-400" : "border-transparent text-muted hover:bg-elevated hover:text-fg",
                    )}
                  >
                    <span className="flex items-center gap-2.5">
                      {it.icon}
                      {!collapsed && t(it.label)}
                    </span>
                    {!collapsed && it.badge !== undefined && (
                      <span className={cn("rounded border px-1.5 font-mono text-[10px]", it.tone ? badgeTone[it.tone] : "border-line-strong bg-elevated text-subtle")}>{it.badge}</span>
                    )}
                  </button>
                ))}
              </div>
            </div>
          ))}
        </nav>
        {!collapsed && (
          <div className="border-t border-line p-3">
            <div className="rounded-lg border border-line bg-elevated p-2.5">
              <div className="label">{t("settings.root")}</div>
              <div className="mt-1 truncate font-mono text-[10px] text-muted" title={app.status?.root}>
                {app.status?.root ?? "…"}
              </div>
            </div>
          </div>
        )}
      </aside>

      <div className="flex min-w-0 flex-1 flex-col">
        <header className="flex h-14 shrink-0 items-center gap-3 border-b border-line bg-surface px-4">
          <div className="flex items-center gap-2">
            <span className="label">{t("top.project")}</span>
            <select className="input w-44 py-1" value={app.project ?? ""} onChange={(e) => app.setProject(e.target.value || null)}>
              {app.projects.length === 0 && <option value="">–</option>}
              {app.projects.map((p) => (
                <option key={p.id} value={p.id}>
                  {p.key_prefix} — {p.name}
                </option>
              ))}
            </select>
          </div>
          {!inTauri && <span className="chip border-amber-500/40 bg-amber-500/10 text-amber-600 dark:text-amber-300">{t("top.demo")}</span>}
          <button onClick={() => setPalette(true)} className="flex min-w-0 flex-1 items-center gap-2 rounded-lg border border-line bg-background px-3 py-1.5 text-left text-xs text-subtle hover:border-line-strong">
            <Search className="h-3.5 w-3.5" />
            <span className="truncate">{t("top.search")}</span>
            <kbd className="ml-auto rounded border border-line-strong px-1 font-mono text-[10px]">Ctrl K</kbd>
          </button>
          <button
            onClick={() => app.setAutopilot(!app.autopilot)}
            className={cn("btn", app.autopilot && "border-emerald-500/40 bg-emerald-500/10 text-emerald-600 dark:text-emerald-300")}
            title={t("top.autopilot")}
          >
            <Play className="h-3.5 w-3.5" />
            {t("top.autopilot")}: {app.autopilot ? "ON" : "OFF"}
          </button>
          <button
            onClick={() => {
              api.stopAll().then(() => app.notify(t("top.stopped")));
              app.setAutopilot(false);
            }}
            className="btn-danger"
            title={t("top.stop")}
          >
            <OctagonX className="h-3.5 w-3.5" />
          </button>
          <button onClick={() => app.setLang(app.lang === "vi" ? "en" : "vi")} className="btn font-mono" title={t("settings.language")}>
            {app.lang.toUpperCase()}
          </button>
          <div className="flex rounded-lg border border-line-strong bg-elevated p-0.5">
            {(["light", "dark", "system"] as const).map((m) => (
              <button key={m} onClick={() => app.setThemeMode(m)} className={cn("rounded-md p-1.5 text-subtle", app.themeMode === m && "bg-surface text-fg shadow-sm")} title={m}>
                {m === "light" ? <Sun className="h-3.5 w-3.5" /> : m === "dark" ? <Moon className="h-3.5 w-3.5" /> : <Monitor className="h-3.5 w-3.5" />}
              </button>
            ))}
          </div>
        </header>
        <main className="min-h-0 flex-1 overflow-y-auto p-5">{children}</main>
      </div>

      {palette && <CommandPalette onClose={() => setPalette(false)} />}
      {app.toast && (
        <div className="fixed bottom-4 right-4 z-50 max-w-sm rounded-xl border border-line-strong bg-surface px-4 py-3 text-xs shadow-xl">{app.toast}</div>
      )}
    </div>
  );
}
