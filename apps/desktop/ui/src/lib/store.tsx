// App-wide state: route (hash based, so reload keeps the place), the ticket open in its popup, language, theme,
// selected project, a refresh counter bumped by domain events, command approvals and toasts. Live run output has its
// own context (`useLive`): it changes on every streamed token, and only the views that show it should re-render.
import {
  createContext,
  useCallback,
  useContext,
  useEffect,
  useMemo,
  useRef,
  useState,
  type Dispatch,
  type ReactNode,
  type SetStateAction,
} from "react";
import { api, inTauri, onApproval, onEvent, onModel, onRun } from "./api";
import { dictionaries, type Key, type Lang } from "./i18n";
import type { CommandApproval, ProjectSummary, Status } from "./types";
import { setAgoLang } from "../components/ui";
import type { ToastMessage } from "../components/motion";

export type View =
  | "overview"
  | "tickets"
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
  | "memory"
  | "usage"
  | "settings"
  | "chat"
  | "live"
  | "library"
  | "browser";

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
  /** The run's agent (id), when the backend named it. */
  agent?: string | null;
  text: string;
  thinking: string;
  tools: { name: string; args?: unknown }[];
  /** What the model is doing now: reasoning, writing text, or calling a tool (`tool` names it). */
  phase: "thinking" | "text" | "tool" | "idle";
  tool?: string;
  /** The current tool's arguments, once the call is known whole. */
  toolArgs?: unknown;
  done: boolean;
  /** Last event (ms since epoch). */
  at: number;
}

const TAIL = 20_000;
/** Finished runs are dropped from the live map after this long (ms). */
const KEEP_DONE = 10 * 60_000;

interface Ctx {
  status: Status | null;
  route: Route;
  go: (view: View, id?: string) => void;
  /** The ticket shown in the popup over the current view (a ticket has no page of its own); null = none. */
  ticket: { key: string; project: string } | null;
  /** Opens a ticket in the popup, of `project` (default: the selected one); null closes it. */
  openTicket: (key: string | null, project?: string | null) => void;
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
  approvals: CommandApproval[];
  setApprovals: Dispatch<SetStateAction<CommandApproval[]>>;
  toast: ToastMessage | null;
  /** Tells the user `m` for a few seconds; `action` is a button that leads to it. */
  notify: (m: string, action?: ToastMessage["action"]) => void;
  reloadStatus: () => void;
}

const C = createContext<Ctx | null>(null);
const L = createContext<Record<string, LiveRun>>({});

export function useApp(): Ctx {
  const c = useContext(C);
  if (!c) throw new Error("useApp outside the provider");
  return c;
}

/** Live output of the runs in progress (and of those finished in the last minutes), by run id. */
/** One streamed event of a run applied to the live runs. */
function applyRunEvent(prev: Record<string, LiveRun>, run: string, ev: any, agent: string | null): Record<string, LiveRun> {
  let m = prev;
  if (!m[run]) {
    // A new run: forget the ones that finished a while ago.
    const now = Date.now();
    m = Object.fromEntries(Object.entries(m).filter(([, l]) => !l.done || now - l.at < KEEP_DONE));
  }
  const cur: LiveRun = m[run] ?? { agent, text: "", thinking: "", tools: [], phase: "idle", done: false, at: 0 };
  const next: LiveRun = { ...cur, agent: cur.agent ?? agent, at: Date.now() };
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
    next.toolArgs = undefined;
  } else if (ev.type === "tool") {
    next.tools = [...cur.tools.slice(-50), { name: ev.name, args: ev.args }];
    next.phase = "tool";
    next.tool = ev.name;
    next.toolArgs = ev.args;
  } else if (ev.type === "usage") {
    // A step ended: the next one starts by thinking.
    next.phase = "thinking";
    next.thinking = "";
  } else if (ev.type === "done") {
    next.done = true;
    next.phase = "idle";
  }
  return { ...m, [run]: next };
}

