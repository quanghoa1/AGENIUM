import { RefreshCw } from "lucide-react";
import { useEffect, useMemo, useState } from "react";
import { Segmented } from "../../components/motion";
import { Empty, FreeTag, Metric, PageHeader, Section, ago, cn, tokens } from "../../components/ui";
import { api } from "../../lib/api";
import type { Lang } from "../../lib/i18n";
import { isFree, priceOf, usePrices } from "../../lib/prices";
import { useApp } from "../../lib/store";
import type { AgentEntry, RunRow, VendorUsage } from "../../lib/types";
import { BarList, Legend, SeriesTable, StackedColumns, type Series } from "./charts";
import { PricesTable, accountOf, type Account } from "./prices";

type Range = "7" | "30" | "90" | "all";
type Measure = "tokens" | "cost";

/** Accounts drawn in their own colour; the rest fold into "other". */
const NAMED = 7;
const OTHER = "var(--series-other)";
const DAY = 86_400_000;

/** Compact counts for axes and labels: 950, 12.5k, 1.2M. */
const count = (n: number) => (n >= 1e6 ? `${+(n / 1e6).toFixed(1)}M` : n >= 1e3 ? `${+(n / 1e3).toFixed(1)}k` : String(Math.round(n)));
const money = (n: number) => (n === 0 ? "$0" : n >= 1000 ? `$${+(n / 1000).toFixed(1)}k` : n >= 0.1 ? `$${n.toFixed(2)}` : `$${+n.toFixed(4)}`);

const midnight = (d: Date) => new Date(d.getFullYear(), d.getMonth(), d.getDate());
const monday = (d: Date) => {
  const m = midnight(d);
  m.setDate(m.getDate() - ((m.getDay() + 6) % 7));
  return m;
};
const dayKey = (d: Date) => `${d.getFullYear()}-${String(d.getMonth() + 1).padStart(2, "0")}-${String(d.getDate()).padStart(2, "0")}`;

/** The periods a range is drawn in: days, or weeks past 90 days. */
function periodsOf(range: Range, first: Date | null, lang: Lang) {
  const today = midnight(new Date());
  const from = range === "all" ? midnight(first ?? today) : new Date(today.getTime() - (Number(range) - 1) * DAY);
  const week = Math.round((today.getTime() - from.getTime()) / DAY) + 1 > 90;
  const start = week ? monday(from) : from;
  const keys: string[] = [];
  const names: string[] = [];
  for (let d = start; d <= today; d = new Date(d.getFullYear(), d.getMonth(), d.getDate() + (week ? 7 : 1))) {
    keys.push(dayKey(d));
    names.push(d.toLocaleDateString(lang === "vi" ? "vi-VN" : "en-US", { day: "numeric", month: "short" }));
  }
  return { start, keys, names, week, bucket: (d: Date) => dayKey(week ? monday(d) : midnight(d)) };
}

interface Tally {
  tokens: number;
  cost: number;
  runs: number;
}

