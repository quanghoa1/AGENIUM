// The Map page's node panel (ADR-166), as BloodHound's entity panel: what the node is, its links out and in grouped
// by type with counts (a group can be put on the map), and the evidence of a link: the cited code as the main branch
// has it, or the page, opened in the app's Browser.
import { ChevronDown, ChevronRight, ExternalLink, Waypoints } from "lucide-react";
import { useEffect, useState } from "react";
import { cn } from "../../components/ui";
import { api } from "../../lib/api";
import { useApp } from "../../lib/store";
import type { MapGraph, MapLinkView, MapNodeView } from "../../lib/types";
import { iconOf } from "./kinds";

const isUrl = (e: string) => /^https?:\/\//.test(e);

function Group({ title, count, children, open: start = true }: { title: string; count?: number; children: React.ReactNode; open?: boolean }) {
  const [open, setOpen] = useState(start);
  return (
    <div className="border-t border-line">
      <button className="flex w-full items-center gap-1.5 px-3 py-2 text-left text-xs font-semibold" onClick={() => setOpen(!open)}>
        {open ? <ChevronDown className="h-3.5 w-3.5" /> : <ChevronRight className="h-3.5 w-3.5" />}
        <span className="flex-1">{title}</span>
        {count !== undefined && <span className="chip">{count}</span>}
      </button>
      {open && <div className="px-3 pb-2.5">{children}</div>}
    </div>
  );
}

function Evidence({ project, evidence }: { project: string; evidence: string }) {
  const app = useApp();
  const [code, setCode] = useState<string | null>(null);
  useEffect(() => {
    if (isUrl(evidence) || !evidence) return;
    api.mapCode(project, evidence).then(setCode).catch((e) => setCode(String(e)));
  }, [project, evidence]);
  if (!evidence) return null;
  if (isUrl(evidence))
    return (
      <div className="mt-1 flex flex-wrap items-center gap-2">
        <span className="break-all font-mono text-[11px] text-accent">{evidence}</span>
        <button
          className="btn"
          onClick={() => {
            try {
              localStorage.setItem(`agenium-browser-url:${project}`, evidence);
            } catch {
              /* the Browser opens its last page instead */
            }
            app.go("browser");
          }}
        >
          <ExternalLink className="h-3.5 w-3.5" /> {app.t("map.openPage")}
        </button>
      </div>
    );
  return (
    <div className="mt-1">
      <div className="font-mono text-[11px] text-subtle">{evidence}</div>
      <pre className="mt-1 max-h-48 overflow-auto rounded-md border border-line bg-surface p-2 text-[11px] leading-snug">{code ?? app.t("map.loading")}</pre>
    </div>
  );
}

