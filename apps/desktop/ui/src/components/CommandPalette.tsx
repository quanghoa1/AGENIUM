import { ArrowRight, Search } from "lucide-react";
import { useEffect, useMemo, useState } from "react";
import { api } from "../lib/api";
import { useApp, type View } from "../lib/store";
import { motion } from "motion/react";
import { spring } from "./motion";
import type { AgentEntry, Card } from "../lib/types";
import { cn } from "./ui";

interface Hit {
  cat: string;
  title: string;
  sub: string;
  go: () => void;
}

export function CommandPalette({ onClose }: { onClose: () => void }) {
  const app = useApp();
  const [q, setQ] = useState("");
  // The selected result's background glides with the arrow keys.
  const highlight = spring.highlight;
  const [sel, setSel] = useState(0);
  const [cards, setCards] = useState<Card[]>([]);
  const [agents, setAgents] = useState<AgentEntry[]>([]);

  useEffect(() => {
    if (app.project) api.board(app.project).then((cols) => setCards(cols.flatMap((c) => c.cards)));
    api.agents().then(setAgents);
  }, [app.project]);

  const hits = useMemo<Hit[]>(() => {
    const s = q.trim().toLowerCase();
    const m = (...v: (string | null | undefined)[]) => !s || v.some((x) => x?.toLowerCase().includes(s));
    const views: [View, string][] = [
      ["overview", app.t("nav.overview")],
      ["needs", app.t("nav.needs")],
      ["tickets", app.t("nav.tickets")],
      ["runs", app.t("nav.runs")],
      ["chat", app.t("top.talk")],
      ["agents", app.t("nav.agents")],
      ["teams", app.t("nav.teams")],
      ["settings", app.t("nav.settings")],
    ];
    const out: Hit[] = [];
    for (const [v, label] of views) if (m(label, v)) out.push({ cat: "VIEW", title: label, sub: v, go: () => app.go(v) });
    for (const c of cards) if (m(c.key, c.title)) out.push({ cat: "TICKET", title: `${c.key} ${c.title}`, sub: c.assignee ?? "–", go: () => app.go("ticket", c.key) });
    for (const a of agents) if (m(a.agent.name, a.agent.role)) out.push({ cat: "AGENT", title: a.agent.name, sub: `${a.agent.role} · ${a.project_name}`, go: () => app.go("agent", a.agent.id) });
    for (const p of app.projects) if (m(p.name, p.key_prefix)) out.push({ cat: "PROJECT", title: `${p.key_prefix} — ${p.name}`, sub: p.status, go: () => (app.setProject(p.id), app.go("tickets")) });
    return out.slice(0, 14);
  }, [q, cards, agents, app]);

  useEffect(() => setSel(0), [q]);

  const run = (h?: Hit) => {
    if (!h) return;
    h.go();
    onClose();
  };

  return (
    <div>
      <div className="card w-[640px] max-w-[92vw] overflow-hidden shadow-2xl">
        <div className="flex items-center gap-2 border-b border-line px-4 py-3">
          <Search className="h-4 w-4 text-subtle" />
          <input
            autoFocus
            className="flex-1 bg-transparent text-sm outline-none placeholder:text-subtle"
            placeholder={app.t("top.search")}
            value={q}
            onChange={(e) => setQ(e.target.value)}
            onKeyDown={(e) => {
              if (e.key === "Escape") onClose();
              if (e.key === "ArrowDown") (e.preventDefault(), setSel((i) => Math.min(hits.length - 1, i + 1)));
              if (e.key === "ArrowUp") (e.preventDefault(), setSel((i) => Math.max(0, i - 1)));
              if (e.key === "Enter") run(hits[sel]);
            }}
          />
        </div>
        <div className="max-h-[420px] overflow-y-auto p-1.5">
          {hits.map((h, i) => (
            <button
              key={h.cat + h.title}
              onMouseEnter={() => setSel(i)}
              onClick={() => run(h)}
              className={cn("relative flex w-full items-center gap-3 rounded-lg px-3 py-2 text-left", i !== sel && "hover:bg-elevated")}
            >
              {i === sel && <motion.span layoutId="palette-sel" transition={highlight} className="absolute inset-0 rounded-lg bg-blue-600/15" />}
              <span className="chip relative z-10 w-16 justify-center border-line-strong bg-elevated text-subtle">{h.cat}</span>
              <span className="relative z-10 min-w-0 flex-1">
                <span className="block truncate text-xs font-medium">{h.title}</span>
                <span className="block truncate text-[11px] text-subtle">{h.sub}</span>
              </span>
              <ArrowRight className="relative z-10 h-3.5 w-3.5 text-subtle" />
            </button>
          ))}
          {hits.length === 0 && <div className="p-6 text-center text-xs text-subtle">{app.t("common.none")}</div>}
        </div>
      </div>
    </div>
  );
}
