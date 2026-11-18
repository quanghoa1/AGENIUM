// The project's map (ADR-166), laid out as BloodHound lays out its graph: the canvas fills the page; Search and Path
// float on its top left (a search keeps one kind with `kind:`), the node's panel on its right, the controls on its
// top right. The user only looks: the map is the project's agents' to add to, each piece checked by Agenium.
import { ArrowLeftRight, Copy, LocateFixed, Route, Search, Waypoints } from "lucide-react";
import { useEffect, useMemo, useRef, useState } from "react";
import { NeedsProject } from "../components/GettingStarted";
import { ProjectPicker } from "../components/ProjectPicker";
import { Empty, PageHeader, cn } from "../components/ui";
import { api } from "../lib/api";
import { useApp } from "../lib/store";
import type { MapGraph, MapNodeView, MapStats } from "../lib/types";
import { iconOf } from "./map/kinds";
import { force, sequential } from "./map/layout";
import { MapCanvas } from "./map/MapCanvas";
import { NodePanel } from "./map/NodePanel";

type Mode = "search" | "path";

/** A search box whose suggestions are the map's nodes. */
function NodeInput({ project, value, onPick, placeholder, id }: { project: string; value: MapNodeView | null; onPick: (n: MapNodeView) => void; placeholder: string; id?: string }) {
  const app = useApp();
  const [q, setQ] = useState("");
  const [hits, setHits] = useState<MapNodeView[] | null>(null);
  const timer = useRef<number | undefined>(undefined);
  useEffect(() => {
    window.clearTimeout(timer.current);
    if (!q.trim()) return setHits(null);
    timer.current = window.setTimeout(() => api.mapSearch(project, q).then(setHits).catch(() => setHits([])), 150);
  }, [q, project]);
  return (
    <div className="relative">
      <input
        id={id}
        className="input text-xs"
        placeholder={value ? value.name : placeholder}
        value={q}
        onChange={(e) => setQ(e.target.value)}
        aria-label={placeholder}
      />
      {hits && (
        <ul className="absolute z-10 mt-1 max-h-72 w-full overflow-auto rounded-lg border border-line bg-elevated p-1 shadow-md" data-testid="map-hits">
          {hits.length === 0 && <li className="px-2 py-1.5 text-[11px] text-subtle">{app.t("map.noHits")}</li>}
          {hits.map((n) => {
            const Icon = iconOf(n.kind);
            return (
              <li key={n.key}>
                <button
                  className="flex w-full items-center gap-2 rounded-md px-2 py-1.5 text-left text-xs hover:bg-surface"
                  onClick={() => {
                    onPick(n);
                    setQ("");
                    setHits(null);
                  }}
                >
                  <Icon className="h-3.5 w-3.5 shrink-0 text-accent" />
                  <span className="min-w-0 flex-1 truncate" title={n.key}>
                    {n.name}
                  </span>
                  <span className="chip">{n.kind}</span>
                </button>
              </li>
            );
          })}
        </ul>
      )}
    </div>
  );
}