export function NodePanel({
  project,
  node,
  onPick,
  onShow,
}: {
  project: string;
  node: MapNodeView;
  onPick: (key: string) => void;
  onShow: (g: MapGraph) => void;
}) {
  const app = useApp();
  const [around, setAround] = useState<MapGraph | null>(null);
  const [open, setOpen] = useState<number | null>(null);
  useEffect(() => {
    setAround(null);
    setOpen(null);
    api.mapAround(project, node.key, 1).then(setAround).catch(() => undefined);
  }, [project, node.key]);
  const Icon = iconOf(node.kind);
  const names = new Map((around?.nodes ?? []).map((n) => [n.key, n]));
  const side = (out: boolean) => {
    const ls = (around?.links ?? []).filter((l) => (out ? l.src === node.key : l.dst === node.key));
    const groups = new Map<string, MapLinkView[]>();
    for (const l of ls) groups.set(l.kind, [...(groups.get(l.kind) ?? []), l]);
    return { ls, groups: [...groups.entries()] };
  };
  const status = (s: string) =>
    s === "confirmed" ? null : (
      <span className={cn("chip", s === "stale" ? "border-amber-500/40 bg-amber-500/10 text-amber-600 dark:text-amber-400" : "")} title={app.t(`map.statusHelp.${s as "tentative" | "stale"}`)}>
        {app.t(`map.status.${s as "tentative" | "stale"}`)}
      </span>
    );
  const list = (out: boolean) => {
    const { ls, groups } = side(out);
    return (
      <Group title={app.t(out ? "map.outbound" : "map.inbound")} count={ls.length}>
        {groups.length === 0 && <div className="text-[11px] text-subtle">—</div>}
        {groups.map(([kind, gl]) => (
          <div key={kind} className="mb-1.5">
            <div className="flex items-center justify-between text-[11px] text-muted">
              <span>
                {kind.replaceAll("_", " ")} · {gl.length}
              </span>
              <button
                className="text-[11px] text-accent hover:underline"
                onClick={() => {
                  const keep = new Set([node.key, ...gl.map((l) => (out ? l.dst : l.src))]);
                  onShow({ nodes: (around?.nodes ?? []).filter((n) => keep.has(n.key)), links: gl, more: 0 });
                }}
              >
                {app.t("map.showAll")}
              </button>
            </div>
            <ul>
              {gl.map((l) => {
                const other = names.get(out ? l.dst : l.src);
                return (
                  <li key={l.id} className="rounded-md hover:bg-surface">
                    <div className="flex items-center gap-1.5 px-1 py-0.5">
                      <button className="min-w-0 flex-1 truncate text-left text-xs" title={other?.key} onClick={() => onPick(out ? l.dst : l.src)}>
                        {out ? "→ " : "← "}
                        {other?.name ?? (out ? l.dst : l.src)}
                      </button>
                      {status(l.status)}
                      <button className="text-[11px] text-subtle hover:text-fg" onClick={() => setOpen(open === l.id ? null : l.id)} aria-label={app.t("map.evidence")}>
                        {app.t("map.evidence")}
                      </button>
                    </div>
                    {open === l.id && (
                      <div className="px-1 pb-1.5">
                        <Evidence project={project} evidence={l.evidence} />
                        <div className="mt-1 text-[11px] text-subtle">
                          {app.t("map.addedBy").replace("{agent}", l.agent)}
                          {l.ticket ? ` ${app.t("map.inTicket").replace("{ticket}", l.ticket)}` : ""} · {new Date(l.added_at).toLocaleString()}
                        </div>
                      </div>
                    )}
                  </li>
                );
              })}
            </ul>
          </div>
        ))}
      </Group>
    );
  };
  return (
    <div className="flex max-h-full flex-col overflow-auto rounded-lg border border-line bg-elevated/95 text-xs shadow-sm" data-testid="map-node-panel">
      <div className="flex items-start gap-2.5 p-3">
        <Icon className="mt-0.5 h-5 w-5 shrink-0 text-accent" />
        <div className="min-w-0 flex-1">
          <div className="break-words text-[13px] font-semibold leading-snug">{node.name}</div>
          <div className="mt-0.5 flex flex-wrap items-center gap-1.5 text-[11px] text-muted">
            <span>{node.kind}</span>
            {status(node.status) ?? <span className="text-emerald-600 dark:text-emerald-400">{app.t("map.status.confirmed")}</span>}
          </div>
        </div>
      </div>
      <Group title={app.t("map.info")}>
        {node.summary && <div className="mb-1.5 leading-relaxed text-muted">{node.summary}</div>}
        {node.file && (
          <div className="font-mono text-[11px] text-subtle">
            {node.file}
            {node.lines ? `:${node.lines[0]}-${node.lines[1]}` : ""}
          </div>
        )}
        <div className="mt-1 text-[11px] text-subtle">
          {app.t("map.addedBy").replace("{agent}", node.agent)}
          {node.ticket ? ` ${app.t("map.inTicket").replace("{ticket}", node.ticket)}` : ""} · {app.t("map.checked").replace("{at}", new Date(node.checked_at).toLocaleString())}
        </div>
      </Group>
      {list(true)}
      {list(false)}
      {node.evidence && (
        <Group title={app.t("map.evidence")} open={false}>
          <Evidence project={project} evidence={node.evidence} />
        </Group>
      )}
      <div className="border-t border-line px-3 py-2 text-[11px] text-subtle">
        <Waypoints className="mr-1 inline h-3 w-3" />
        {node.key}
      </div>
    </div>
  );
}
