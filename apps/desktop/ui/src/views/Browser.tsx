// The preview browser (ADR-121): tabs showing the apps the agents build, or any web page worth pointing at, with an
// element picker. Each tab belongs to a project (several projects and teams can be worked on at once); its picked
// elements gather on the right with a note each and go to that project's Lead in one message, with what lets an agent
// find them: selector, tag and classes, text, HTML, computed styles, size and place, and the component and source
// file when the page is a development build. A page outside the user's machine is sent as a reference to follow.
import { ArrowLeft, ArrowRight, Copy, Crosshair, Globe, Link2, Pencil, Plus, RotateCw, Send, Trash2, X } from "lucide-react";
import { useCallback, useEffect, useLayoutEffect, useRef, useState } from "react";
import { Annotate } from "../components/Annotate";
import { NeedsProject } from "../components/GettingStarted";
import { Overlay, useCovered } from "../components/motion";
import { Empty, Mono, cn } from "../components/ui";
import { api, onBrowser } from "../lib/api";
import { inTauri, useApp } from "../lib/store";

/** What the picker sends for one element (see apps/desktop/src/browser_picker.js). */
export interface Picked {
  url: string;
  title: string;
  viewport: [number, number];
  selector: string;
  element: string;
  tag: string;
  classes: string[];
  attrs: Record<string, string>;
  text: string;
  html: string;
  rect: [number, number, number, number];
  styles: Record<string, string>;
  component: { framework: string; name: string | null; file: string | null } | null;
  parent: string | null;
  /** "element" (a click) or "region" (a drag: a blank area or several elements). */
  kind?: "element" | "region";
  dpr?: number;
  /** The elements a region holds. */
  elements?: { element: string; selector: string; text: string }[];
  /** Screenshot of the element or region (PNG, base64), annotated or not; saved into the project when sent. */
  image?: string | null;
  /** Ties a screenshot that arrives later to its pick. */
  key?: string;
  note: string;
}

interface Tab {
  id: string;
  project: string;
  /** The address typed in the bar. */
  url: string;
  /** The page the tab shows (after redirects), once it loaded. */
  current: string | null;
  opened: boolean;
  picking: boolean;
  picks: Picked[];
  message: string;
}

// Tabs outlive the page: leaving the browser and coming back keeps them (their pages stay open, hidden).
let kept: { tabs: Tab[]; active: string | null } = { tabs: [], active: null };
let seq = 0;
let autoOpened = false;
const newId = () => `t${Date.now().toString(36)}${(seq++).toString(36)}`;

const urlKey = (project: string) => `agenium-browser-url:${project}`;
const readUrl = (project: string) => {
  try {
    return localStorage.getItem(urlKey(project)) || "http://localhost:3000";
  } catch {
    return "http://localhost:3000";
  }
};

// Which project's app a local address serves (`origin` -> project id): a tab on that origin belongs to the project.
const ORIGINS = "agenium-browser-origins";
function origins(): Record<string, string> {
  try {
    return JSON.parse(localStorage.getItem(ORIGINS) || "{}");
  } catch {
    return {};
  }
}
function linkOrigin(url: string, project: string) {
  try {
    const o = new URL(url).origin;
    localStorage.setItem(ORIGINS, JSON.stringify({ ...origins(), [o]: project }));
  } catch {
    /* per-viewer convenience only */
  }
}
/** The project whose app `url` is, if the user said so. */
export function projectOf(url: string | null): string | null {
  if (!url) return null;
  try {
    return origins()[new URL(url).origin] ?? null;
  } catch {
    return null;
  }
}

/** A page served from this machine (the app being built), as opposed to a site to take as a reference. */
export function isLocal(url: string): boolean {
  try {
    const h = new URL(url).hostname;
    return h === "localhost" || h === "127.0.0.1" || h === "[::1]" || h.endsWith(".localhost") || h.endsWith(".local");
  } catch {
    return false;
  }
}

