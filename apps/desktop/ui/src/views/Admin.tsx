import { CheckCircle2, Copy, XCircle } from "lucide-react";
import { useEffect, useMemo, useState } from "react";
import { Empty, Mono, PageHeader, Section, cn, tokens } from "../components/ui";
import { api } from "../lib/api";
import { useApp } from "../lib/store";
import type { AuditEntry, Capabilities, RunRow, TeamTemplate } from "../lib/types";

export function ProvidersView() {
  const app = useApp();
  const [c, setC] = useState<Capabilities | null>(null);
  useEffect(() => {
    api.capabilities().then(setC);
  }, []);
  if (!c) return <div className="text-xs text-subtle">{app.t("common.loading")}</div>;
  return (
    <div className="space-y-4">
      <PageHeader title={app.t("nav.providers")} sub={app.t("providers.sub")} />
      <Section title={app.t("providers.api")}>
        <table className="w-full text-xs">
          <thead>
            <tr className="label border-b border-line text-left">
              <th className="py-2">id</th>
              <th>kind</th>
              <th>base URL</th>
              <th>key</th>
              <th>concurrency</th>
            </tr>
          </thead>
          <tbody className="divide-y divide-line">
            {c.providers.map((p) => (
              <tr key={p.id}>
                <td className="py-2 font-medium">{p.id}</td>
                <td className="text-muted">{p.kind}</td>
                <td className="font-mono text-[11px] text-subtle">{p.base_url ?? "default"}</td>
                <td>{p.has_key ? <CheckCircle2 className="h-4 w-4 text-emerald-500" /> : <span className="text-subtle">none</span>}</td>
                <td className="font-mono">{p.max_concurrency}</td>
              </tr>
            ))}
          </tbody>
        </table>
        {c.providers.length === 0 && <Empty>{app.t("common.none")}</Empty>}
      </Section>
      <Section title={app.t("providers.cli")}>
        <table className="w-full text-xs">
          <thead>
            <tr className="label border-b border-line text-left">
              <th className="py-2">CLI</th>
              <th>version</th>
              <th>login</th>
              <th>usable</th>
              <th>note</th>
            </tr>
          </thead>
          <tbody className="divide-y divide-line">
            {c.clis.map((x) => (
              <tr key={x.kind}>
                <td className="py-2 font-medium">{x.kind.toLowerCase()}</td>
                <td className="font-mono">{x.version ?? "–"}</td>
                <td className="text-muted">{typeof x.login === "string" ? x.login : JSON.stringify(x.login)}</td>
                <td>{x.compatible ? <CheckCircle2 className="h-4 w-4 text-emerald-500" /> : <XCircle className="h-4 w-4 text-subtle" />}</td>
                <td className="text-[11px] text-subtle">{x.note ?? (x.path ? "" : "not installed")}</td>
              </tr>
            ))}
          </tbody>
        </table>
        <div className="mt-3 text-[11px] text-subtle">
          {app.t("providers.fallback")}: {c.cli_fallback.provider ? <Mono>{c.cli_fallback.provider}/{c.cli_fallback.model}</Mono> : app.t("providers.notSet")}
        </div>
      </Section>
    </div>
  );
}

export function TemplatesView() {
  const app = useApp();
  const [ts, setTs] = useState<TeamTemplate[]>([]);
  useEffect(() => {
    api.templates().then(setTs);
  }, []);
  return (
    <div className="space-y-4">
      <PageHeader title={app.t("nav.templates")} sub={app.t("templates.sub")} />
      {ts.map((t) => (
        <Section key={t.id} title={`${t.name} · ${t.id}@${t.version} · report ${t.report_template}`}>
          <div className="mb-3 text-xs text-muted">{t.description}</div>
          <div className="grid gap-2 md:grid-cols-2 xl:grid-cols-3">
            {t.roles.map((r) => (
              <div key={r.id} className="rounded-lg border border-line bg-elevated/40 p-3">
                <div className="flex items-center gap-2">
                  <span className="text-xs font-semibold">{r.id}</span>
                  <span className={cn("chip", r.kind === "lead" ? "border-purple-500/30 text-purple-600 dark:text-purple-300" : r.planner ? "border-cyan-500/30 text-cyan-600 dark:text-cyan-300" : "border-line-strong text-muted")}>
                    {r.planner ? "planner" : r.kind}
                    {r.count > 1 ? ` ×${r.count}` : ""}
                  </span>
                </div>
                <div className="mt-1.5 line-clamp-4 text-[11px] text-subtle">{r.prompt}</div>
                {r.allowed_paths.length > 0 && <div className="mt-2 font-mono text-[10px] text-muted">scope: {r.allowed_paths.join(", ")}</div>}
              </div>
            ))}
          </div>
        </Section>
      ))}
    </div>
  );
}

