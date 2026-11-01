import { ArrowLeft, CheckCircle2, SquareTerminal, XCircle } from "lucide-react";
import { Suspense, lazy, useEffect, useState } from "react";
import { ContextGauge } from "../components/ContextGauge";
import { Markdown } from "../components/Markdown";
import { SearchBox, found } from "../components/SearchBox";
import { CacheHit, Empty, Mono, OutcomeBadge, PageHeader, Section, ago, cn, tokens } from "../components/ui";
import { api } from "../lib/api";
import { useApp, useLive } from "../lib/store";
import type { Key } from "../lib/i18n";
import type { LogEntry, RunDetail, RunRow } from "../lib/types";
import { plainArgs, startsSubagent, subagentOf } from "../lib/subagents";
import { MemoryStep, memorySource } from "./run/MemoryStep";

export function RunsView() {
  const app = useApp();
  const [rows, setRows] = useState<RunRow[]>([]);
  useEffect(() => {
    api.runs(150).then(setRows);
  }, [app.tick]);
  const [query, setQuery] = useState("");
  const shown = rows.filter((r) => found(query, r.agent_name, r.ticket_key, r.project_name, r.model, r.outcome ?? "running", r.source));
  return (
    <div>
      <PageHeader title={app.t("nav.runs")} sub={`${rows.filter((r) => !r.ended_at).length} ${app.t("runs.live")}`}>
        <SearchBox value={query} onChange={setQuery} placeholder={app.t("search.runs")} />
      </PageHeader>
      <div className="card overflow-x-auto">
        <table className="w-full text-xs">
          <thead>
            <tr className="label border-b border-line text-left">
              <th className="px-4 py-2">{app.t("runs.outcome")}</th>
              <th className="px-3">{app.t("runs.agent")}</th>
              <th className="px-3">{app.t("runs.ticket")}</th>
              <th className="px-3">{app.t("common.model")}</th>
              <th className="px-3">{app.t("runs.steps")}</th>
              <th className="px-3">{app.t("ticket.tokens")}</th>
              <th className="px-3">{app.t("runs.cost")}</th>
              <th className="px-3 text-right">{app.t("runs.started")}</th>
            </tr>
          </thead>
          <tbody className="divide-y divide-line">
            {shown.map((r) => (
              <tr key={r.id} className="cursor-pointer hover:bg-elevated/60" onClick={() => app.go("run", r.id)}>
                <td className="px-4 py-2">
                  <OutcomeBadge outcome={r.outcome} />
                </td>
                <td className="px-3 font-medium">{r.agent_name}</td>
                <td className="px-3">
                  {r.ticket_key ? (
                    <>
                      <Mono className="text-cyan-600 dark:text-cyan-400">{r.ticket_key}</Mono> <span className="text-subtle">{r.project_name}</span>
                    </>
                  ) : r.outcome === "Memory" ? (
                    // A memory run learned from a ticket or the chat; one recorded before its source was logged says so only.
                    <span className="text-violet-600 dark:text-violet-300">
                      {r.source ? app.t("runmem.list").replace("{x}", memorySource({ source: r.source }).ticket ?? app.t("runmem.chat")) : app.t("runmem.listOld")}
                    </span>
                  ) : (
                    <span className="text-subtle">{app.t("runs.noTicket")}</span>
                  )}
                </td>
                <td className="px-3 text-muted">{r.model}</td>
                <td className="px-3 font-mono">{r.steps}</td>
                <td className="px-3 font-mono">{tokens(r.usage.tokens_in + r.usage.tokens_out)}</td>
                <td className="px-3 font-mono">${r.usage.estimated_cost_usd.toFixed(3)}</td>
                <td className="px-3 text-right font-mono text-subtle">{ago(r.started_at)}</td>
              </tr>
            ))}
          </tbody>
        </table>
        {shown.length === 0 && (
          <div className="p-4">
            <Empty>{app.t("common.none")}</Empty>
          </div>
        )}
      </div>
    </div>
  );
}

const SHELL = new Set(["shell", "cli:Bash", "cli:PowerShell", "cli:run_command", "cli:shell"]);

const TerminalView = lazy(() => import("../components/Terminal"));

