import { FreeTag } from "../../components/ui";
import { isFree, priceOf } from "../../lib/prices";
import { useApp } from "../../lib/store";
import type { PriceView, VendorUsage } from "../../lib/types";

/** Where a run ran and how it is paid: a provider's API (per token) or a vendor's CLI (its subscription). */
export interface Account {
  key: string;
  label: string;
  provider: string;
  cli: boolean;
}

const CLI: Record<string, string> = { claude: "Claude Code", codex: "Codex", grok: "Grok", agy: "Antigravity", opencode: "OpenCode" };

export function accountOf(provider: string, cli: boolean): Account {
  return { key: `${cli ? "cli" : "api"}:${provider}`, label: cli ? `${CLI[provider] ?? provider} · CLI` : `${provider} · API`, provider, cli };
}

/** A subscription's usage read from the vendor: a CLI's own (`claude`, `codex`, `agy`), OpenCode's Go plan for its CLI
 *  and for an API provider on that plan (`opencode-go:<provider>`). */
function subscription(a: Account, vendors: VendorUsage[]): VendorUsage | undefined {
  if (a.cli) return vendors.find((v) => v.vendor === a.provider || (a.provider === "opencode" && v.vendor.startsWith("opencode-go:")));
  return vendors.find((v) => v.vendor === `opencode-go:${a.provider}`);
}

const usd = (n: number) => `$${n < 1 ? n.toFixed(3).replace(/0$/, "") : n.toFixed(2)}`;

/** Each vendor's prices for the models in use: an API's US dollars per million tokens (the price catalogue, or
 *  pricing.toml), a CLI's subscription plan and how much of its windows is used. */
export function PricesTable({ models, prices, vendors }: { models: { account: Account; model: string }[]; prices: PriceView | null; vendors: VendorUsage[] }) {
  const app = useApp();
  return (
    <div className="overflow-x-auto">
      <table className="w-full text-xs">
        <thead>
          <tr className="label border-b border-line text-left">
            <th className="py-2 pr-3">{app.t("usage.vendor")}</th>
            <th className="px-3">{app.t("common.model")}</th>
            <th className="px-3 text-right">{app.t("usage.in")}</th>
            <th className="px-3 text-right">{app.t("usage.out")}</th>
            <th className="px-3 text-right">{app.t("usage.cacheRead")}</th>
            <th className="px-3 text-right">{app.t("usage.cacheWrite")}</th>
            <th className="px-3">{app.t("usage.subscription")}</th>
          </tr>
        </thead>
        <tbody className="divide-y divide-line">
          {models.map(({ account, model }, i) => {
            const p = account.cli ? undefined : priceOf(prices, account.provider, model);
            const sub = subscription(account, vendors);
            const first = i === 0 || models[i - 1].account.key !== account.key;
            const cell = (n?: number) => <td className="px-3 text-right font-mono" style={{ fontVariantNumeric: "tabular-nums" }}>{n === undefined ? "–" : usd(n)}</td>;
            return (
              <tr key={`${account.key}|${model}`}>
                <td className="py-1.5 pr-3 font-medium">{first ? account.label : ""}</td>
                <td className="px-3 text-muted">
                  {model} {isFree(p) && <FreeTag />}
                </td>
                {account.cli ? (
                  <td colSpan={4} className="px-3 text-subtle">
                    {app.t("usage.cliPaid")}
                  </td>
                ) : p ? (
                  <>
                    {cell(p.input)}
                    {cell(p.output)}
                    {cell(p.cache_read)}
                    {cell(p.cache_write)}
                  </>
                ) : (
                  <td colSpan={4} className="px-3 text-subtle">
                    {app.t("usage.noPrice")}
                  </td>
                )}
                <td className="px-3 text-muted">
                  {first && sub
                    ? [sub.plan, sub.windows.map((w) => `${w.window} ${Math.round(w.used_percent)}%`).join(" · ")].filter(Boolean).join(" — ")
                    : first && account.cli
                      ? "–"
                      : ""}
                </td>
              </tr>
            );
          })}
        </tbody>
      </table>
    </div>
  );
}