export function AuditView() {
  const app = useApp();
  const [rows, setRows] = useState<AuditEntry[]>([]);
  const [onlyRefused, setOnlyRefused] = useState(false);
  useEffect(() => {
    if (app.project) api.audit(app.project).then((r) => setRows(r as AuditEntry[]));
  }, [app.project, app.tick]);
  const shown = rows.filter((r) => !onlyRefused || !r.accepted);
  return (
    <div className="space-y-4">
      <PageHeader title={app.t("nav.audit")} sub={app.t("audit.sub")}>
        <label className="flex items-center gap-1.5 text-xs">
          <input type="checkbox" checked={onlyRefused} onChange={(e) => setOnlyRefused(e.target.checked)} /> {app.t("audit.refusedOnly")}
        </label>
        <button
          className="btn"
          onClick={() =>
            app.project &&
            api.audit(app.project, undefined, "md").then((md) => navigator.clipboard.writeText(String(md)).then(() => app.notify(app.t("audit.copied"))))
          }
        >
          <Copy className="h-3.5 w-3.5" /> Markdown
        </button>
      </PageHeader>
      <div className="card overflow-x-auto">
        <table className="w-full text-xs">
          <tbody className="divide-y divide-line">
            {[...shown].reverse().map((e, i) => (
              <tr key={i}>
                <td className="whitespace-nowrap px-3 py-2 font-mono text-[11px] text-subtle">{e.at.replace("T", " ").slice(0, 19)}</td>
                <td className="px-3 font-medium">{e.actor}</td>
                <td className="px-3">
                  {e.ticket ? (
                    <button className="font-mono text-cyan-600 hover:underline dark:text-cyan-400" onClick={() => app.go("ticket", e.ticket!)}>
                      {e.ticket}
                    </button>
                  ) : (
                    "–"
                  )}
                </td>
                <td className="px-3">{e.action}</td>
                <td className="px-3">{e.accepted ? <span className="text-emerald-600 dark:text-emerald-400">{app.t("ticket.applied")}</span> : <span className="text-rose-600 dark:text-rose-400">{app.t("ticket.refused")}</span>}</td>
                <td className="max-w-[420px] truncate px-3 font-mono text-[10px] text-subtle" title={JSON.stringify(e.detail)}>
                  {JSON.stringify(e.detail)}
                </td>
              </tr>
            ))}
          </tbody>
        </table>
        {shown.length === 0 && <div className="p-4"><Empty>{app.t("common.none")}</Empty></div>}
      </div>
    </div>
  );
}

export function UsageView() {
  const app = useApp();
  const [rows, setRows] = useState<RunRow[]>([]);
  useEffect(() => {
    api.runs(2000).then(setRows);
  }, [app.tick]);
  const agg = useMemo(() => {
    const m = new Map<string, { runs: number; ok: number; tin: number; tout: number; cost: number; model: string }>();
    for (const r of rows) {
      const k = r.agent_name ?? r.agent;
      const x = m.get(k) ?? { runs: 0, ok: 0, tin: 0, tout: 0, cost: 0, model: r.model };
      x.runs++;
      if (r.outcome === "Completed") x.ok++;
      x.tin += r.usage.tokens_in;
      x.tout += r.usage.tokens_out;
      x.cost += r.usage.estimated_cost_usd;
      m.set(k, x);
    }
    return [...m.entries()].sort((a, b) => b[1].tin - a[1].tin);
  }, [rows]);
  const total = agg.reduce((s, [, x]) => s + x.cost, 0);
  return (
    <div className="space-y-4">
      <PageHeader title={app.t("nav.usage")} sub={`${app.t("usage.sub")} ${rows.length} runs · $${total.toFixed(3)}`} />
      <div className="card overflow-x-auto">
        <table className="w-full text-xs">
          <thead>
            <tr className="label border-b border-line text-left">
              <th className="px-4 py-2">agent</th>
              <th className="px-3">model</th>
              <th className="px-3">runs</th>
              <th className="px-3">{app.t("usage.completed")}</th>
              <th className="px-3">tokens in</th>
              <th className="px-3">tokens out</th>
              <th className="px-3">est. cost</th>
            </tr>
          </thead>
          <tbody className="divide-y divide-line">
            {agg.map(([k, x]) => (
              <tr key={k}>
                <td className="px-4 py-2 font-medium">{k}</td>
                <td className="px-3 text-muted">{x.model}</td>
                <td className="px-3 font-mono">{x.runs}</td>
                <td className="px-3 font-mono">{Math.round((100 * x.ok) / x.runs)}%</td>
                <td className="px-3 font-mono">{tokens(x.tin)}</td>
                <td className="px-3 font-mono">{tokens(x.tout)}</td>
                <td className="px-3 font-mono">${x.cost.toFixed(3)}</td>
              </tr>
            ))}
          </tbody>
        </table>
        {agg.length === 0 && <div className="p-4"><Empty>{app.t("common.none")}</Empty></div>}
      </div>
    </div>
  );
}

