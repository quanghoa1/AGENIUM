import { Package } from "lucide-react";
import { useEffect, useState } from "react";
import { Empty, Section } from "../../components/ui";
import { api } from "../../lib/api";
import { useApp } from "../../lib/store";
import type { Plugin } from "../../lib/types";
import { Toggle } from "./switches";

/** The agent's Plugin tab (ADR-157): each installed plugin on or off whole, its skills and servers with it. */
export function AgentPlugins({ agent, enabled, onToggle }: { agent: string; enabled: string[]; onToggle: (name: string, on: boolean) => void }) {
  const app = useApp();
  const [list, setList] = useState<Plugin[] | null>(null);
  useEffect(() => {
    api
      .plugins()
      .then((xs) => setList(xs.map((x) => x.plugin)))
      .catch((e) => app.notify(String(e)));
  }, [agent, app.tick]);
  if (!list) return <div className="text-xs text-subtle">{app.t("common.loading")}</div>;
  return (
    <Section title={app.t("plugins.agentTitle")}>
      <div className="mb-2 text-[11px] text-subtle">{app.t("plugins.agentNote")}</div>
      <div className="space-y-1.5">
        {list.map((p) => {
          const skills = Object.keys(p.skills);
          const sub = [
            app.t("plugins.counts").replace("{skills}", String(skills.length)).replace("{mcp}", String(p.mcp.length)),
            skills.slice(0, 6).join(", ") + (skills.length > 6 ? "…" : ""),
          ]
            .filter(Boolean)
            .join(" · ");
          return (
            <Toggle
              key={p.name}
              on={enabled.includes(`plugin:${p.name}`)}
              onChange={(v) => onToggle(p.name, v)}
              title={`${p.name}${p.version ? ` v${p.version}` : ""}`}
              sub={sub}
            />
          );
        })}
        {list.length === 0 && (
          <Empty>
            <button className="inline-flex items-center gap-1 text-accent hover:underline" onClick={() => app.go("library")}>
              <Package className="h-3 w-3" /> {app.t("plugins.none")}
            </button>
          </Empty>
        )}
      </div>
    </Section>
  );
}