/** Token use and cost over time, by vendor account and by model, with the vendors' prices and each agent's share. */
export function UsageView() {
  const app = useApp();
  const [rows, setRows] = useState<RunRow[]>([]);
  const [agents, setAgents] = useState<AgentEntry[]>([]);
  const [vendors, setVendors] = useState<VendorUsage[]>([]);
  const [range, setRange] = useState<Range>("30");
  const [measure, setMeasure] = useState<Measure>("tokens");
  const [asTable, setAsTable] = useState<Record<string, boolean>>({});
  const [refreshing, setRefreshing] = useState(false);
  const prices = usePrices(app.tick);
  useEffect(() => {
    api.runs(5000).then(setRows);
    api.agents().then((a) => setAgents(a ?? []));
    api.vendorUsage().then(setVendors, () => undefined);
  }, [app.tick]);

  // Colours follow the account, never its rank: given over every run, so a narrower range never repaints them.
  const colour = useMemo(() => {
    const keys = [...new Set(rows.map((r) => accountOf(r.provider, r.api === false).key))].sort();
    return new Map(keys.map((k, i) => [k, i < NAMED ? `var(--series-${i + 1})` : OTHER]));
  }, [rows]);

  const v = useMemo(() => {
    const first = rows.length ? new Date(Math.min(...rows.map((r) => Date.parse(r.started_at)))) : null;
    const per = periodsOf(range, first, app.lang);
    const shown = rows.filter((r) => Date.parse(r.started_at) >= per.start.getTime());
    const at = new Map(per.keys.map((k, i) => [k, i]));
    const tok = new Map<string, Series>();
    const cost = new Map<string, Series>();
    const accounts = new Map<string, Tally & { a: Account }>();
    const models = new Map<string, Tally & { a: Account; model: string }>();
    // `work`: the runs that can complete (memory runs cannot).
    const byAgent = new Map<string, Tally & { ok: number; work: number; tin: number; tout: number; model: string; provider: string }>();
    let tin = 0;
    let tout = 0;
    let total = 0;
    let ok = 0;
    let finished = 0;
    for (const r of shown) {
      const a = accountOf(r.provider, r.api === false);
      const t = r.usage.tokens_in + r.usage.tokens_out;
      const c = r.usage.estimated_cost_usd;
      tin += r.usage.tokens_in;
      tout += r.usage.tokens_out;
      total += c;
      if (r.outcome === "Completed") ok++;
      if (r.outcome !== "Memory") finished++;
      const col = colour.get(a.key) ?? OTHER;
      const sk = col === OTHER ? "other" : a.key;
      const i = at.get(per.bucket(new Date(r.started_at)));
      if (i !== undefined) {
        for (const [m, n] of [
          [tok, t],
          [cost, c],
        ] as const) {
          const s = m.get(sk) ?? { key: sk, label: sk === "other" ? app.t("usage.other") : a.label, color: col, values: per.keys.map(() => 0) };
          s.values[i] += n;
          m.set(sk, s);
        }
      }
      const acc = accounts.get(a.key) ?? { a, tokens: 0, cost: 0, runs: 0 };
      acc.tokens += t;
      acc.cost += c;
      acc.runs++;
      accounts.set(a.key, acc);
      const mk = `${a.key}|${r.model}`;
      const mod = models.get(mk) ?? { a, model: r.model, tokens: 0, cost: 0, runs: 0 };
      mod.tokens += t;
      mod.cost += c;
      mod.runs++;
      models.set(mk, mod);
      const ak = r.agent_name ?? r.agent;
      const ag = byAgent.get(ak) ?? { tokens: 0, cost: 0, runs: 0, ok: 0, work: 0, tin: 0, tout: 0, model: r.model, provider: r.provider };
      ag.runs++;
      if (r.outcome === "Completed") ag.ok++;
      if (r.outcome !== "Memory") ag.work++;
      ag.tin += r.usage.tokens_in;
      ag.tout += r.usage.tokens_out;
      ag.cost += c;
      byAgent.set(ak, ag);
    }
    // Series in their colour order, "other" last.
    const order = (m: Map<string, Series>) => [...m.values()].sort((x, y) => (x.key === "other" ? 1 : y.key === "other" ? -1 : x.key.localeCompare(y.key)));
    return {
      per,
      shown,
      tok: order(tok),
      cost: order(cost),
      accounts: [...accounts.values()],
      models: [...models.values()],
      byAgent: [...byAgent.entries()].sort((a, b) => b[1].tin - a[1].tin),
      tin,
      tout,
      total,
      ok,
      finished,
    };
  }, [rows, range, colour, app.lang, app.t]);

  // The prices of every model in use or set on an agent, by account.
  const priced = useMemo(() => {
    const m = new Map<string, { account: Account; model: string }>();
    for (const r of rows) {
      const a = accountOf(r.provider, r.api === false);
      m.set(`${a.key}|${r.model}`, { account: a, model: r.model });
    }
    for (const e of agents) {
      const a = accountOf(e.agent.model.provider, e.agent.backend === "Cli");
      m.set(`${a.key}|${e.agent.model.model}`, { account: a, model: e.agent.model.model });
    }
    return [...m.values()].sort((x, y) => x.account.label.localeCompare(y.account.label) || x.model.localeCompare(y.model));
  }, [rows, agents]);

  const value = (x: Tally) => (measure === "tokens" ? x.tokens : x.cost);
  const fmt = measure === "tokens" ? count : money;
  const runsWord = app.t("usage.runs");
  const per = app.t(v.per.week ? "usage.week" : "usage.day");
  const chart = (title: string, series: Series[], format: (n: number) => string) => (
    <Section
      title={title}
      right={
        <Segmented
          id={`usage-view-${title}`}
          value={asTable[title] ? "table" : "chart"}
          onChange={(x) => setAsTable((t) => ({ ...t, [title]: x === "table" }))}
          itemClassName="px-2 py-0.5 text-[11px]"
          items={[
            { value: "chart", label: app.t("usage.chart") },
            { value: "table", label: app.t("usage.table") },
          ]}
        />
      }
    >
      <Legend series={series} />
      {asTable[title] ? (
        <SeriesTable names={v.per.names} series={series} format={format} total={app.t("usage.total")} />
      ) : (
        <StackedColumns label={title} periods={v.per.keys} names={v.per.names} series={series} format={format} />
      )}
    </Section>
  );

  return (
    <div className="space-y-4">
      <PageHeader title={app.t("nav.usage")} sub={app.t("usage.sub")}>
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

      {/* One row of filters above everything they scope. */}
      <div className="flex flex-wrap items-center gap-3">
        <Segmented
          id="usage-range"
          value={range}
          onChange={setRange}
          items={(["7", "30", "90", "all"] as Range[]).map((r) => ({ value: r, label: app.t(r === "all" ? "usage.range.all" : "usage.range.days").replace("{n}", r) }))}
        />
        <Segmented
          id="usage-measure"
          value={measure}
          onChange={setMeasure}
          items={[
            { value: "tokens", label: app.t("usage.measure.tokens") },
            { value: "cost", label: app.t("usage.measure.cost") },
          ]}
        />
      </div>

      <div className="grid grid-cols-2 gap-4 xl:grid-cols-4">
        <Metric label={app.t("usage.cost")} value={money(v.total)} sub={`${v.shown.length} ${runsWord}`} />
        <Metric label={app.t("usage.tokensIn")} value={count(v.tin)} />
        <Metric label={app.t("usage.tokensOut")} value={count(v.tout)} />
        <Metric label={app.t("usage.completed")} value={v.finished ? `${Math.round((100 * v.ok) / v.finished)}%` : "–"} sub={`${v.ok} / ${v.finished} ${runsWord}`} />
      </div>

      {v.shown.length === 0 ? (
        <Empty>{app.t("usage.none")}</Empty>
      ) : (
        <>
          <div className="grid gap-4 xl:grid-cols-2">
            {chart(app.t("usage.tokensPer").replace("{p}", per), v.tok, count)}
            {chart(app.t("usage.costPer").replace("{p}", per), v.cost, money)}
          </div>
          <div className="grid gap-4 xl:grid-cols-2">
            <Section title={`${app.t("usage.byVendor")} · ${app.t(measure === "tokens" ? "usage.measure.tokens" : "usage.measure.cost")}`}>
              <BarList
                format={fmt}
                rows={[...v.accounts]
                  .sort((a, b) => value(b) - value(a))
                  .map((x) => ({ key: x.a.key, label: x.a.label, sub: `${x.runs} ${runsWord} · ${count(x.tokens)} tokens · ${money(x.cost)}`, value: value(x), color: colour.get(x.a.key) ?? OTHER }))}
              />
            </Section>
            <Section title={`${app.t("usage.byModel")} · ${app.t(measure === "tokens" ? "usage.measure.tokens" : "usage.measure.cost")}`}>
              <BarList
                format={fmt}
                rows={[...v.models]
                  .sort((a, b) => value(b) - value(a))
                  .slice(0, 12)
                  .map((x) => ({ key: `${x.a.key}|${x.model}`, label: x.model, sub: `${x.a.label} · ${x.runs} ${runsWord}`, value: value(x), color: colour.get(x.a.key) ?? OTHER }))}
              />
            </Section>
          </div>
        </>
      )}

      <Section title={app.t("usage.prices")} right={<span className="text-[11px] text-subtle">{app.t("usage.pricesNote")}</span>}>
        <PricesTable models={priced} prices={prices} vendors={vendors} />
      </Section>

      <Section title={app.t("usage.byAgent")}>
        <div className="overflow-x-auto">
          <table className="w-full text-xs">
            <thead>
              <tr className="label border-b border-line text-left">
                <th className="py-2 pr-3">{app.t("runs.agent")}</th>
                <th className="px-3">{app.t("common.model")}</th>
                <th className="px-3">{app.t("usage.runs")}</th>
                <th className="px-3">{app.t("usage.completed")}</th>
                <th className="px-3">{app.t("usage.tokensIn")}</th>
                <th className="px-3">{app.t("usage.tokensOut")}</th>
                <th className="px-3">{app.t("ticket.cost")}</th>
              </tr>
            </thead>
            <tbody className="divide-y divide-line">
              {v.byAgent.map(([k, x]) => (
                <tr key={k}>
                  <td className="py-2 pr-3 font-medium">{k}</td>
                  <td className="px-3 text-muted">
                    {x.model} {isFree(priceOf(prices, x.provider, x.model)) && <FreeTag />}
                  </td>
                  <td className="px-3 font-mono">{x.runs}</td>
                  <td className="px-3 font-mono">{x.work ? `${Math.round((100 * x.ok) / x.work)}%` : "–"}</td>
                  <td className="px-3 font-mono">{tokens(x.tin)}</td>
                  <td className="px-3 font-mono">{tokens(x.tout)}</td>
                  <td className="px-3 font-mono">${x.cost.toFixed(3)}</td>
                </tr>
              ))}
            </tbody>
          </table>
          {v.byAgent.length === 0 && <Empty>{app.t("common.none")}</Empty>}
        </div>
      </Section>
    </div>
  );
}