export function SettingsView() {
  const app = useApp();
  return (
    <div className="space-y-4">
      <PageHeader title={app.t("nav.settings")} />
      <div className="grid gap-4 md:grid-cols-2">
        <Section title={app.t("settings.theme")}>
          <div className="flex gap-2">
            {(["light", "dark", "system"] as const).map((m) => (
              <button key={m} className={cn("btn", app.themeMode === m && "border-accent text-accent")} onClick={() => app.setThemeMode(m)}>
                {m}
              </button>
            ))}
          </div>
        </Section>
        <Section title={app.t("settings.language")}>
          <div className="flex gap-2">
            {(["vi", "en"] as const).map((l) => (
              <button key={l} className={cn("btn", app.lang === l && "border-accent text-accent")} onClick={() => app.setLang(l)}>
                {l === "vi" ? "Tiếng Việt" : "English"}
              </button>
            ))}
          </div>
          <div className="mt-2 text-[11px] text-subtle">{app.t("settings.languageNote")} ({app.status?.user_language ?? "–"})</div>
        </Section>
        <Section title={app.t("settings.root")}>
          <Mono>{app.status?.root}</Mono>
          <div className="mt-2 text-[11px] text-subtle">{app.t("settings.rootNote")}</div>
        </Section>
        <Section title={app.t("top.autopilot")}>
          <div className="text-xs text-muted">{app.t("settings.autopilotNote")}</div>
          <button className={cn("btn mt-2", app.autopilot && "border-emerald-500/40 text-emerald-600")} onClick={() => app.setAutopilot(!app.autopilot)}>
            {app.autopilot ? "ON" : "OFF"}
          </button>
        </Section>
      </div>
    </div>
  );
}

export function SetupView() {
  const app = useApp();
  const [ui, setUi] = useState(app.lang);
  const [user, setUser] = useState("vi");
  return (
    <div className="flex h-full items-center justify-center bg-background p-6">
      <div className="card w-[460px] p-6">
        <div className="mb-4 flex items-center gap-3">
          <div className="flex h-10 w-10 items-center justify-center rounded-lg bg-gradient-to-br from-blue-600 to-cyan-500 font-mono font-bold text-white">AG</div>
          <div>
            <div className="text-base font-semibold">{app.t("setup.title")}</div>
            <Mono className="text-subtle">{app.status?.root}</Mono>
          </div>
        </div>
        <label className="block text-xs">
          <span className="label">{app.t("setup.ui")}</span>
          <select className="input mt-1" value={ui} onChange={(e) => (setUi(e.target.value as "vi" | "en"), app.setLang(e.target.value as "vi" | "en"))}>
            <option value="vi">Tiếng Việt</option>
            <option value="en">English</option>
          </select>
        </label>
        <label className="mt-3 block text-xs">
          <span className="label">{app.t("setup.user")}</span>
          <input className="input mt-1" value={user} onChange={(e) => setUser(e.target.value)} />
        </label>
        <button className="btn-primary mt-4 w-full justify-center" onClick={() => api.setup(ui, user).then(app.reloadStatus).catch((e) => app.notify(String(e)))}>
          {app.t("setup.go")}
        </button>
      </div>
    </div>
  );
}
