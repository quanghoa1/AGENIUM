import { Scale } from "lucide-react";
import { Mono, Section, ago } from "../components/ui";
import { api } from "../lib/api";
import { useApp } from "../lib/store";
import type { Fact, MemoryConflict } from "../lib/types";

/** Pairs of an agent's memories that may contradict each other (ADR-145), found when its memory is settled: the user
 *  says which one holds (the other is folded into it, its history kept) or that both do. Nothing is settled without
 *  this decision. */
export function MemoryConflicts({ agent, conflicts, onDone }: { agent: string; conflicts: MemoryConflict[]; onDone: () => void }) {
  const app = useApp();
  if (conflicts.length === 0) return null;
  const decide = (c: MemoryConflict, choice: "a" | "b" | "both") =>
    api
      .resolveMemoryConflict(agent, c.a.id, c.b.id, choice)
      .then(() => (app.notify(app.t("common.done")), onDone()))
      .catch((e) => app.notify(String(e)));
  const side = (c: MemoryConflict, f: Fact, choice: "a" | "b") => (
    <div className="flex min-w-0 flex-1 flex-col gap-1.5 rounded-lg border border-line bg-background/60 p-2.5">
      <div className="text-xs">{f.text}</div>
      <div className="text-[11px] text-subtle">
        <Mono>#{f.id}</Mono> · {f.source} · {ago(f.created_at)}
      </div>
      <button className="btn mt-auto self-start" onClick={() => decide(c, choice)}>
        {app.t("memory.conflict.keep")}
      </button>
    </div>
  );
  return (
    <Section title={`${app.t("memory.conflicts")} (${conflicts.length})`}>
      <div className="text-[11px] text-subtle">{app.t("memory.conflicts.note")}</div>
      <div className="mt-2 space-y-2">
        {conflicts.map((c) => (
          <div key={`${c.a.id}-${c.b.id}`} className="rounded-lg border border-amber-500/40 bg-amber-500/5 p-2.5" data-conflict={`${c.a.id}-${c.b.id}`}>
            <div className="mb-2 flex items-center gap-1.5 text-[11px] text-amber-600 dark:text-amber-300">
              <Scale className="h-3.5 w-3.5" />
              {app.t(c.by === "judge" ? "memory.conflict.byJudge" : "memory.conflict.byFigures")} · {Math.round(c.similarity * 100)}%
            </div>
            <div className="flex flex-col gap-2 md:flex-row">
              {side(c, c.a, "a")}
              {side(c, c.b, "b")}
            </div>
            <button className="btn mt-2" onClick={() => decide(c, "both")}>
              {app.t("memory.conflict.both")}
            </button>
          </div>
        ))}
      </div>
    </Section>
  );
}
