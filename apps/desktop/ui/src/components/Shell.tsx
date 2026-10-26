import {
  BarChart3,
  Bell,
  Bot,
  Brain,
  ChevronLeft,
  ChevronRight,
  Cpu,
  FileClock,
  FolderKanban,
  KanbanSquare,
  Layers,
  LayoutDashboard,
  MessageSquare,
  Monitor,
  Moon,
  OctagonX,
  Pause,
  Play,
  CheckCheck,
  MessagesSquare,
  Radio,
  Search,
  Settings,
  Sun,
  Terminal,
  Users,
  Wrench,
} from "lucide-react";
import { useEffect, useState, type ReactNode } from "react";
import { api } from "../lib/api";
import type { Key } from "../lib/i18n";
import { inTauri, useApp, useLive, type View } from "../lib/store";
import { motion } from "motion/react";
import { CommandPalette } from "./CommandPalette";
import { CountBadge, Overlay, Segmented, spring, Toast } from "./motion";
import { cn } from "./ui";
import { UsageFooter } from "./UsageFooter";

interface NavItem {
  id: View;
  label: Key;
  icon: ReactNode;
  badge?: number;
  tone?: "amber" | "cyan" | "blue";
}

export function Shell({ children }: { children: ReactNode }) {
  const app = useApp();
  const live = useLive();
  const { t, route, go } = app;
  const [collapsed, setCollapsed] = useState(false);
  const glide = spring.selection;
  const [palette, setPalette] = useState(false);
  const current = app.projects.find((p) => p.id === app.project);
  const needs = app.projects.reduce((n, p) => n + p.needs_you, 0) + app.approvals.length;
  // Runs streaming right now (the Live tab's badge).
  const working = Object.values(live).filter((l) => !l.done && Date.now() - l.at < 120_000).length;
  useEffect(() => {
    const f = (e: KeyboardEvent) => {
      if ((e.ctrlKey || e.metaKey) && e.key.toLowerCase() === "k") {
        e.preventDefault();
        setPalette((v) => !v);
      }
      // F11: real fullscreen on and off (the window opens maximized).
      if (e.key === "F11" && inTauri) {
        e.preventDefault();
        import("@tauri-apps/api/window").then(async ({ getCurrentWindow }) => {
          const w = getCurrentWindow();
          await w.setFullscreen(!(await w.isFullscreen()));
        });
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
        { id: "live", label: "nav.live", icon: <Radio className="h-4 w-4" />, badge: working || undefined, tone: "blue" },
        { id: "needs", label: "nav.needs", icon: <Bell className="h-4 w-4" />, badge: needs || undefined, tone: "amber" },
        { id: "tickets", label: "nav.tickets", icon: <KanbanSquare className="h-4 w-4" />, badge: current?.tickets || undefined, tone: "cyan" },
        { id: "runs", label: "nav.runs", icon: <Terminal className="h-4 w-4" /> },
      ],
    },
    {
      label: "nav.organize",
      items: [
        { id: "projects", label: "nav.projects", icon: <FolderKanban className="h-4 w-4" />, badge: app.projects.length || undefined },
        { id: "agents", label: "nav.agents", icon: <Bot className="h-4 w-4" /> },
        { id: "memory", label: "nav.memory", icon: <Brain className="h-4 w-4" /> },
        { id: "teams", label: "nav.teams", icon: <Users className="h-4 w-4" /> },
      ],
    },
    {
      label: "nav.capabilities",
      items: [
        { id: "providers", label: "nav.providers", icon: <Cpu className="h-4 w-4" /> },
        { id: "library", label: "nav.library", icon: <Wrench className="h-4 w-4" /> },
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
                      "press relative flex w-full items-center justify-between rounded-lg px-2.5 py-2 text-xs font-medium",
                      active(it.id) ? "text-blue-600 dark:text-blue-400" : "text-muted hover:bg-elevated hover:text-fg",
                    )}
                  >
                    {active(it.id) && (
                      // The selection glides from item to item.
                      <motion.span layoutId="nav-active" transition={glide} className="absolute inset-0 rounded-lg border border-blue-500/30 bg-blue-600/15" />
                    )}
                    <span className="relative z-10 flex items-center gap-2.5">
                      {it.icon}
                      {!collapsed && t(it.label)}
                    </span>
                    {!collapsed && it.badge !== undefined && (
                      <CountBadge
                        count={it.badge}
                        className={cn("relative z-10 min-w-[1.5rem] rounded border px-1.5 font-mono text-[10px]", it.tone ? badgeTone[it.tone] : "border-line-strong bg-elevated text-subtle")}
                      />
                    )}
                  </button>
                ))}
              </div>
            </div>
          ))}
        </nav>
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
          <RunControl />
          <button
            onClick={() => {
              api.stopAll().then(() => (app.notify(t("top.stopped")), app.refresh()));
            }}
            className="btn-danger"
            title={t("top.stop")}
          >
            <OctagonX className="h-3.5 w-3.5" />
          </button>
          <button onClick={() => app.setLang(app.lang === "vi" ? "en" : "vi")} className="btn font-mono" title={t("settings.language")}>
            {app.lang.toUpperCase()}
          </button>
          <Segmented
            id="theme"
            value={app.themeMode}
            onChange={app.setThemeMode}
            itemClassName="px-1.5 py-1.5"
            items={(["light", "dark", "system"] as const).map((m) => ({
              value: m,
              label: m === "light" ? <Sun className="h-3.5 w-3.5" /> : m === "dark" ? <Moon className="h-3.5 w-3.5" /> : <Monitor className="h-3.5 w-3.5" />,
            }))}
          />
        </header>
        <main className="min-h-0 flex-1 overflow-y-auto p-5">{children}</main>
        <UsageFooter />
      </div>

      <Overlay open={palette} onClose={() => setPalette(false)} kind="palette" top="pt-24">
        <CommandPalette onClose={() => setPalette(false)} />
      </Overlay>
      <Toast message={app.toast} />
    </div>
  );
}

