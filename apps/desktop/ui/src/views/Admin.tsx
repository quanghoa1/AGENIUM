import { Copy, Loader2, RefreshCw, ShieldAlert, ShieldCheck } from "lucide-react";
import { useEffect, useMemo, useState } from "react";
import { Logo } from "../components/Logo";
import { ConfirmModal } from "../components/motion";
import { Empty, FreeTag, Mono, PageHeader, Section, ago, cn, tokens } from "../components/ui";
import { api } from "../lib/api";
import { isFree, priceOf, usePrices } from "../lib/prices";
import { useApp } from "../lib/store";
import type { Key } from "../lib/i18n";
import type { AuditEntry, CommandPolicy, JudgeActivity, JudgeConfig, JudgeForm, JudgeUses, RunRow } from "../lib/types";

/** An event or a refused call in words: `ProjectCreated` -> "Project created" (or its translation). */
export function actionLabel(action: string, t: (k: Key) => string): string {
  const key = `audit.ev.${action}` as Key;
  const said = t(key);
  if (said !== key) return said;
  const words = action.replace(/_/g, " ").replace(/([a-z])([A-Z])/g, "$1 $2").toLowerCase();
  return words.charAt(0).toUpperCase() + words.slice(1);
}

/** An event's detail as `field: value` pairs (ids shortened), not a line of JSON. */
function detailText(d: unknown): string {
  if (d === null || d === undefined) return "";
  if (typeof d !== "object") return String(d);
  return Object.entries(d as Record<string, unknown>)
    .filter(([, v]) => v !== null && v !== undefined && v !== "")
    .map(([k, v]) => {
      const s = typeof v === "object" ? JSON.stringify(v) : String(v);
      // ULIDs are 26 characters: the first 8 tell them apart on one screen.
      const short = /^[0-9A-HJKMNP-TV-Z]{26}$/.test(s) ? `${s.slice(0, 8)}…` : s.length > 80 ? `${s.slice(0, 79)}…` : s;
      return `${k.replace(/_/g, " ")}: ${short}`;
    })
    .join(" · ");
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
      <PageHeader
        title={app.t("nav.audit")}
        sub={`${app.t("audit.sub")} ${app.t("audit.scope").replace("{x}", app.projects.find((p) => p.id === app.project)?.name ?? "–")}`}
      >
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
          <thead>
            <tr className="label border-b border-line text-left">
              <th className="px-3 py-2">{app.t("ticket.auditAt")}</th>
              <th className="px-3">{app.t("ticket.auditActor")}</th>
              <th className="px-3">Ticket</th>
              <th className="px-3">{app.t("ticket.auditAction")}</th>
              <th className="px-3">{app.t("ticket.auditResult")}</th>
              <th className="px-3">{app.t("ticket.auditDetail")}</th>
            </tr>
          </thead>
          <tbody className="divide-y divide-line">
            {[...shown].reverse().map((e, i) => (
              <tr key={i}>
                <td className="whitespace-nowrap px-3 py-2 font-mono text-[11px] text-subtle">{e.at.replace("T", " ").slice(0, 19)}</td>
                <td className="px-3 font-medium">{e.actor === "user" ? app.t("audit.you") : e.actor}</td>
                <td className="px-3">
                  {e.ticket ? (
                    <button className="font-mono text-cyan-600 hover:underline dark:text-cyan-400" onClick={() => app.go("ticket", e.ticket!)}>
                      {e.ticket}
                    </button>
                  ) : (
                    "–"
                  )}
                </td>
                <td className="px-3" title={e.action}>
                  {actionLabel(e.action, app.t)}
                </td>
                <td className="px-3">{e.accepted ? <span className="text-emerald-600 dark:text-emerald-400">{app.t("ticket.applied")}</span> : <span className="text-rose-600 dark:text-rose-400">{app.t("ticket.refused")}</span>}</td>
                <td className="max-w-[520px] truncate px-3 text-[11px] text-subtle" title={JSON.stringify(e.detail, null, 2)}>
                  {detailText(e.detail)}
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
  const [refreshing, setRefreshing] = useState(false);
  const prices = usePrices(app.tick);
  useEffect(() => {
    api.runs(2000).then(setRows);
  }, [app.tick]);
  const agg = useMemo(() => {
    const m = new Map<string, { runs: number; ok: number; tin: number; tout: number; cost: number; model: string; provider: string }>();
    for (const r of rows) {
      const k = r.agent_name ?? r.agent;
      const x = m.get(k) ?? { runs: 0, ok: 0, tin: 0, tout: 0, cost: 0, model: r.model, provider: r.provider };
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
      <PageHeader title={app.t("nav.usage")} sub={`${app.t("usage.sub")} ${rows.length} ${app.t(rows.length === 1 ? "overview.run" : "overview.runs")} · $${total.toFixed(3)}`}>
        <span className="text-[11px] text-subtle">
          {prices?.fetched_at ? `${app.t("usage.pricesFrom")} ${prices.source?.replace(/^https?:\/\//, "").split("/")[0]} · ${ago(prices.fetched_at)}` : app.t("usage.noPrices")}
        </span>
        <button
          className="btn"
          disabled={refreshing}
          onClick={() => {
            setRefreshing(true);
            api
              .refreshPrices()
              .then((r) => (app.notify(r.priced === null ? app.t("usage.pricesOff") : `${app.t("usage.pricesUpdated")} ${r.priced}`), app.refresh()))
              .catch((e) => app.notify(String(e)))
              .finally(() => setRefreshing(false));
          }}
        >
          <RefreshCw className={cn("h-3.5 w-3.5", refreshing && "animate-spin")} /> {app.t("usage.refreshPrices")}
        </button>
      </PageHeader>
      <div className="card overflow-x-auto">
        <table className="w-full text-xs">
          <thead>
            <tr className="label border-b border-line text-left">
              <th className="px-4 py-2">{app.t("runs.agent")}</th>
              <th className="px-3">{app.t("common.model")}</th>
              <th className="px-3">{app.t("usage.runs")}</th>
              <th className="px-3">{app.t("usage.completed")}</th>
              <th className="px-3">{app.t("usage.tokensIn")}</th>
              <th className="px-3">{app.t("usage.tokensOut")}</th>
              <th className="px-3">{app.t("ticket.cost")}</th>
            </tr>
          </thead>
          <tbody className="divide-y divide-line">
            {agg.map(([k, x]) => (
              <tr key={k}>
                <td className="px-4 py-2 font-medium">{k}</td>
                <td className="px-3 text-muted">
                  {x.model} {isFree(priceOf(prices, x.provider, x.model)) && <FreeTag />}
                </td>
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
                {app.t(`settings.theme.${m}` as Key)}
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
          <div className="mt-2 text-[11px] text-subtle">{app.t("settings.languageNote")}</div>
        </Section>
        <CommandPolicySection />
        <JudgeSection />
        <Section title={app.t("settings.support")}>
          <div className="text-[11px] text-subtle">{app.t("settings.supportNote")}</div>
          <button
            className="btn mt-2"
            onClick={() =>
              api
                .supportBundle("")
                .then((dir) => app.notify(`${app.t("settings.supportDone")} ${dir}`))
                .catch((e) => app.notify(String(e)))
            }
          >
            {app.t("settings.supportMake")}
          </button>
        </Section>
      </div>
    </div>
  );
}

/** The optional TypeSafe Jev judge (ADR-75, ADR-81): endpoint, model, the key (kept in the vault) and what it may do;
 *  its state (off, on, turned off after a failure) and a test. */
function JudgeSection() {
  const app = useApp();
  const [c, setC] = useState<JudgeConfig | null>(null);
  const [form, setForm] = useState<JudgeForm | null>(null);
  const [key, setKey] = useState("");
  const [busy, setBusy] = useState(false);
  const [activity, setActivity] = useState<JudgeActivity[]>([]);
  useEffect(() => {
    api.judgeActivity().then((a) => setActivity(a ?? [])).catch(() => undefined);
  }, [app.tick, busy]);
  const load = () =>
    api
      .judgeConfig()
      .then((j) => {
        setC(j);
        setForm({
          base_url: j.base_url || "https://api.typesafe.ai",
          model: j.model || "jev-1.14.0",
          // A first setup starts with the two uses measured to take work off the agents (docs/jev-comparison.md).
          review_assist: j.review_assist,
          skill_hint: j.configured ? j.skill_hint : true,
          memory_gate: j.configured ? j.memory_gate : true,
          skill_quality: j.skill_quality,
          done_check: j.done_check,
        });
        setKey("");
      })
      .catch(() => undefined);
  useEffect(() => {
    load();
  }, []);
  if (!c || !form) return null;
  const run = (p: Promise<unknown>, done: string) => {
    setBusy(true);
    p.then(() => (app.notify(done), load(), app.refresh()))
      .catch((e) => (app.notify(String(e)), load()))
      .finally(() => setBusy(false));
  };
  const save = (test: boolean) =>
    run(
      api.setJudge({ ...form, api_key: key.trim() || null }).then(() => (test ? api.enableJudge() : undefined)),
      app.t(test ? "settings.judgeEnabled" : "settings.judgeSaved"),
    );
  const uses: [keyof JudgeUses, Key][] = [
    ["review_assist", "settings.judgeReview"],
    ["skill_hint", "settings.judgeSkill"],
    ["memory_gate", "settings.judgeMemory"],
    ["skill_quality", "settings.judgeQuality"],
    ["done_check", "settings.judgeDone"],
  ];
  return (
    <Section title={app.t("settings.judge")}>
      <div className="text-[11px] text-subtle">{app.t("settings.judgeNote")}</div>
      <div className="mt-2 text-xs">
        {!c.configured ? (
          <span className="text-muted">{app.t("settings.judgeOff")}</span>
        ) : c.disabled ? (
          <span className="text-rose-600 dark:text-rose-400">
            {app.t("settings.judgeDisabled")}
            {c.reason ? `: ${c.reason}` : ""}
          </span>
        ) : (
          <span className="text-emerald-600 dark:text-emerald-400">{app.t("settings.judgeOn")}</span>
        )}
      </div>
      <div className="mt-3 grid gap-2 md:grid-cols-3">
        <label className="block text-xs">
          <span className="label">{app.t("settings.judgeEndpoint")}</span>
          <input className="input mt-1" value={form.base_url} onChange={(e) => setForm({ ...form, base_url: e.target.value })} />
        </label>
        <label className="block text-xs">
          <span className="label">{app.t("settings.judgeModel")}</span>
          <input className="input mt-1" value={form.model} onChange={(e) => setForm({ ...form, model: e.target.value })} />
        </label>
        <label className="block text-xs">
          <span className="label">{app.t("settings.judgeKey")}</span>
          <input
            className="input mt-1"
            type="password"
            autoComplete="off"
            value={key}
            placeholder={c.has_key ? app.t("settings.judgeKeySaved") : ""}
            onChange={(e) => setKey(e.target.value)}
          />
        </label>
      </div>
      <div className="mt-3 label">{app.t("settings.judgeUses")}</div>
      <div className="mt-1 space-y-1">
        {uses.map(([k, label]) => (
          <label key={k} className="flex items-center gap-2 text-xs">
            <input type="checkbox" checked={form[k]} onChange={(e) => setForm({ ...form, [k]: e.target.checked })} />
            <span title={k}>{app.t(label)}</span>
          </label>
        ))}
      </div>
      <div className="mt-3 flex flex-wrap gap-2">
        <button className="btn-primary" disabled={busy || !form.base_url.trim() || !form.model.trim() || (!c.has_key && !key.trim())} onClick={() => save(true)}>
          {app.t("settings.judgeSaveTest")}
        </button>
        <button className="btn" disabled={busy} onClick={() => save(false)}>
          {app.t("settings.judgeSave")}
        </button>
        {c.configured && (
          <button className="btn" disabled={busy} onClick={() => run(api.setJudge({ ...form, base_url: "" }), app.t("settings.judgeSaved"))}>
            {app.t("settings.judgeTurnOff")}
          </button>
        )}
      </div>
      <div className="mt-4 label">{app.t("settings.judgeActivity")}</div>
      {activity.length === 0 ? (
        <div className="mt-1 text-[11px] text-subtle">{app.t("settings.judgeNoActivity")}</div>
      ) : (
        <div className="mt-1 max-h-72 space-y-1 overflow-y-auto">
          {activity.map((x, i) => (
            <div key={i} className={cn("flex items-start gap-2 rounded-lg border px-2.5 py-1.5 text-[11px]", x.ok ? "border-line" : "border-rose-500/40 bg-rose-500/5")}>
              <span className="chip shrink-0 border-fuchsia-500/40 bg-fuchsia-500/10 text-fuchsia-600 dark:text-fuchsia-300">Jev</span>
              <Mono className="shrink-0 text-subtle">{x.use}</Mono>
              {x.ticket && <Mono className="shrink-0 text-cyan-600 dark:text-cyan-400">{x.ticket}</Mono>}
              <span className={cn("min-w-0 flex-1", !x.ok && "text-rose-600 dark:text-rose-300")}>{x.outcome}</span>
              <Mono className="shrink-0 text-subtle">
                {x.ms} ms · {ago(x.at)}
              </Mono>
            </div>
          ))}
        </div>
      )}
    </Section>
  );
}

/** How agent shell commands are approved and whether this machine has a working sandbox (ADR-64). */
function CommandPolicySection() {
  const app = useApp();
  const [p, setP] = useState<CommandPolicy | null>(null);
  const [ask, setAsk] = useState(false);
  useEffect(() => {
    api.commandPolicy().then(setP).catch(() => setP(null));
  }, []);
  const setUnsandboxed = (on: boolean) => api.setUnsandboxed(on).then(setP).catch((e) => app.notify(String(e)));
  if (!p) return null;
  return (
    <Section title={app.t("settings.commands")}>
      <div className="flex flex-wrap gap-2">
        {(["sandboxed", "on-request", "never"] as const).map((m) => (
          <button
            key={m}
            className={cn("btn", p.approval === m && "border-accent text-accent")}
            onClick={() => api.setCommandPolicy(m).then(setP).catch((e) => app.notify(String(e)))}
          >
            {app.t(`settings.commands.${m}`)}
          </button>
        ))}
      </div>
      <div className="mt-2 text-[11px] text-subtle">{app.t(`settings.commands.${p.approval}.hint`)}</div>
      <div className={cn("mt-2 flex items-center gap-1.5 text-[11px]", p.backend ? "text-emerald-600 dark:text-emerald-300" : "text-amber-600 dark:text-amber-300")}>
        {p.backend ? <ShieldCheck className="h-3.5 w-3.5" /> : <ShieldAlert className="h-3.5 w-3.5" />}
        {p.backend ? app.t("settings.sandboxOn") : app.t("settings.sandboxOff")}
      </div>
      {!p.backend && (
        <label className="mt-3 flex items-start gap-2 rounded-lg border border-amber-500/40 bg-amber-500/5 p-2.5 text-xs">
          <input
            type="checkbox"
            className="mt-0.5"
            checked={p.unsandboxed}
            onChange={(e) => (e.target.checked ? setAsk(true) : setUnsandboxed(false))}
          />
          <span>
            <span className="font-medium">{app.t("settings.unsandboxed")}</span>
            <span className="mt-0.5 block text-[11px] text-subtle">{app.t("settings.unsandboxed.hint")}</span>
          </span>
        </label>
      )}
      <ConfirmModal
        open={ask}
        title={app.t("settings.unsandboxed")}
        note={app.t("settings.unsandboxed.warn")}
        confirm={app.t("settings.unsandboxed.ok")}
        icon={<ShieldAlert className="h-3.5 w-3.5" />}
        onClose={() => setAsk(false)}
        onConfirm={() => (setAsk(false), setUnsandboxed(true))}
      />
    </Section>
  );
}

/** The embedding model is put in place before the app opens (memory recalls by meaning with it; ADR-47). */
export function ModelPrepView() {
  const app = useApp();
  const m = app.status!.model!;
  return (
    <div className="flex h-full items-center justify-center bg-background p-6">
      <div className="card w-[460px] p-6">
        <div className="mb-4 flex items-center gap-3">
          <Logo className="h-10 w-10 shrink-0 rounded-lg shadow-sm" />
          <div className="text-base font-semibold">{app.t("model.title")}</div>
        </div>
        <div className="text-xs text-muted">{app.t("model.why").replace("{mb}", String(m.mb))}</div>
        {m.phase === "error" ? (
          <>
            <div className="mt-3 rounded-lg border border-rose-500/40 bg-rose-500/10 p-2 text-[11px] text-rose-600 dark:text-rose-300">{m.error}</div>
            <button className="btn-primary mt-4 w-full justify-center" onClick={() => api.retryEmbeddingModel().then(app.reloadStatus)}>
              {app.t("model.retry")}
            </button>
          </>
        ) : (
          <div className="mt-3 flex items-center gap-2 text-[11px] text-subtle">
            <Loader2 className="h-3.5 w-3.5 animate-spin" />
            <Mono>{m.step || app.t(m.phase === "checking" ? "model.checking" : "model.installing")}</Mono>
          </div>
        )}
      </div>
    </div>
  );
}

export function SetupView() {
  const app = useApp();
  const [ui, setUi] = useState(app.lang);
  return (
    <div className="flex h-full items-center justify-center bg-background p-6">
      <div className="card w-[460px] p-6">
        <div className="mb-4 flex items-center gap-3">
          <Logo className="h-10 w-10 shrink-0 rounded-lg shadow-sm" />
          <div className="text-base font-semibold">{app.t("setup.title")}</div>
        </div>
        <label className="block text-xs">
          <span className="label">{app.t("setup.ui")}</span>
          <select className="input mt-1" value={ui} onChange={(e) => (setUi(e.target.value as "vi" | "en"), app.setLang(e.target.value as "vi" | "en"))}>
            <option value="vi">Tiếng Việt</option>
            <option value="en">English</option>
          </select>
        </label>
        <div className="mt-2 text-[11px] text-subtle">{app.t("settings.languageNote")}</div>
        <button className="btn-primary mt-4 w-full justify-center" onClick={() => api.setup(ui).then(app.reloadStatus).catch((e) => app.notify(String(e)))}>
          {app.t("setup.go")}
        </button>
      </div>
    </div>
  );
}
