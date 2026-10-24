import { ArrowLeft, CheckCircle2, XCircle } from "lucide-react";
import { useEffect, useState } from "react";
import { Markdown } from "../components/Markdown";
import { Empty, Mono, OutcomeBadge, PageHeader, Section, ago, cn, tokens } from "../components/ui";
import { api } from "../lib/api";
import { useApp } from "../lib/store";
import type { EventRecord, LogEntry, RunDetail, RunRow } from "../lib/types";

export function RunsView() {
  const app = useApp();
  const [rows, setRows] = useState<RunRow[]>([]);
  useEffect(() => {
    api.runs(150).then(setRows);
  }, [app.tick]);
  return (
    <div>
      <PageHeader title={app.t("nav.runs")} sub={`${rows.filter((r) => !r.ended_at).length} ${app.t("runs.live")}`} />
      <div className="card overflow-x-auto">
        <table className="w-full text-xs">
          <thead>
            <tr className="label border-b border-line text-left">
              <th className="px-4 py-2">{app.t("runs.outcome")}</th>
              <th className="px-3">{app.t("runs.agent")}</th>
              <th className="px-3">{app.t("common.model")}</th>
              <th className="px-3">{app.t("runs.steps")}</th>
              <th className="px-3">{app.t("ticket.tokens")}</th>
              <th className="px-3">{app.t("runs.cost")}</th>
              <th className="px-3 text-right">{app.t("runs.started")}</th>
            </tr>
          </thead>
          <tbody className="divide-y divide-line">
            {rows.map((r) => (
              <tr key={r.id} className="cursor-pointer hover:bg-elevated/60" onClick={() => app.go("run", r.id)}>
                <td className="px-4 py-2">
                  <OutcomeBadge outcome={r.outcome} />
                </td>
                <td className="px-3 font-medium">{r.agent_name}</td>
                <td className="px-3 text-muted">{r.model}</td>
                <td className="px-3 font-mono">{r.steps}</td>
                <td className="px-3 font-mono">{tokens(r.usage.tokens_in + r.usage.tokens_out)}</td>
                <td className="px-3 font-mono">${r.usage.estimated_cost_usd.toFixed(3)}</td>
                <td className="px-3 text-right font-mono text-subtle">{ago(r.started_at)}</td>
              </tr>
            ))}
          </tbody>
        </table>
        {rows.length === 0 && (
          <div className="p-4">
            <Empty>{app.t("common.none")}</Empty>
          </div>
        )}
      </div>
    </div>
  );
}

const SHELL = new Set(["shell", "cli:Bash", "cli:PowerShell", "cli:run_command", "cli:shell"]);

function Step({ e }: { e: LogEntry }) {
  if (e.kind !== "tool") {
    return (
      <div className="flex gap-3 rounded-lg border border-line p-2.5">
        <Mono className="w-6 text-subtle">{e.n}</Mono>
        <div className="min-w-0 flex-1">
          <div className="label">{e.kind}</div>
          {typeof e.data?.summary === "string" ? <Markdown className="mt-1 text-xs" text={e.data.summary} /> : <div className="mt-1 font-mono text-[11px] text-subtle">{JSON.stringify(e.data).slice(0, 400)}</div>}
        </div>
        <Mono className="text-subtle">{ago(e.at)}</Mono>
      </div>
    );
  }
  const shell = SHELL.has(e.data.tool);
  const cmd = e.data.args?.command ?? e.data.args?.CommandLine;
  return (
    <div className="flex gap-3 rounded-lg border border-line p-2.5">
      <Mono className="w-6 text-subtle">{e.n}</Mono>
      {e.data.ok ? <CheckCircle2 className="h-4 w-4 shrink-0 text-emerald-500" /> : <XCircle className="h-4 w-4 shrink-0 text-rose-500" />}
      <div className="min-w-0 flex-1">
        <div className="text-xs font-medium">{e.data.tool}</div>
        {shell && cmd ? (
          <div className="mt-1 overflow-hidden rounded-md border border-zinc-800 bg-[#0b0e14] font-mono text-[11px] text-zinc-200">
            <div className="border-b border-zinc-800 px-2.5 py-1.5 text-emerald-400">$ {cmd}</div>
            {e.data.result && <pre className={cn("max-h-64 overflow-auto whitespace-pre-wrap px-2.5 py-1.5", !e.data.ok && "text-rose-300")}>{String(e.data.result)}</pre>}
          </div>
        ) : (
          <>
            <div className="truncate font-mono text-[11px] text-muted">{JSON.stringify(e.data.args)}</div>
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
  useEffect(() => {
    api.runDetail(id).then(setD).catch((e) => app.notify(String(e)));
  }, [id, app.tick]);
  if (!d) return <div className="text-xs text-subtle">{app.t("common.loading")}</div>;
  const live = app.live[id];
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
            <button className="font-mono text-cyan-600 hover:underline dark:text-cyan-400" onClick={() => app.go("ticket", d.ticket_key!)}>
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
          <button className="btn" onClick={() => setRaw(!raw)}>
            {raw ? app.t("runs.readable") : "JSON"}
          </button>
        }
      >
        <div className="space-y-2">
          {d.log.map((e) =>
            raw ? (
              <pre key={e.n} className="overflow-x-auto rounded-md bg-background p-2 font-mono text-[10px]">
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

export function ActivityView() {
  const app = useApp();
  const [rows, setRows] = useState<EventRecord[]>([]);
  useEffect(() => {
    api.events(undefined, 300).then((e) => setRows([...e].reverse()));
  }, [app.tick]);
  return (
    <div>
      <PageHeader title={app.t("nav.activity")} sub={app.t("activity.sub")} />
      <div className="card divide-y divide-line">
        {rows.map((e) => (
          <div key={e.seq} className="flex items-center gap-3 px-4 py-2 text-xs">
            <Mono className="w-12 text-subtle">#{e.seq}</Mono>
            <span className="w-44 font-medium">{e.kind}</span>
            <span className="w-28 truncate text-muted">{e.actor.startsWith("agent:") ? "agent" : e.actor}</span>
            <span className="min-w-0 flex-1 truncate font-mono text-[11px] text-subtle">
              {e.payload?.key ? `${e.payload.key} ` : ""}
              {JSON.stringify(e.payload)}
            </span>
            <Mono className="text-subtle">{ago(e.at)}</Mono>
          </div>
        ))}
        {rows.length === 0 && (
          <div className="p-4">
            <Empty>{app.t("common.none")}</Empty>
          </div>
        )}
      </div>
    </div>
  );
}