/** The selected project's team: talking about the requirements, working on its own, stopped by you, or done
 *  (ADR-63). Stop and go on are yours; everything in between the team does by itself. */
function RunControl() {
  const app = useApp();
  const p = app.projects.find((x) => x.id === app.project);
  if (!p?.run_state) return null;
  const s = p.run_state;
  const act = (f: (id: string) => Promise<void>) => f(p.id).then(() => app.refresh()).catch((e) => app.notify(String(e)));
  const tone =
    s === "Running"
      ? "border-emerald-500/40 bg-emerald-500/10 text-emerald-600 dark:text-emerald-300"
      : s === "Paused"
        ? "border-amber-500/40 bg-amber-500/10 text-amber-600 dark:text-amber-300"
        : s === "Done"
          ? "border-blue-500/40 bg-blue-500/10 text-blue-600 dark:text-blue-300"
          : "border-line-strong text-muted";
  return (
    <div className="flex shrink-0 items-center gap-1" title={app.t(`run.${s}.hint`)}>
      <span className={cn("inline-flex items-center gap-1.5 rounded-lg border px-2.5 py-1.5 text-xs font-medium", tone)}>
        {s === "Running" ? (
          <span className="relative inline-flex h-2 w-2">
            <span className="absolute inset-0 animate-ping rounded-full bg-emerald-500/60" />
            <span className="relative h-2 w-2 rounded-full bg-emerald-500" />
          </span>
        ) : s === "Paused" ? (
          <Pause className="h-3.5 w-3.5" />
        ) : s === "Done" ? (
          <CheckCheck className="h-3.5 w-3.5" />
        ) : (
          <MessagesSquare className="h-3.5 w-3.5" />
        )}
        {app.t(`run.${s}`)}
      </span>
      {s === "Running" && (
        <button className="btn" onClick={() => act(api.pauseProject)}>
          <Pause className="h-3.5 w-3.5" /> {app.t("run.pause")}
        </button>
      )}
      {s === "Paused" && (
        <button className="btn-primary" onClick={() => act(api.resumeProject)}>
          <Play className="h-3.5 w-3.5" /> {app.t("run.resume")}
        </button>
      )}
    </div>
  );
}