/** The message the Lead gets: the user's words first, then each element with what locates it. */
export function pickMessage(message: string, picks: Picked[], shots: (string | null)[] = []): string {
  const parts: string[] = [];
  if (message.trim()) parts.push(message.trim());
  const page = picks[0];
  parts.push(
    isLocal(page.url)
      ? `Elements picked in the preview browser (${page.url}, viewport ${page.viewport[0]}×${page.viewport[1]}):`
      : `Elements picked on an external site as a reference — not part of this project (${page.url}). Apply what each note asks to our project, taking the look and structure from these:`,
  );
  picks.forEach((p, i) => {
    const lines = [`${i + 1}. ${p.kind === "region" ? "Region: " : ""}${p.element}${p.text ? ` "${p.text.slice(0, 120)}"` : ""}`];
    if (p.note.trim()) lines.push(`   ${isLocal(p.url) ? "Change" : "Wanted"}: ${p.note.trim()}`);
    if (shots[i]) lines.push(`   Screenshot (with the user's marks if any; view it with view_image): ${shots[i]}`);
    lines.push(`   ${p.kind === "region" ? "Inside" : "Selector"}: ${p.selector}`);
    for (const e of p.elements ?? []) lines.push(`   - ${e.element} (${e.selector})${e.text ? ` "${e.text}"` : ""}`);
    if (p.component) lines.push(`   Component: ${[p.component.name, p.component.file].filter(Boolean).join(" at ")} (${p.component.framework})`);
    if (p.url !== page.url) lines.push(`   Page: ${p.url}`);
    lines.push(`   Box: ${p.rect[2]}×${p.rect[3]} at (${p.rect[0]}, ${p.rect[1]})`);
    const st = Object.entries(p.styles).map(([k, v]) => `${k}: ${v}`).join("; ");
    if (st) lines.push(`   Styles: ${st}`);
    const at = Object.entries(p.attrs).map(([k, v]) => `${k}="${v}"`).join(" ");
    if (at) lines.push(`   Attributes: ${at}`);
    lines.push("   HTML:", "   ```html", ...p.html.split("\n").map((l) => `   ${l}`), "   ```");
    parts.push(lines.join("\n"));
  });
  return parts.join("\n\n");
}

const hostOf = (u: string | null) => {
  if (!u) return null;
  try {
    return new URL(u).host;
  } catch {
    return u;
  }
};