/** The run's shell session as terminal text: each command (green prompt), its output, a red line when it failed. */
function session(log: LogEntry[]): string {
  const out: string[] = [];
  for (const e of log) {
    if (e.kind !== "tool" || !SHELL.has(e.data.tool)) continue;
    const cmd = e.data.args?.command ?? e.data.args?.CommandLine;
    if (!cmd) continue;
    out.push(`\x1b[32m$\x1b[0m \x1b[1m${cmd}\x1b[0m\n`);
    if (e.data.result) out.push(String(e.data.result).replace(/\n?$/, "\n"));
    if (!e.data.ok) out.push("\x1b[31m[failed]\x1b[0m\n");
    out.push("\n");
  }
  return out.join("");
}

/** What Agenium did to the run (ADR-155): compacted it, delivered a message, resumed a session…, said in words. */
function ControlStep({ e }: { e: LogEntry }) {
  const app = useApp();
  const what = String(e.data?.what ?? "");
  const key = `control.${what}` as Key;
  const label = app.t(key) === key ? what : app.t(key);
  const strong = what === "compact" || what === "fit";
  return (
    <div className={cn("flex gap-3 rounded-lg border p-2.5", strong ? "border-violet-500/40 bg-violet-500/5" : "border-line")}>
      <Mono className="w-6 text-subtle">{e.n}</Mono>
      <div className="min-w-0 flex-1">
        <div className={cn("label", strong && "text-violet-600 dark:text-violet-300")}>{label}</div>
        {e.data?.text && <div className="mt-1 whitespace-pre-wrap text-xs text-muted">{String(e.data.text)}</div>}
      </div>
      <Mono className="text-subtle">{ago(e.at)}</Mono>
    </div>
  );
}

function Step({ e }: { e: LogEntry }) {
  if (e.kind === "memory" || (e.kind === "run" && e.data?.kind === "memory")) return <MemoryStep e={e} />;
  if (e.kind === "control") return <ControlStep e={e} />;
  if (e.kind !== "tool") {
    return (
      <div className="flex gap-3 rounded-lg border border-line p-2.5">
        <Mono className="w-6 text-subtle">{e.n}</Mono>
        <div className="min-w-0 flex-1">
          <div className="label">{e.kind === "judge" ? `Jev · ${e.data?.use ?? ""}` : e.kind}</div>
          {e.kind === "judge" && typeof e.data?.text === "string" ? (
            <Markdown className="mt-1 text-xs" text={e.data.text} />
          ) : typeof e.data?.summary === "string" ? (
            <Markdown className="mt-1 text-xs" text={e.data.summary} />
          ) : (
            <div className="mt-1 font-mono text-[11px] text-subtle">{JSON.stringify(e.data).slice(0, 400)}</div>
          )}
        </div>
        <Mono className="text-subtle">{ago(e.at)}</Mono>
      </div>
    );
  }
  const shell = SHELL.has(e.data.tool);
  const cmd = e.data.args?.command ?? e.data.args?.CommandLine;
  // ADR-129: a step a CLI's subagent made is indented under it and named; the step that starts one says so.
  const sub = subagentOf(e.data.args);
  const starts = startsSubagent(e.data.args);
  const args = plainArgs(e.data.args);
  return (
    <div className={cn("flex gap-3 rounded-lg border border-line p-2.5", sub && "ml-8 border-l-2 border-l-violet-500/60")}>
      <Mono className="w-6 text-subtle">{e.n}</Mono>
      {e.data.ok ? <CheckCircle2 className="h-4 w-4 shrink-0 text-emerald-500" /> : <XCircle className="h-4 w-4 shrink-0 text-rose-500" />}
      <div className="min-w-0 flex-1">
        <div className="flex flex-wrap items-center gap-2 text-xs font-medium">
          {e.data.tool}
          {starts && <span className="chip border-violet-500/40 bg-violet-500/10 text-violet-600 dark:text-violet-300">▶ {starts}</span>}
          {sub && <span className="chip border-violet-500/30 bg-violet-500/5 text-violet-600 dark:text-violet-300">{sub}</span>}
        </div>
        {shell && cmd ? (
          <div className="mt-1 overflow-hidden rounded-md border border-zinc-800 bg-[#0b0e14] font-mono text-[11px] text-zinc-200">
            <div className="border-b border-zinc-800 px-2.5 py-1.5 text-emerald-400">$ {cmd}</div>
            {e.data.result && <pre className={cn("max-h-64 overflow-auto whitespace-pre-wrap px-2.5 py-1.5", !e.data.ok && "text-rose-300")}>{String(e.data.result)}</pre>}
          </div>
        ) : (
          <>
            <div className="truncate font-mono text-[11px] text-muted">{JSON.stringify(args)}</div>
            {e.data.result && <div className={cn("mt-1 line-clamp-4 whitespace-pre-wrap font-mono text-[11px]", e.data.ok ? "text-subtle" : "text-rose-600 dark:text-rose-400")}>{String(e.data.result)}</div>}
          </>
        )}
      </div>
      <Mono className="text-subtle">{ago(e.at)}</Mono>
    </div>
  );
}

