// App-wide state: route (hash based, so reload keeps the place), language, theme, selected project, a refresh
// counter bumped by domain events, live run output, command approvals and toasts.
import { createContext, useCallback, useContext, useEffect, useMemo, useRef, useState, type ReactNode } from "react";
import { api, inTauri, onApproval, onEvent, onRun } from "./api";
import { dictionaries, type Key, type Lang } from "./i18n";
import type { CommandApproval, ProjectSummary, Status } from "./types";

export type View =
  | "overview"
  | "activity"
  | "tickets"
  | "ticket"
  | "runs"
  | "run"
  | "needs"
  | "projects"
  | "agents"
  | "agent"
  | "teams"
  | "providers"
  | "templates"
  | "audit"
  | "usage"
  | "settings"
  | "chat"
  | "live"
  | "library";

export interface Route {
  view: View;
  id?: string;
}

function parseHash(): Route {
  const [view, id] = window.location.hash.replace(/^#\/?/, "").split("/");
  return { view: (view || "overview") as View, id: id ? decodeURIComponent(id) : undefined };
}

export type ThemeMode = "dark" | "light" | "system";

/** Live output of one run: streamed text, the tail of its thinking, tool calls, and what it does right now. */
export interface LiveRun {
  text: string;
  thinking: string;
  tools: { name: string; args?: unknown }[];
  /** What the model is doing now: reasoning, writing text, or calling a tool (`tool` names it). */
  phase: "thinking" | "text" | "tool" | "idle";
  tool?: string;
  done: boolean;
  /** Last event (ms since epoch). */
  at: number;
}

const TAIL = 20_000;

interface Ctx {
  status: Status | null;
  route: Route;
  go: (view: View, id?: string) => void;
  lang: Lang;
  setLang: (l: Lang) => void;
  t: (k: Key) => string;
  themeMode: ThemeMode;
  setThemeMode: (m: ThemeMode) => void;
  projects: ProjectSummary[];
  project: string | null;
  setProject: (id: string | null) => void;
  /** Bumped on every committed event: views reload what they show. */
  tick: number;
  refresh: () => void;
  live: Record<string, LiveRun>;
  approvals: CommandApproval[];
  setApprovals: (a: CommandApproval[]) => void;
  toast: string | null;
  notify: (m: string) => void;
  autopilot: boolean;
  setAutopilot: (on: boolean) => void;
  reloadStatus: () => void;
}

const C = createContext<Ctx | null>(null);

export function useApp(): Ctx {
  const c = useContext(C);
  if (!c) throw new Error("useApp outside the provider");
  return c;
}

function readLocal(k: string): string | null {
  try {
    return localStorage.getItem(k);
  } catch {
    return null;
  }
}

function writeLocal(k: string, v: string) {
  try {
    localStorage.setItem(k, v);
  } catch {
    /* storage may be unavailable */
  }
}

export function AppProvider({ children }: { children: ReactNode }) {
  const [status, setStatus] = useState<Status | null>(null);
  const [route, setRoute] = useState<Route>(parseHash);
  const [lang, setLangState] = useState<Lang>((readLocal("agenium-lang") as Lang) || "vi");
  const [themeMode, setThemeModeState] = useState<ThemeMode>((readLocal("agenium-theme") as ThemeMode) || "dark");
  const [projects, setProjects] = useState<ProjectSummary[]>([]);
  const [project, setProjectState] = useState<string | null>(readLocal("agenium-project"));
  const [tick, setTick] = useState(0);
  const [live, setLive] = useState<Record<string, LiveRun>>({});
  const [approvals, setApprovals] = useState<CommandApproval[]>([]);
  const [toast, setToast] = useState<string | null>(null);
  const [autopilot, setAutopilotState] = useState(false);
  const toastTimer = useRef<number | undefined>(undefined);

  const reloadStatus = useCallback(() => {
    api.status().then((s) => {
      setStatus(s);
      setAutopilotState(s.autopilot);
    });
  }, []);
  useEffect(reloadStatus, [reloadStatus]);

  useEffect(() => {
    const f = () => setRoute(parseHash());
    window.addEventListener("hashchange", f);
    return () => window.removeEventListener("hashchange", f);
  }, []);

  // Theme: class on <html>, "system" follows the OS.
  useEffect(() => {
    const apply = () => {
      const dark =
        themeMode === "dark" || (themeMode === "system" && window.matchMedia("(prefers-color-scheme: dark)").matches);
      document.documentElement.classList.toggle("dark", dark);
      document.documentElement.classList.toggle("light", !dark);
    };
    apply();
    const mq = window.matchMedia("(prefers-color-scheme: dark)");
    mq.addEventListener("change", apply);
    return () => mq.removeEventListener("change", apply);
  }, [themeMode]);

  const refresh = useCallback(() => setTick((n) => n + 1), []);

  useEffect(() => {
    if (!status?.ready) return;
    api.projects().then((ps) => {
      setProjects(ps);
      setProjectState((cur) => (cur && ps.some((p) => p.id === cur) ? cur : (ps[0]?.id ?? null)));
    });
  }, [status?.ready, tick]);

  // Domain events refresh the views (coalesced to at most one refresh per 400 ms).
  useEffect(() => {
    let pending: number | undefined;
    const un = onEvent(() => {
      if (pending !== undefined) return;
      pending = window.setTimeout(() => {
        pending = undefined;
        setTick((n) => n + 1);
      }, 400);
    });
    return () => {
      un.then((f) => f());
    };
  }, []);

  useEffect(() => {
    const un = onRun((run, ev) => {
      setLive((m) => {
        const cur: LiveRun = m[run] ?? { text: "", thinking: "", tools: [], phase: "idle", done: false, at: 0 };
        const next: LiveRun = { ...cur, at: Date.now() };
        if (ev.type === "text") {
          next.text = (cur.text + ev.text).slice(-TAIL);
          next.phase = "text";
        } else if (ev.type === "thinking") {
          next.thinking = (cur.thinking + ev.text).slice(-2000);
          next.phase = "thinking";
        } else if (ev.type === "restart") {
          next.text = "";
          next.phase = "thinking";
        } else if (ev.type === "tool_start") {
          next.phase = "tool";
          next.tool = ev.name;
        } else if (ev.type === "tool") {
          next.tools = [...cur.tools.slice(-50), { name: ev.name, args: ev.args }];
          next.phase = "tool";
          next.tool = ev.name;
        } else if (ev.type === "usage") {
          // A step ended: the next one starts by thinking.
          next.phase = "thinking";
          next.thinking = "";
        } else if (ev.type === "done") {
          next.done = true;
          next.phase = "idle";
        }
        return { ...m, [run]: next };
      });
    });
    return () => {
      un.then((f) => f());
    };
  }, []);

  const notify = useCallback((m: string) => {
    setToast(m);
    window.clearTimeout(toastTimer.current);
    toastTimer.current = window.setTimeout(() => setToast(null), 5000);
  }, []);

  useEffect(() => {
    const un = onApproval((a) => {
      setApprovals((xs) => [...xs.filter((x) => x.id !== a.id), a]);
      notify(`${a.agent}: ${a.command}`);
    });
    return () => {
      un.then((f) => f());
    };
  }, [notify]);

  const value = useMemo<Ctx>(
    () => ({
      status,
      route,
      go: (view, id) => {
        window.location.hash = "/" + view + (id ? "/" + encodeURIComponent(id) : "");
      },
      lang,
      setLang: (l) => {
        setLangState(l);
        writeLocal("agenium-lang", l);
      },
      t: (k) => dictionaries[lang][k] ?? k,
      themeMode,
      setThemeMode: (m) => {
        setThemeModeState(m);
        writeLocal("agenium-theme", m);
      },
      projects,
      project,
      setProject: (id) => {
        setProjectState(id);
        if (id) writeLocal("agenium-project", id);
      },
      tick,
      refresh,
      live,
      approvals,
      setApprovals,
      toast,
      notify,
      autopilot,
      setAutopilot: (on) => {
        api.setAutopilot(on).then(setAutopilotState);
      },
      reloadStatus,
    }),
    [status, route, lang, themeMode, projects, project, tick, refresh, live, approvals, toast, notify, autopilot, reloadStatus],
  );
  return <C.Provider value={value}>{children}</C.Provider>;
}

export { inTauri };