export function ProjectMapView() {
  const app = useApp();
  const project = app.project;
  const p = app.projects.find((x) => x.id === project);
  const [stats, setStats] = useState<MapStats | null>(null);
  const [graph, setGraph] = useState<MapGraph>({ nodes: [], links: [], more: 0 });
  const [sel, setSel] = useState<string | null>(null);
  const [mode, setMode] = useState<Mode>("search");
  const [layoutMode, setLayoutMode] = useState<"sequential" | "force">("sequential");
  const [from, setFrom] = useState<MapNodeView | null>(null);
  const [to, setTo] = useState<MapNodeView | null>(null);
  const [path, setPath] = useState<Set<number>>(new Set());
  const [note, setNote] = useState<string | null>(null);
  const [menu, setMenu] = useState<{ key: string; x: number; y: number } | null>(null);
  const [fit, setFit] = useState(0);

  const overview = () => {
    if (!project) return;
    setPath(new Set());
    setNote(null);
    api.mapAround(project, null).then((g) => {
      setGraph(g);
      setFit((f) => f + 1);
    });
  };
  // Another project: its most connected part. The counts follow the app's events; what the user is looking at stays.
  useEffect(() => {
    if (!project) return;
    setSel(null);
    overview();
  }, [project]);
  useEffect(() => {
    if (project) api.mapStats(project).then(setStats).catch(() => setStats(null));
  }, [project, app.tick]);

  const show = (g: MapGraph, select?: string) => {
    setGraph(g);
    setPath(new Set());
    setNote(null);
    if (select !== undefined) setSel(select);
    setFit((f) => f + 1);
  };
  const around = (key: string, depth = 1) => project && api.mapAround(project, key, depth).then((g) => show(g, key));
  const findPath = async (a = from, b = to) => {
    if (!project || !a || !b) return;
    const g = await api.mapPath(project, a.key, b.key);
    if (!g) {
      setNote(app.t("map.noPath"));
      return;
    }
    setGraph(g);
    setPath(new Set(g.links.map((l) => l.id)));
    setSel(null);
    setNote(null);
    setFit((f) => f + 1);
  };

  const placed = useMemo(
    () => (layoutMode === "sequential" ? sequential(graph.nodes, graph.links, from?.key ?? sel) : force(graph.nodes, graph.links)),
    // The layout follows the graph and the mode, not every selection: the nodes stay where they are when one is clicked.
    [graph, layoutMode],
  );
  if (!project || !p) return <NeedsProject title={app.t("nav.map")} />;
  const node = sel ? graph.nodes.find((n) => n.key === sel) : undefined;
  const nodeByKey = (k: string) => graph.nodes.find((n) => n.key === k) ?? null;

  return (
    <div>
      <PageHeader title={p.name} sub={app.t("map.sub")}>
        <ProjectPicker />
        {stats && (
          <span className="text-xs text-muted" data-testid="map-stats">
            {app.t("map.counts").replace("{n}", String(stats.nodes)).replace("{l}", String(stats.links))}
            {stats.stale > 0 && <span className="ml-2 text-amber-600 dark:text-amber-400">{app.t("map.staleCount").replace("{n}", String(stats.stale))}</span>}
            {stats.tentative > 0 && <span className="ml-2">{app.t("map.tentativeCount").replace("{n}", String(stats.tentative))}</span>}
          </span>
        )}
      </PageHeader>
      {stats && stats.nodes === 0 ? (
        <Empty>{app.t("map.empty")}</Empty>
      ) : (
        <div className="relative h-[calc(100vh-170px)] min-h-[520px] overflow-hidden rounded-lg border border-line bg-surface" onClick={() => setMenu(null)}>
          <MapCanvas
            nodes={placed}
            links={graph.links}
            selected={sel}
            path={path}
            fitKey={String(fit)}
            onSelect={(k) => {
              setSel(k);
              setPath(new Set());
            }}
            onMenu={(key, x, y) => setMenu({ key, x, y })}
          />
          {/* Search and Path, top left. */}
          <div className="absolute left-3 top-3 w-72 rounded-lg border border-line bg-elevated/95 p-2 shadow-sm">
            <div className="mb-2 flex gap-1">
              {(["search", "path"] as const).map((m) => (
                <button key={m} className={cn("btn", mode === m && "border-accent/50 bg-accent/10 text-accent")} onClick={() => setMode(m)}>
                  {m === "search" ? <Search className="h-3.5 w-3.5" /> : <Route className="h-3.5 w-3.5" />} {app.t(m === "search" ? "map.search" : "map.path")}
                </button>
              ))}
            </div>
            {mode === "search" ? (
              <>
                <NodeInput id="map-search" project={project} value={null} placeholder={app.t("map.search")} onPick={(n) => around(n.key)} />
                <div className="mt-1.5 text-[11px] text-subtle">{app.t("map.searchHint")}</div>
              </>
            ) : (
              <div className="space-y-1.5">
                <NodeInput project={project} value={from} placeholder={app.t("map.from")} onPick={setFrom} />
                <NodeInput project={project} value={to} placeholder={app.t("map.to")} onPick={setTo} />
                <div className="flex gap-1">
                  <button className="btn-primary" disabled={!from || !to} onClick={() => findPath()}>
                    <Route className="h-3.5 w-3.5" /> {app.t("map.findPath")}
                  </button>
                  <button
                    className="btn"
                    onClick={() => {
                      setFrom(to);
                      setTo(from);
                      findPath(to, from);
                    }}
                  >
                    <ArrowLeftRight className="h-3.5 w-3.5" /> {app.t("map.swap")}
                  </button>
                </div>
              </div>
            )}
            {note && <div className="mt-1.5 text-[11px] text-amber-600 dark:text-amber-400">{note}</div>}
          </div>
          {/* Controls, top right (left of the panel). */}
          <div className={cn("absolute top-3 flex gap-1", node ? "right-[22rem]" : "right-3")}>
            <select className="input w-auto text-xs" aria-label={app.t("map.layout")} value={layoutMode} onChange={(e) => setLayoutMode(e.target.value as "sequential" | "force")}>
              <option value="sequential">{app.t("map.layout.sequential")}</option>
              <option value="force">{app.t("map.layout.force")}</option>
            </select>
            <button className="btn" onClick={() => setFit((f) => f + 1)}>
              <LocateFixed className="h-3.5 w-3.5" /> {app.t("map.fit")}
            </button>
            <button className="btn" onClick={overview}>
              <Waypoints className="h-3.5 w-3.5" /> {app.t("map.overview")}
            </button>
          </div>
          {node && (
            <div className="absolute bottom-3 right-3 top-3 w-[21rem]">
              <NodePanel project={project} node={node} onPick={(k) => around(k)} onShow={(g) => show(g, node.key)} />
            </div>
          )}
          {/* Legend and hint, bottom left. */}
          <div className="pointer-events-none absolute bottom-3 left-3 max-w-[50%] space-y-1 rounded-md bg-elevated/90 px-2 py-1.5 text-[11px] text-muted">
            <div className="flex flex-wrap gap-x-3">
              <span className="flex items-center gap-1"><span className="inline-block h-0.5 w-5 bg-[var(--border-strong)]" />{app.t("map.legend.verified")}</span>
              <span className="flex items-center gap-1"><span className="inline-block w-5 border-t-2 border-dashed border-[var(--border-strong)]" />{app.t("map.legend.unsure")}</span>
            </div>
            <div>{app.t("map.hint")}</div>
            {graph.more > 0 && <div>{app.t("map.more").replace("{n}", String(graph.more))}</div>}
          </div>
          {menu && (
            <ul className="fixed z-20 w-56 rounded-lg border border-line bg-elevated p-1 text-xs shadow-md" style={{ left: menu.x, top: menu.y }} data-testid="map-menu">
              {[
                { label: app.t("map.menu.start"), icon: <Route className="h-3.5 w-3.5" />, run: () => (setFrom(nodeByKey(menu.key)), setMode("path")) },
                { label: app.t("map.menu.end"), icon: <Route className="h-3.5 w-3.5" />, run: () => (setTo(nodeByKey(menu.key)), setMode("path")) },
                { label: app.t("map.menu.around"), icon: <Waypoints className="h-3.5 w-3.5" />, run: () => around(menu.key, 2) },
                {
                  label: app.t("map.menu.copy"),
                  icon: <Copy className="h-3.5 w-3.5" />,
                  run: () => navigator.clipboard?.writeText(menu.key).then(() => app.notify(app.t("map.copied"))),
                },
              ].map((i) => (
                <li key={i.label}>
                  <button
                    className="flex w-full items-center gap-2 rounded-md px-2 py-1.5 text-left hover:bg-surface"
                    onClick={(e) => {
                      e.stopPropagation();
                      i.run();
                      setMenu(null);
                    }}
                  >
                    {i.icon} {i.label}
                  </button>
                </li>
              ))}
            </ul>
          )}
        </div>
      )}
    </div>
  );
}