export function RunView({ id }: { id: string }) {
  const app = useApp();
  const [d, setD] = useState<RunDetail | null>(null);
  const [raw, setRaw] = useState(false);
  const [term, setTerm] = useState(false);
  const live = useLive()[id];
  useEffect(() => {
    api.runDetail(id).then(setD).catch((e) => app.notify(String(e)));
  }, [id, app.tick]);
  if (!d) return <div className="text-xs text-subtle">{app.t("common.loading")}</div>;
  const r = d.run;
  return (
    <div className="space-y-4">
      <button className="btn" onClick={() => app.go("runs")}>
        <ArrowLeft className="h-3.5 w-3.5" /> {app.t("nav.runs")}
      </button>
      <div className="card grid grid-cols-2 gap-4 p-4 text-xs md:grid-cols-6">
        <div>
          <div className="label">run</div>
          <Mono>{r.id.slice(-10)}</Mono>
        </div>
        <div>
          <div className="label">{app.t("runs.agent")}</div>
          {r.agent_name}
        </div>
        <div>
          <div className="label">{app.t("runs.ticket")}</div>
          {d.ticket_key ? (
            <button className="font-mono text-cyan-600 hover:underline dark:text-cyan-400" onClick={() => app.openTicket(d.ticket_key!, d.ticket_project)}>
              {d.ticket_key}
            </button>
          ) : (
            "–"
          )}
        </div>
        <div>
          <div className="label">{app.t("common.model")}</div>
          {r.model}
        </div>
        <div>
          <div className="label">
            {app.t("runs.steps")} · {app.t("ticket.tokens")}
          </div>
          <Mono>
            {r.steps} · {tokens(r.usage.tokens_in)} in / {tokens(r.usage.tokens_out)} out
          </Mono>
          {r.cache_hit != null && (
            <div className="mt-1">
              <CacheHit value={r.cache_hit} />
            </div>
          )}
          <div className="mt-1">
            <ContextGauge usage={r.usage} />
          </div>
        </div>
        <div>
          <div className="label">{app.t("runs.outcome")}</div>
          <OutcomeBadge outcome={r.outcome} />
        </div>
      </div>
      {live && !r.ended_at && (
        <Section title={app.t("runs.liveOutput")}>
          {live.text ? <Markdown className="text-xs" text={live.text} /> : <div className="text-xs text-subtle">…</div>}
          {live.tools.length > 0 && (
            <div className="mt-2 space-y-1">
              {live.tools.slice(-8).map((t, i) => (
                <div key={i} className="font-mono text-[11px] text-muted">
                  ▸ {t.name} {JSON.stringify(t.args)?.slice(0, 140)}
                </div>
              ))}
            </div>
          )}
        </Section>
      )}
      <Section
        title={app.t("runs.steps")}
        right={
          <div className="flex gap-1.5">
            {d.log.some((e) => e.kind === "tool" && SHELL.has(e.data.tool)) && (
              <button className={cn("btn", term && "border-accent text-accent")} onClick={() => setTerm(!term)}>
                <SquareTerminal className="h-3.5 w-3.5" /> {app.t("runs.terminal")}
              </button>
            )}
            <button className="btn" onClick={() => setRaw(!raw)}>
              {raw ? app.t("runs.readable") : "JSON"}
            </button>
          </div>
        }
      >
        {term && (
          <div className="mb-3">
            <Suspense fallback={<div className="text-xs text-subtle">{app.t("common.loading")}</div>}>
              <TerminalView text={session(d.log)} />
            </Suspense>
          </div>
        )}
        <div className="space-y-2">
          {d.log.map((e) =>
            raw ? (
              <pre key={e.n} className="overflow-x-auto rounded-md bg-background p-2 font-mono text-[11px]">
                {JSON.stringify(e, null, 2)}
              </pre>
            ) : (
              <Step key={e.n} e={e} />
            ),
          )}
          {d.log.length === 0 && <Empty>{app.t("common.none")}</Empty>}
        </div>
      </Section>
    </div>
  );
}
