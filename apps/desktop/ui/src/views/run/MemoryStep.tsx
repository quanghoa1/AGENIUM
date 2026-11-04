import { Brain, Undo2 } from "lucide-react";
import { ConfirmModal } from "../../components/motion";
import { Mono, ago, cn } from "../../components/ui";
import type { Key } from "../../lib/i18n";
import { useState } from "react";
import { api } from "../../lib/api";
import { useApp } from "../../lib/store";
import type { LogEntry } from "../../lib/types";

const KIND: Record<string, Key> = { fact: "runmem.kind.fact", preference: "runmem.kind.preference", lesson: "runmem.kind.lesson" };
const REFLECT: Record<string, Key> = { update: "runmem.reflect.update", new: "runmem.reflect.new", none: "runmem.reflect.none" };
const OP: Record<string, [Key, string]> = {
  add: ["runmem.op.add", "border-emerald-500/30 bg-emerald-500/10 text-emerald-600 dark:text-emerald-300"],
  update: ["runmem.op.update", "border-amber-500/30 bg-amber-500/10 text-amber-600 dark:text-amber-300"],
  delete: ["runmem.op.delete", "border-rose-500/30 bg-rose-500/10 text-rose-600 dark:text-rose-300"],
};

function KindChip({ kind }: { kind: string }) {
  const app = useApp();
  return <span className="chip shrink-0 border-violet-500/30 bg-violet-500/10 text-violet-600 dark:text-violet-300">{KIND[kind] ? app.t(KIND[kind]) : kind}</span>;
}

/** What a memory run learned from: a ticket (its key opens it) or the chat with the Lead, after which run. */
export function memorySource(data: any): { ticket?: string; chat: boolean } {
  const s = String(data?.source ?? "");
  return s.startsWith("ticket:") ? { ticket: s.slice(7), chat: false } : { chat: s.startsWith("chat:") };
}

/** A memory line of a run's log, in words: the memories the agent was reminded of when it started (`recalled`), what
 *  a memory extraction learned from an episode and changed in the memory (its report), shared lessons that left the
 *  pool (`pool_dropped`), a settling pass (`settled`), the proven lessons reflected on (`reflected`), or why it did
 *  not run. The memory run's first line (`kind: memory`) names what it learned from. */