export function BrowserView() {
  const app = useApp();
  const area = useRef<HTMLDivElement>(null);
  const [tabs, setTabsState] = useState<Tab[]>(kept.tabs);
  const [active, setActiveState] = useState<string | null>(kept.active);
  const [sending, setSending] = useState(false);
  // The pick whose screenshot is being drawn on (the native page hides meanwhile: it would cover the dialog).
  const [drawing, setDrawing] = useState<{ tab: string; index: number } | null>(null);
  const tabsRef = useRef(tabs);
  tabsRef.current = tabs;
  const activeRef = useRef(active);
  activeRef.current = active;

  const setTabs = useCallback((f: (t: Tab[]) => Tab[]) => {
    setTabsState((t) => {
      const next = f(t);
      kept = { ...kept, tabs: next };
      return next;
    });
  }, []);
  const patch = useCallback((id: string, p: Partial<Tab>) => setTabs((ts) => ts.map((t) => (t.id === id ? { ...t, ...p } : t))), [setTabs]);
  const bounds = useCallback(() => {
    const r = area.current?.getBoundingClientRect();
    return r ? { x: r.left, y: r.top, w: r.width, h: r.height } : null;
  }, []);

  const select = useCallback(
    (id: string | null) => {
      const prev = activeRef.current;
      if (prev && prev !== id) {
        api.browserVisible(prev, false).catch(() => undefined);
        api.browserAction(prev, "pick-off").catch(() => undefined);
        patch(prev, { picking: false });
      }
      kept = { ...kept, active: id };
      activeRef.current = id;
      setActiveState(id);
      if (id) {
        const b = bounds();
        if (b) api.browserBounds(id, b.x, b.y, b.w, b.h).catch(() => undefined);
        api.browserVisible(id, true).catch(() => undefined);
      }
    },
    [bounds, patch],
  );

  const addTab = useCallback(
    (project: string | null) => {
      if (!project) return;
      const t: Tab = { id: newId(), project, url: readUrl(project), current: null, opened: false, picking: false, picks: [], message: "" };
      setTabs((ts) => [...ts, t]);
      select(t.id);
    },
    [select, setTabs],
  );

  // The first visit opens a tab for the current project (once: React may run this effect twice).
  useEffect(() => {
    if (!autoOpened && kept.tabs.length === 0 && tabsRef.current.length === 0 && app.project) {
      autoOpened = true;
      // Pages left from before the interface reloaded belong to no tab any more.
      api
        .browserCloseAll()
        .catch(() => undefined)
        .finally(() => addTab(app.project));
    }
  }, [addTab, app.project]);

  // Hidden while anything is drawn over the page (the native view covers HTML): a palette, a ticket, a dialog.
  const covered = useCovered();
  useEffect(() => {
    if (!inTauri) return;
    const id = activeRef.current;
    if (id) api.browserVisible(id, !covered).catch(() => undefined);
  }, [covered]);

  // The active tab's native view follows its place in the page; all hide when the page goes away.
  useLayoutEffect(() => {
    if (!inTauri || !area.current) return;
    const sync = () => {
      const id = activeRef.current;
      const b = bounds();
      if (id && b) api.browserBounds(id, b.x, b.y, b.w, b.h).catch(() => undefined);
    };
    const ro = new ResizeObserver(sync);
    ro.observe(area.current);
    window.addEventListener("resize", sync);
    // Back on this page: the tabs' pages are still open.
    for (const t of tabsRef.current) {
      api
        .browserUrl(t.id)
        .then((u) => {
          if (u && u !== "about:blank") patch(t.id, { opened: true, current: u });
        })
        .catch(() => undefined);
    }
    if (activeRef.current) api.browserVisible(activeRef.current, true).catch(() => undefined);
    sync();
    return () => {
      ro.disconnect();
      window.removeEventListener("resize", sync);
      api.browserHideAll().catch(() => undefined);
      for (const t of tabsRef.current) if (t.picking) api.browserAction(t.id, "pick-off").catch(() => undefined);
      setTabs((ts) => ts.map((t) => ({ ...t, picking: false })));
    };
  }, [bounds, patch, setTabs]);

  useEffect(() => {
    // Subscribing is async: a cleanup that runs first (React's dev double mount) unsubscribes once it resolves.
    let off: (() => void) | undefined;
    let gone = false;
    onBrowser({
      url: (tab, u) => {
        const owner = isLocal(u) ? projectOf(u) : null;
        patch(tab, owner ? { current: u, opened: true, project: owner } : { current: u, opened: true });
        // A new page has a fresh picker: turn it on again if the user is picking there.
        if (tabsRef.current.find((t) => t.id === tab)?.picking) setTimeout(() => api.browserAction(tab, "pick-on").catch(() => undefined), 300);
      },
      picked: (tab, raw) => {
        try {
          const p = JSON.parse(raw) as Omit<Picked, "note">;
          const key = `${p.selector}@${Date.now()}`;
          setTabs((ts) => ts.map((t) => (t.id === tab ? { ...t, picks: [...t.picks, { ...p, note: "", image: null, key }] } : t)));
          // Its screenshot: the box of the element or region, with a margin.
          const [x, y, w, h] = p.rect;
          api
            .browserCapture(tab, x, y, w, h, p.dpr ?? 1)
            .then((png) => setTabs((ts) => ts.map((t) => (t.id === tab ? { ...t, picks: t.picks.map((q) => (q.key === key ? { ...q, image: png } : q)) } : t))))
            .catch(() => undefined);
        } catch {
          /* not a pick */
        }
      },
      pickOff: (tab) => patch(tab, { picking: false }),
    }).then((u) => {
      if (gone) u();
      else off = u;
    });
    return () => {
      gone = true;
      off?.();
    };
  }, [patch, setTabs]);

  const tab = tabs.find((t) => t.id === active) ?? null;

  const go = () => {
    const b = bounds();
    if (!tab || !b) return;
    let u = tab.url.trim();
    if (!/^https?:\/\//i.test(u)) u = `${/^(localhost|127\.|\[::1\])/.test(u) ? "http" : "https"}://${u}`;
    patch(tab.id, { url: u });
    if (isLocal(u)) {
      try {
        localStorage.setItem(urlKey(tab.project), u);
      } catch {
        /* per-viewer convenience only */
      }
    }
    api
      .browserOpen(tab.id, u, b.x, b.y, b.w, b.h)
      .then(() => patch(tab.id, { opened: true }))
      .catch((e) => app.notify(String(e)));
  };

  const togglePick = useCallback(() => {
    const t = tabsRef.current.find((x) => x.id === activeRef.current);
    if (!t || !t.opened) return;
    patch(t.id, { picking: !t.picking });
    api.browserAction(t.id, t.picking ? "pick-off" : "pick-on").catch(() => undefined);
  }, [patch]);

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.ctrlKey && e.shiftKey && e.key.toLowerCase() === "d") {
        e.preventDefault();
        togglePick();
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [togglePick]);

  const closeTab = (id: string) => {
    api.browserClose(id).catch(() => undefined);
    const rest = tabsRef.current.filter((t) => t.id !== id);
    if (rest.length === 0) autoOpened = false;
    setTabs(() => rest);
    if (activeRef.current === id) {
      kept = { ...kept, active: null };
      activeRef.current = null;
      setActiveState(null);
      if (rest.length) select(rest[rest.length - 1].id);
    }
  };

  const send = () => {
    if (!tab || tab.picks.length === 0) return;
    setSending(true);
    Promise.all(tab.picks.map((p) => (p.image ? api.browserSavePick(tab.project, p.image).catch(() => null) : Promise.resolve(null))))
      .then((shots) => api.chat(tab.project, pickMessage(tab.message, tab.picks, shots)))
      .then(() => {
        app.notify(app.t("browser.sent"));
        patch(tab.id, { picks: [], message: "" });
      })
      .catch((e) => app.notify(String(e)))
      .finally(() => setSending(false));
  };

  if (!app.project && tabs.length === 0) return <NeedsProject title={app.t("nav.browser")} />;
  if (!inTauri) return <Empty>{app.t("browser.desktopOnly")}</Empty>;
  const projectName = (id: string) => app.projects.find((p) => p.id === id)?.name ?? id;
  const external = !!tab?.current && !isLocal(tab.current);

  return (
    <div className="flex h-[calc(100vh-7.5rem)] min-h-[480px] gap-3">
      <div className="flex min-w-0 flex-1 flex-col overflow-hidden rounded-xl border border-line bg-surface">
        <div className="flex items-end gap-1 overflow-x-auto border-b border-line bg-elevated/40 px-2 pt-1.5">
          {tabs.map((t) => (
            <div
              key={t.id}
              className={cn(
                "group flex max-w-[220px] cursor-pointer items-center gap-1.5 rounded-t-lg border border-b-0 px-2.5 py-1.5 text-xs",
                t.id === active ? "border-line bg-surface" : "border-transparent text-muted hover:bg-surface/60",
              )}
              onClick={() => select(t.id)}
              title={t.current ?? t.url}
            >
              <Globe className="h-3 w-3 shrink-0" />
              <span className="truncate">{hostOf(t.current) ?? app.t("browser.newTab")}</span>
              <TabKind t={t} name={projectName} />
              {t.picks.length > 0 && <span className="shrink-0 rounded-full bg-accent/15 px-1.5 text-[10px] text-accent">{t.picks.length}</span>}
              <button
                className="shrink-0 rounded p-0.5 opacity-60 hover:bg-elevated hover:opacity-100"
                title={app.t("common.close")}
                onClick={(e) => {
                  e.stopPropagation();
                  closeTab(t.id);
                }}
              >
                <X className="h-3 w-3" />
              </button>
            </div>
          ))}
          <button className="mb-1 ml-1 rounded p-1 text-muted hover:bg-elevated" title={app.t("browser.newTab")} onClick={() => addTab(app.project ?? tab?.project ?? null)}>
            <Plus className="h-3.5 w-3.5" />
          </button>
        </div>
        {tab && (
          <div className="flex items-center gap-1.5 border-b border-line p-2">
            <button className="btn px-2" title={app.t("browser.back")} disabled={!tab.opened} onClick={() => api.browserAction(tab.id, "back")}>
              <ArrowLeft className="h-3.5 w-3.5" />
            </button>
            <button className="btn px-2" title={app.t("browser.forward")} disabled={!tab.opened} onClick={() => api.browserAction(tab.id, "forward")}>
              <ArrowRight className="h-3.5 w-3.5" />
            </button>
            <button className="btn px-2" title={app.t("browser.reload")} disabled={!tab.opened} onClick={() => api.browserAction(tab.id, "reload")}>
              <RotateCw className="h-3.5 w-3.5" />
            </button>
            <select
              className="input w-36 text-xs"
              title={app.t("browser.tabProject")}
              value={tab.project}
              onChange={(e) => {
                if (tab.current && isLocal(tab.current)) linkOrigin(tab.current, e.target.value);
                patch(tab.id, { project: e.target.value });
              }}
            >
              {app.projects.map((p) => (
                <option key={p.id} value={p.id}>
                  {p.name}
                </option>
              ))}
            </select>
            <form
              className="flex min-w-0 flex-1 items-center gap-1.5"
              onSubmit={(e) => {
                e.preventDefault();
                go();
              }}
            >
              <input className="input min-w-0 flex-1 font-mono text-xs" value={tab.url} onChange={(e) => patch(tab.id, { url: e.target.value })} placeholder="http://localhost:3000" />
              <button className="btn" type="submit">
                {app.t("browser.open")}
              </button>
            </form>
            <button className={cn("btn", tab.picking && "border-accent bg-accent/10 text-accent")} title="Ctrl+Shift+D" disabled={!tab.opened} onClick={togglePick}>
              <Crosshair className="h-3.5 w-3.5" /> {tab.picking ? app.t("browser.picking") : app.t("browser.pick")}
            </button>
          </div>
        )}
        <div ref={area} className="relative flex-1 bg-background">
          {(!tab || !tab.opened) && <div className="absolute inset-0 flex items-center justify-center p-6 text-center text-xs text-subtle">{app.t("browser.empty")}</div>}
        </div>
        {tab?.current && (
          <div className="flex items-center gap-2 truncate border-t border-line px-3 py-1 font-mono text-[11px] text-subtle">
            {external && <span className="chip shrink-0 border-amber-500/30 bg-amber-500/10 font-sans text-amber-600 dark:text-amber-300">{app.t("browser.reference")}</span>}
            {!external && !projectOf(tab.current) && (
              <button
                className="chip shrink-0 border-line-strong bg-elevated font-sans text-muted hover:text-fg"
                title={app.t("browser.linkHint")}
                onClick={() => (linkOrigin(tab.current!, tab.project), patch(tab.id, {}))}
              >
                <Link2 className="h-3 w-3" /> {app.t("browser.link").replace("{p}", projectName(tab.project))}
              </button>
            )}
            <span className="truncate">{tab.current}</span>
          </div>
        )}
      </div>

      <div className="flex w-[360px] shrink-0 flex-col overflow-hidden rounded-xl border border-line bg-surface">
        <div className="border-b border-line px-3 py-2">
          <div className="text-xs font-semibold">{app.t("browser.picked")}</div>
          <div className="mt-0.5 text-[11px] text-subtle">{external ? app.t("browser.referenceHint") : app.t("browser.pickedHint")}</div>
        </div>
        <div className="flex-1 space-y-2 overflow-y-auto p-2">
          {(!tab || tab.picks.length === 0) && <div className="p-3 text-center text-[11px] text-subtle">{app.t("browser.none")}</div>}
          {tab?.picks.map((p, i) => (
            <div key={i} className="rounded-lg border border-line bg-elevated/50 p-2">
              <div className="flex items-center justify-between gap-2">
                <Mono className="truncate text-xs font-semibold text-cyan-600 dark:text-cyan-400">
                  {i + 1}. {p.element}
                </Mono>
                <div className="flex shrink-0 gap-1">
                  {/* The element as an agent gets it (selector, HTML, styles, note), to paste anywhere. */}
                  <button
                    className="btn px-1.5"
                    title={app.t("browser.copy")}
                    onClick={() => navigator.clipboard.writeText(pickMessage("", [p]).split("\n\n").slice(1).join("\n\n")).then(() => app.notify(app.t("browser.copied")))}
                  >
                    <Copy className="h-3 w-3" />
                  </button>
                  <button className="btn px-1.5" title={app.t("common.delete")} onClick={() => patch(tab.id, { picks: tab.picks.filter((_, j) => j !== i) })}>
                    <Trash2 className="h-3 w-3" />
                  </button>
                </div>
              </div>
              {p.text && <div className="mt-1 line-clamp-2 text-[11px] text-muted">"{p.text}"</div>}
              {p.component && <div className="mt-1 truncate font-mono text-[10px] text-emerald-600 dark:text-emerald-400">{[p.component.name, p.component.file].filter(Boolean).join(" · ")}</div>}
              <div className="mt-1 truncate font-mono text-[10px] text-subtle" title={p.selector}>
                {!isLocal(p.url) && `${hostOf(p.url)} · `}
                {p.selector}
              </div>
              {p.image === null && <div className="mt-1.5 h-10 animate-pulse rounded bg-elevated" />}
              {p.image && (
                <button className="group relative mt-1.5 block w-full overflow-hidden rounded border border-line" title={app.t("browser.draw")} onClick={() => (api.browserVisible(tab.id, false), setDrawing({ tab: tab.id, index: i }))}>
                  <img className="max-h-40 w-full object-contain" src={`data:image/png;base64,${p.image}`} alt="" />
                  <span className="absolute right-1 top-1 flex items-center gap-1 rounded bg-black/60 px-1.5 py-0.5 text-[10px] text-white opacity-80 group-hover:opacity-100">
                    <Pencil className="h-3 w-3" /> {app.t("browser.draw")}
                  </span>
                </button>
              )}
              <textarea
                className="input mt-1.5 h-14 w-full resize-none text-xs"
                placeholder={isLocal(p.url) ? app.t("browser.notePlaceholder") : app.t("browser.referencePlaceholder")}
                value={p.note}
                onChange={(e) => patch(tab.id, { picks: tab.picks.map((x, j) => (j === i ? { ...x, note: e.target.value } : x)) })}
              />
            </div>
          ))}
        </div>
        {tab && (
          <div className="space-y-2 border-t border-line p-2">
            <textarea className="input h-16 w-full resize-none text-xs" placeholder={app.t("browser.messagePlaceholder")} value={tab.message} onChange={(e) => patch(tab.id, { message: e.target.value })} />
            <button className="btn-primary w-full justify-center" disabled={tab.picks.length === 0 || sending} onClick={send}>
              <Send className="h-3.5 w-3.5" />{" "}
              {sending ? app.t("browser.sending") : app.t("browser.send").replace("{n}", String(tab.picks.length)).replace("{p}", projectName(tab.project))}
            </button>
          </div>
        )}
      </div>
      <Overlay open={!!drawing} onClose={() => (drawing && api.browserVisible(drawing.tab, true), setDrawing(null))} kind="dialog" top="pt-[4vh]">
        <div className="card p-4 shadow-2xl">
          {drawing && (() => {
            const t = tabs.find((x) => x.id === drawing.tab);
            const p = t?.picks[drawing.index];
            if (!t || !p?.image) return null;
            const close = () => (api.browserVisible(t.id, true), setDrawing(null));
            return (
              <Annotate
                image={p.image}
                onCancel={close}
                onDone={(png) => (patch(t.id, { picks: t.picks.map((q, j) => (j === drawing.index ? { ...q, image: png } : q)) }), close())}
              />
            );
          })()}
        </div>
      </Overlay>
    </div>
  );
}

/** What a tab is: the app of a project (its address was linked to it), a reference site, or a local page no project
 * claims yet. */
function TabKind({ t, name }: { t: Tab; name: (id: string) => string }) {
  const app = useApp();
  if (!t.current) return <span className="shrink-0 rounded bg-elevated px-1 font-mono text-[10px] text-subtle">{name(t.project)}</span>;
  if (!isLocal(t.current)) return <span className="shrink-0 rounded bg-amber-500/15 px-1 text-[10px] text-amber-600 dark:text-amber-300">{app.t("browser.reference")}</span>;
  const owner = projectOf(t.current);
  return owner ? (
    <span className="shrink-0 rounded bg-cyan-500/15 px-1 font-mono text-[10px] text-cyan-600 dark:text-cyan-400">{name(owner)}</span>
  ) : (
    <span className="shrink-0 rounded border border-dashed border-line-strong px-1 text-[10px] text-subtle">{app.t("browser.unlinked")}</span>
  );
}