export function useLive(): Record<string, LiveRun> {
  return useContext(L);
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
  const [ticket, setTicket] = useState<{ key: string; project: string } | null>(null);
  // First start: the system's language when the app has it (Vietnamese), else English; the user's choice wins after.
  const [lang, setLangState] = useState<Lang>(
    (readLocal("agenium-lang") as Lang) || (navigator.language?.toLowerCase().startsWith("vi") ? "vi" : "en"),
  );
  // Relative times (`ago`) follow the interface language.
  setAgoLang(lang);
  const [themeMode, setThemeModeState] = useState<ThemeMode>((readLocal("agenium-theme") as ThemeMode) || "dark");
  const [projects, setProjects] = useState<ProjectSummary[]>([]);
  const [project, setProjectState] = useState<string | null>(readLocal("agenium-project"));
  const [tick, setTick] = useState(0);
  const [live, setLive] = useState<Record<string, LiveRun>>({});
  const [approvals, setApprovals] = useState<CommandApproval[]>([]);
  const [toast, setToast] = useState<ToastMessage | null>(null);
  const toastTimer = useRef<number | undefined>(undefined);

  const reloadStatus = useCallback(() => {
    // A failed status read is tried again rather than leaving the window on its loading screen.
    api.status().then(setStatus, () => window.setTimeout(() => api.status().then(setStatus, () => {}), 2000));
  }, []);
  useEffect(reloadStatus, [reloadStatus]);
  // The language chosen at setup or in Settings is kept in the config: it wins over this window's stored choice,
  // which is lost with the webview's data.
  const savedLang = status?.ui_language;
  useEffect(() => {
    if (savedLang === "vi" || savedLang === "en") {
      setLangState(savedLang);
      writeLocal("agenium-lang", savedLang);
    }
  }, [savedLang]);
  useEffect(() => {
    const off = onModel(reloadStatus);
    return () => void off.then((f) => f());
  }, [reloadStatus]);

  useEffect(() => {
    // Going elsewhere (a link in the ticket popup, the back button) closes the popup.
    const f = () => (setRoute(parseHash()), setTicket(null));
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

  // The project list is read on every refresh: a failure is told once (not on every tick), as is a project that
  // cannot be read.
  const projectsError = useRef("");
  useEffect(() => {
    if (!status?.ready) return;
    const tell = (m: string) => {
      if (m && m !== projectsError.current) notify(m);
      projectsError.current = m;
    };
    api
      .projects()
      .then((ps) => {
        setProjects(ps);
        setProjectState((cur) => (cur && ps.some((p) => p.id === cur) ? cur : (ps[0]?.id ?? null)));
        tell(ps.filter((p) => p.error).map((p) => `${p.name}: ${p.error}`).join("; "));
      })
      .catch((e) => tell(String(e)));
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
    // Streamed tokens come many times a second: they are applied together every 50 ms, so the views showing live
    // output (the office, the chat, the run page) re-render a few times a second, not once per token. A timer, not
    // an animation frame: a hidden window still keeps up.
    let queue: [string, any, string | null][] = [];
    let timer: number | undefined;
    const flush = () => {
      timer = undefined;
      const batch = queue;
      queue = [];
      setLive((prev) => batch.reduce((m, [run, ev, agent]) => applyRunEvent(m, run, ev, agent), prev));
    };
    const un = onRun((run, ev, agent) => {
      queue.push([run, ev, agent]);
      if (timer === undefined) timer = window.setTimeout(flush, 50);
    });
    return () => {
      window.clearTimeout(timer);
      un.then((f) => f());
    };
  }, []);

  const notify = useCallback((m: string, action?: ToastMessage["action"]) => {
    setToast({ text: m, action: action && { ...action, run: () => (setToast(null), action.run()) } });
    window.clearTimeout(toastTimer.current);
    // One with a way somewhere stays long enough to take it.
    toastTimer.current = window.setTimeout(() => setToast(null), action ? 12000 : 5000);
  }, []);

  // A failed backend call nobody caught still reaches the user (as a toast), instead of a view waiting silently.
  useEffect(() => {
    const f = (e: PromiseRejectionEvent) => {
      e.preventDefault();
      const r = e.reason;
      notify(typeof r === "string" ? r : r instanceof Error ? r.message : String(r));
    };
    window.addEventListener("unhandledrejection", f);
    return () => window.removeEventListener("unhandledrejection", f);
  }, [notify]);

  useEffect(() => {
    const un = onApproval((a) => {
      setApprovals((xs) => [...xs.filter((x) => x.id !== a.id), a]);
      notify(`${a.agent}: ${a.command}`);
    });
    return () => {
      un.then((f) => f());
    };
  }, [notify]);

  const go = useCallback((view: View, id?: string) => {
    setTicket(null);
    window.location.hash = "/" + view + (id ? "/" + encodeURIComponent(id) : "");
  }, []);
  const setLang = useCallback((l: Lang) => {
    setLangState(l);
    writeLocal("agenium-lang", l);
    // The config keeps it too, for what the app shows outside the window (desktop notifications).
    if (inTauri) api.setUiLanguage(l).catch(() => undefined);
  }, []);
  const t = useCallback((k: Key) => dictionaries[lang][k] ?? k, [lang]);
  const setThemeMode = useCallback((m: ThemeMode) => {
    setThemeModeState(m);
    writeLocal("agenium-theme", m);
  }, []);
  const openTicket = useCallback(
    (key: string | null, of?: string | null) => {
      const p = of ?? project;
      setTicket(key && p ? { key, project: p } : null);
    },
    [project],
  );
  const setProject = useCallback((id: string | null) => {
    setProjectState(id);
    if (id) writeLocal("agenium-project", id);
  }, []);

  // The team starting on a project and a project getting done are told, with the way there (the first load tells
  // nothing: only a change seen while the app is open).
  const runStates = useRef<Record<string, string | undefined> | null>(null);
  useEffect(() => {
    const was = runStates.current;
    runStates.current = Object.fromEntries(projects.map((p) => [p.id, p.run_state]));
    if (!was) return;
    for (const p of projects) {
      const name = p.name;
      // Started: after the requirements were agreed, or on new work for a done project.
      if ((was[p.id] === "Agreeing" || was[p.id] === "Done") && p.run_state === "Running")
        notify(t("run.startedToast").replace("{p}", name), { label: t("run.watch"), run: () => (setProject(p.id), go("live")) });
      if (was[p.id] === "Running" && p.run_state === "Done")
        notify(t("run.doneToast").replace("{p}", name), { label: t("run.review"), run: () => (setProject(p.id), go("projects")) });
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [projects]);

  const value = useMemo<Ctx>(
    () => ({
      status,
      route,
      go,
      ticket,
      openTicket,
      lang,
      setLang,
      t,
      themeMode,
      setThemeMode,
      projects,
      project,
      setProject,
      tick,
      refresh,
      approvals,
      setApprovals,
      toast,
      notify,
      reloadStatus,
    }),
    [status, route, go, ticket, openTicket, lang, setLang, t, themeMode, setThemeMode, projects, project, setProject, tick, refresh, approvals, toast, notify, reloadStatus],
  );
  return (
    <C.Provider value={value}>
      <L.Provider value={live}>{children}</L.Provider>
    </C.Provider>
  );
}

export { inTauri };
