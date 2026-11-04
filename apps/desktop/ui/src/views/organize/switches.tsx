import { Plug, Wrench } from "lucide-react";
import { useEffect, useState } from "react";
import { cn } from "../../components/ui";
import { api } from "../../lib/api";
import { useApp } from "../../lib/store";
import type { LibrarySkill, McpServerRow } from "../../lib/types";

/** Switches for the library skills and the system's MCP servers. */
export function Switches({ skills, mcp, onSkills, onMcp, lib, servers }: { skills: string[]; mcp: string[]; onSkills: (name: string, on: boolean) => void; onMcp: (id: string, on: boolean) => void; lib: LibrarySkill[]; servers: McpServerRow[] }) {
  const app = useApp();
  // Filter the library skills to enable/disable: by tag, and by a name/description search. "on" shows only enabled.
  const [tag, setTag] = useState<string | null>(null);
  const [q, setQ] = useState("");
  const allTags = lib.flatMap((s) => s.tags ?? []).filter((t, i, a) => a.indexOf(t) === i).sort();
  const shown = lib.filter(
    (s) =>
      (tag === null || (s.tags ?? []).includes(tag)) &&
      (q.trim() === "" || (s.name + " " + s.description).toLowerCase().includes(q.trim().toLowerCase())),
  );
  return (
    <div className="grid gap-3 md:grid-cols-2">
      <div>
        <div className="label mb-1.5 flex items-center gap-1">
          <Wrench className="h-3 w-3" /> {app.t("agents.librarySkills")}
          <span className="ml-auto font-mono text-[10px] text-subtle">
            {skills.filter((s) => !s.startsWith("plugin:")).length}/{lib.length}
          </span>
        </div>
        {lib.length > 3 && (
          <div className="mb-1.5 space-y-1.5">
            <input className="input h-7 w-full py-0 text-[11px]" placeholder={app.t("agents.filterSkills")} value={q} onChange={(e) => setQ(e.target.value)} />
            {allTags.length > 0 && (
              <div className="flex flex-wrap gap-1">
                <button className={cn("chip", tag === null ? "border-accent text-accent" : "border-line-strong text-muted")} onClick={() => setTag(null)}>
                  {app.t("library.allTags")}
                </button>
                {allTags.map((t) => (
                  <button key={t} className={cn("chip", tag === t ? "border-indigo-500/50 bg-indigo-500/10 text-indigo-600 dark:text-indigo-300" : "border-line-strong text-muted")} onClick={() => setTag(tag === t ? null : t)}>
                    {t}
                  </button>
                ))}
              </div>
            )}
          </div>
        )}
        <div className="space-y-1.5">
          {shown.map((s) => (
            <Toggle key={s.name} on={skills.includes(s.name)} onChange={(v) => onSkills(s.name, v)} title={s.name} sub={s.description} tags={s.tags} />
          ))}
          {lib.length === 0 && (
            <button className="text-[11px] text-accent hover:underline" onClick={() => app.go("library")}>
              {app.t("agents.noLibrary")}
            </button>
          )}
          {lib.length > 0 && shown.length === 0 && <div className="text-[11px] text-subtle">{app.t("agents.noMatch")}</div>}
        </div>
      </div>
      <div>
        <div className="label mb-1.5 flex items-center gap-1">
          <Plug className="h-3 w-3" /> MCP
          <span className="ml-auto font-mono text-[10px] text-subtle">
            {mcp.length}/{servers.length}
          </span>
        </div>
        <div className="space-y-1.5">
          {servers.map((s) => (
            <Toggle key={s.config.id} on={mcp.includes(s.config.id)} onChange={(v) => onMcp(s.config.id, v)} title={s.config.id} sub={s.config.url ?? s.config.command ?? ""} />
          ))}
          {servers.length === 0 && (
            <button className="text-[11px] text-accent hover:underline" onClick={() => app.go("library")}>
              {app.t("agents.noServers")}
            </button>
          )}
        </div>
      </div>
    </div>
  );
}

export function Toggle({ on, onChange, title, sub, tags }: { on: boolean; onChange: (v: boolean) => void; title: string; sub?: string; tags?: string[] }) {
  return (
    <button type="button" onClick={() => onChange(!on)} className={cn("press flex w-full items-center gap-3 rounded-lg border p-2 text-left", on ? "border-accent/50 bg-blue-600/10" : "border-line hover:border-line-strong")}>
      <span className={cn("relative inline-flex h-4 w-7 shrink-0 rounded-full transition-colors", on ? "bg-accent" : "bg-line-strong")}>
        <span className={cn("absolute top-0.5 h-3 w-3 rounded-full bg-white shadow transition-all", on ? "left-3.5" : "left-0.5")} />
      </span>
      <span className="min-w-0 flex-1">
        <span className="flex items-center gap-1.5">
          <span className="truncate text-xs font-medium">{title}</span>
          {/* The skill's domain tags, so its field is visible while enabling it. */}
          {tags?.map((t) => (
            <span key={t} className="chip shrink-0 border-indigo-500/30 py-0 text-indigo-600 dark:text-indigo-300">
              {t}
            </span>
          ))}
        </span>
        {sub && <span className="block truncate text-[11px] text-subtle">{sub}</span>}
      </span>
    </button>
  );
}

export function useLibrary(tick: number) {
  const [lib, setLib] = useState<LibrarySkill[]>([]);
  const [servers, setServers] = useState<McpServerRow[]>([]);
  useEffect(() => {
    api.skillLibrary().then((l) => setLib(l?.skills ?? []));
    api.mcpServers().then((s) => setServers(s ?? []));
  }, [tick]);
  return { lib, servers };
}