export function MemoryStep({ e, run, undone, reload }: { e: LogEntry; run?: string; undone?: boolean; reload?: () => void }) {
  const app = useApp();
  const [busy, setBusy] = useState(false);
  // A proven lesson the undo would lose: shown, and the undo goes on only once confirmed.
  const [warn, setWarn] = useState<string[] | null>(null);
  // Undo what this learning wrote (its facts out, those it replaced or retired back); a proven lesson asks first.
  const undo = async (force: boolean) => {
    if (!run) return;
    setBusy(true);
    try {
      const r = await api.revertMemoryRun(run, force);
      if (!r.done) {
        setWarn(r.warnings);
        return;
      }
      app.notify(app.t("runmem.undone").replace("{n}", String(r.undone ?? 0)));
      reload?.();
    } catch (err) {
      app.notify(String(err));
    } finally {
      setBusy(false);
    }
  };
  const d = e.data ?? {};
  const fill = (k: Key, xs: Record<string, unknown>) => Object.entries(xs).reduce((s, [a, b]) => s.replace(`{${a}}`, String(b ?? 0)), app.t(k));
  let title: string;
  let body: React.ReactNode;
  if (e.kind === "run") {
    const src = memorySource(d);
    title = app.t("runmem.from");
    body = (
      <div className="mt-1 flex flex-wrap items-center gap-2 text-xs">
        {src.ticket ? (
          <button className="font-mono text-cyan-600 hover:underline dark:text-cyan-400" onClick={() => app.openTicket(src.ticket!, d.project)}>
            {src.ticket}
          </button>
        ) : (
          <span>{src.chat ? app.t("runmem.chat") : String(d.source ?? "–")}</span>
        )}
        {d.from_run && (
          <>
            <span className="text-subtle">· {app.t("runmem.afterRun")}</span>
            <button className="font-mono text-cyan-600 hover:underline dark:text-cyan-400" onClick={() => app.go("run", d.from_run)}>
              {String(d.from_run).slice(-10)}
            </button>
          </>
        )}
      </div>
    );
  } else if (Array.isArray(d.recalled)) {
    title = `${app.t("runmem.recalled")} · ${d.recalled.length}`;
    body =
      d.recalled.length === 0 ? (
        <div className="mt-1 text-xs text-subtle">{app.t("runmem.none")}</div>
      ) : (
        <ul className="mt-1.5 space-y-1">
          {d.recalled.map((r: any) => (
            // A shared lesson's id is the pool's: it may be the same number as one of the agent's own.
            <li key={`${r.shared ? "pool" : "own"}-${r.id}`} className="flex items-start gap-2 text-xs">
              <KindChip kind={r.kind} />
              {r.shared && <span className="chip shrink-0 border-line-strong bg-elevated text-muted">{app.t("runmem.shared")}</span>}
              <span className="min-w-0">{r.text}</span>
            </li>
          ))}
        </ul>
      );
  } else if (e.kind === "memory-revert") {
    title = app.t("runmem.reverted").replace("{n}", String(d.undone ?? 0));
    body = d.skills_flagged?.length > 0 ? <div className="mt-1 text-xs text-muted">{app.t("runmem.skillsFlagged")}: {d.skills_flagged.join(", ")}</div> : null;
  } else if (typeof d.extracted === "number") {
    const changes: any[] = d.changes ?? [];
    // Only a learning whose changes name their facts can be undone (those learned before could not).
    const undoable = !!run && changes.length > 0 && changes.every((c) => c.id != null || c.old_id != null);
    title = app.t("runmem.learned");
    body = (
      <div className="mt-1 space-y-1.5 text-xs">
        {undoable &&
          (undone ? (
            <div className="text-subtle">{app.t("runmem.isUndone")}</div>
          ) : (
            <>
              <button className="btn btn-sm" disabled={busy} onClick={() => undo(false)}>
                <Undo2 className="h-3.5 w-3.5" /> {app.t("runmem.undo")}
              </button>
              <ConfirmModal
                open={!!warn}
                title={app.t("runmem.undo")}
                note={
                  <>
                    {app.t("runmem.undoWarn")}
                    <ul className="mt-2 list-disc pl-5">
                      {warn?.map((w, i) => (
                        <li key={i}>{w}</li>
                      ))}
                    </ul>
                  </>
                }
                confirm={app.t("runmem.undo")}
                icon={<Undo2 className="h-3.5 w-3.5" />}
                onClose={() => setWarn(null)}
                onConfirm={() => {
                  setWarn(null);
                  void undo(true);
                }}
              />
            </>
          ))}
        <div className="text-muted">{fill("runmem.counts", { x: d.extracted, a: d.added, u: d.updated, d: d.deleted, k: d.noop })}</div>
        {(d.helped > 0 || d.misleading?.length > 0 || d.validated > 0) && (
          <div className="text-muted">{fill("runmem.reuse", { h: d.helped, m: d.misleading?.length ?? 0, v: d.validated })}</div>
        )}
        {d.misleading_shared?.length > 0 && <div className="text-muted">{fill("runmem.sharedMisled", { n: d.misleading_shared.length })}</div>}
        {d.failed && <div className="text-rose-600 dark:text-rose-400">{String(d.failed)}</div>}
        {changes.length === 0 ? (
          <div className="text-subtle">{app.t("runmem.nothing")}</div>
        ) : (
          <ul className="space-y-1">
            {changes.map((c, i) => (
              <li key={i} className="flex items-start gap-2">
                <span className={cn("chip shrink-0", OP[c.op]?.[1])}>{OP[c.op] ? app.t(OP[c.op][0]) : c.op}</span>
                <KindChip kind={c.kind} />
                <span className="min-w-0">
                  <span className={cn(c.op === "delete" && "text-subtle line-through")}>{c.text}</span>
                  {c.was && (
                    <span className="block text-subtle">
                      {app.t("runmem.was")}: <span className="line-through">{c.was}</span>
                    </span>
                  )}
                </span>
              </li>
            ))}
          </ul>
        )}
        {d.rejected?.length > 0 && (
          <details className="text-subtle">
            <summary className="cursor-pointer">
              {app.t("runmem.refused")} · {d.rejected.length}
            </summary>
            <ul className="mt-1 list-disc pl-5">
              {d.rejected.map((r: string, i: number) => (
                <li key={i}>{r}</li>
              ))}
            </ul>
          </details>
        )}
      </div>
    );
  } else if (Array.isArray(d.pool_dropped)) {
    // Shared lessons that misled work in two contexts (audit M3): out of the pool, never shown to an agent again.
    title = `${app.t("runmem.poolDropped")} · ${d.pool_dropped.length}`;
    body = (
      <div className="mt-1 space-y-1.5 text-xs">
        <div className="text-muted">{app.t("runmem.poolDroppedNote")}</div>
        <ul className="space-y-1">
          {d.pool_dropped.map((t: string, i: number) => (
            <li key={i} className="text-subtle line-through">
              {t}
            </li>
          ))}
        </ul>
      </div>
    );
  } else if (d.settled) {
    // ADR-145: a settling pass over the agent's memory.
    const s = d.settled;
    title = app.t("runmem.settled");
    body = <div className="mt-1 text-xs text-muted">{fill("runmem.settledCounts", { f: s.facts, o: s.folded, c: s.conflicts })}</div>;
  } else if (Array.isArray(d.reflected)) {
    // ADR-146: lessons proven in two contexts, each reflected on once.
    title = `${app.t("runmem.reflected")} · ${d.reflected.length}`;
    body = (
      <ul className="mt-1.5 space-y-1 text-xs">
        {d.reflected.map((r: any, i: number) => (
          <li key={i} className="flex items-start gap-2">
            <span className="chip shrink-0 border-violet-500/30 bg-violet-500/10 text-violet-600 dark:text-violet-300">
              {r.error ? app.t("runmem.reflect.error") : app.t(REFLECT[r.action] ?? "runmem.reflect.none")}
              {r.skill ? ` · ${r.skill}` : ""}
            </span>
            <span className={cn("min-w-0", r.error || r.result?.refused ? "text-rose-600 dark:text-rose-400" : "text-muted")}>
              {r.error
                ? String(r.error)
                : r.result?.refused
                  ? `${app.t("runmem.reflect.refused")}: ${r.result.refused}`
                  : r.result?.accepted
                    ? app.t("runmem.reflect.accepted")
                    : (r.reason ?? "")}
            </span>
          </li>
        ))}
      </ul>
    );
  } else if (d.skipped) {
    title = app.t("runmem.skipped");
    body = <div className="mt-1 text-xs text-subtle">{String(d.skipped)}</div>;
  } else {
    title = app.t("runmem.failed");
    body = <div className="mt-1 text-xs text-rose-600 dark:text-rose-400">{String(d.error ?? JSON.stringify(d))}</div>;
  }
  return (
    <div className="flex gap-3 rounded-lg border border-violet-500/30 bg-violet-500/5 p-2.5">
      <Mono className="w-6 text-subtle">{e.n}</Mono>
      <Brain className="h-4 w-4 shrink-0 text-violet-500" />
      <div className="min-w-0 flex-1">
        <div className="label">{title}</div>
        {body}
      </div>
      <Mono className="text-subtle">{ago(e.at)}</Mono>
    </div>
  );
}
