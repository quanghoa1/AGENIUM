import { useEffect, useRef, useState } from "react";
import { ModelsReload } from "../../components/ModelsReload";
import { DeprecatedTag, FreeTag, cn } from "../../components/ui";
import { api, type ModelArg } from "../../lib/api";
import { deprecation, isFree, priceLabel, priceOf, usePrices } from "../../lib/prices";
import { useApp } from "../../lib/store";
import type { Capabilities, ModelInfo } from "../../lib/types";

/** Provider (API) or CLI program, model and effort. The model list comes from the provider when it can list them. */
export function ModelPicker({ value, onChange, caps }: { value: ModelArg; onChange: (m: ModelArg) => void; caps: Capabilities | null }) {
  const app = useApp();
  const cli = value.backend === "cli";
  const [models, setModels] = useState<ModelInfo[]>([]);
  useEffect(() => {
    setModels([]);
    if (cli) {
      // What this CLI runs with the user's login (codex model/list, agy models, Claude aliases, the user's OpenCode
      // account: its Go subscription and Zen; OpenCode's free models when not signed in).
      setModels(caps?.cli_models?.[value.provider] ?? []);
      return;
    }
    if (!value.provider) return;
    const listed = caps?.providers.find((p) => p.id === value.provider)?.models ?? [];
    if (listed.length) setModels(listed.map((id) => ({ id })));
    // A late answer for a provider the user already left is dropped.
    let stale = false;
    api
      .providerModels(value.provider)
      .then((c) => !stale && c.value.length && setModels(c.value))
      .catch(() => undefined);
    return () => {
      stale = true;
    };
  }, [value.provider, cli, caps]);
  const listId = `models-${value.provider || "none"}`;
  // Only the model's own effort levels (ADR-114): reported by its vendor or known per model. [] = the model has no
  // effort setting; unknown = its vendor's default applies.
  const info = models.find((m) => m.id === value.model);
  const levels = info?.efforts ?? null;
  const levelKey = levels?.join(",");
  const shownModel = useRef(value.model);
  useEffect(() => {
    // A newly chosen model starts at its own default level; a level the model does not have becomes its default too.
    const changed = shownModel.current !== value.model;
    shownModel.current = value.model;
    if (levels?.length && (changed || !levels.includes(value.effort))) {
      const d = info?.default_effort;
      const effort = d && levels.includes(d) ? d : levels.includes(value.effort) ? value.effort : levels[0];
      if (effort !== value.effort) onChange({ ...value, effort });
    }
  }, [value.model, levelKey]);
  const prices = usePrices(app.tick);
  // The opencode CLI lists the user's OpenCode account: `opencode-go/…` is paid by the Go subscription, `opencode/…` from
  // Zen's credit, a `-free` model (and big-pickle) by nobody. The other CLIs list what their login can run.
  const opencode = cli && value.provider === "opencode";
  const ocName = (id: string) => (id.includes("/") ? id.split("/", 2) : ["opencode", id]);
  const ocFree = (id: string) => {
    const m = ocName(id)[1];
    return m.endsWith("-free") || m === "big-pickle";
  };
  const ocBill = (id: string) => (!opencode || ocFree(id) ? "" : ocName(id)[0] === "opencode-go" ? app.t("model.goPlan") : ocName(id)[0] === "opencode" ? app.t("model.zenCredit") : "");
  const price = (id: string) => (cli ? (opencode && ocFree(id) ? { input: 0, output: 0, cache_read: 0, cache_write: 0 } : undefined) : priceOf(prices, value.provider, id));
  const old = (id: string) => deprecation(prices, value.provider, id);
  const picked = old(value.model);
  return (
    <div className="grid grid-cols-4 gap-2">
      <select className="input" value={value.backend ?? "api"} onChange={(e) => onChange({ ...value, backend: e.target.value as "api" | "cli", provider: "", model: "" })}>
        <option value="api">API</option>
        <option value="cli">CLI</option>
      </select>
      <select className="input" value={value.provider} onChange={(e) => onChange({ ...value, provider: e.target.value, model: "" })}>
        <option value="">{cli ? "CLI…" : app.t("model.provider")}</option>
        {cli
          ? caps?.clis
              .filter((c) => c.compatible)
              .map((c) => (
                // Signed out: not offered (its runs would fail at once). Unknown: offered, and said so.
                <option key={c.kind} value={c.kind.toLowerCase()} disabled={c.login === "not_logged_in"}>
                  {c.kind.toLowerCase()} {c.version}
                  {c.login === "not_logged_in" ? ` (${app.t("providers.notLoggedIn")})` : c.login === "logged_in" ? "" : ` (${app.t("model.loginUnknown")})`}
                </option>
              ))
          : caps?.providers.map((p) => (
              <option key={p.id} value={p.id}>
                {p.id}
              </option>
            ))}
      </select>
      <div className="flex items-center gap-1">
        <div className="relative min-w-0 flex-1">
          <input className={cn("input w-full", isFree(price(value.model)) && "pr-14", picked && "pr-28")} list={listId} placeholder="model" value={value.model} onChange={(e) => onChange({ ...value, model: e.target.value })} />
          {picked ? (
            <DeprecatedTag instead={picked.instead} className="absolute right-2 top-1/2 -translate-y-1/2" />
          ) : (
            isFree(price(value.model)) && <FreeTag className="absolute right-2 top-1/2 -translate-y-1/2" />
          )}
        </div>
        {/* The list as kept; this lists the provider's or the CLI's models again now. */}
        {value.provider && (
          <ModelsReload
            compact
            at={caps?.models_at?.[`${cli ? "cli" : "api"}:${value.provider}`]?.at}
            onReload={() => (cli ? api.reloadCliModels(value.provider) : api.providerModels(value.provider, true)).then((c) => setModels(c.value))}
          />
        )}
      </div>
      <datalist id={listId}>
        {models.map((m) => {
          const p = price(m.id);
          return (
            <option key={m.id} value={m.id}>
              {[old(m.id) ? app.t("model.deprecatedTag").toUpperCase() : "", isFree(p) ? app.t("model.freeTag").toUpperCase() : p ? app.t("model.perMillion").replace("{p}", priceLabel(p)) : ocBill(m.id), m.supports_tools === false ? app.t("model.noTools") : ""].filter(Boolean).join(" · ")}
            </option>
          );
        })}
      </datalist>
      {levels?.length ? (
        <select className="input" value={value.effort} onChange={(e) => onChange({ ...value, effort: e.target.value })}>
          {levels.map((x) => (
            <option key={x} value={x}>
              {x === info?.default_effort ? `${x} (${app.t("model.default")})` : x}
            </option>
          ))}
        </select>
      ) : (
        <select className="input" disabled title={levels ? app.t("model.noEffortTitle") : app.t("model.vendorDefaultTitle")}>
          <option>{levels ? app.t("model.noEffort") : app.t("model.vendorDefault")}</option>
        </select>
      )}
      {picked && (
        <div className="col-span-4 flex flex-wrap items-center gap-2 text-[11px] text-amber-600 dark:text-amber-300">
          <span>{app.t("model.deprecatedPicked")}</span>
          {picked.instead && (
            <button type="button" className="btn" onClick={() => onChange({ ...value, model: picked.instead! })}>
              {app.t("model.useInstead")} {picked.instead}
            </button>
          )}
        </div>
      )}
    </div>
  );
}

export const emptyModel: ModelArg = { provider: "", model: "", effort: "high", backend: "api" };

export function AgentModel({ id, backend, model, onSaved }: { id: string; backend: string; model: { provider: string; model: string; effort: string }; onSaved: () => void }) {
  const app = useApp();
  const [caps, setCaps] = useState<Capabilities | null>(null);
  const [m, setM] = useState<ModelArg>({ provider: model.provider, model: model.model, effort: model.effort, backend: backend === "Cli" ? "cli" : "api" });
  useEffect(() => {
    api.capabilities().then(setCaps);
  }, []);
  return (
    <div className="space-y-2">
      <ModelPicker value={m} onChange={setM} caps={caps} />
      <div className="text-[11px] text-subtle">{app.t("agents.keep")}</div>
      <button
        className="btn-primary"
        disabled={!m.provider || (m.provider === model.provider && m.model === model.model && m.effort === model.effort && m.backend === (backend === "Cli" ? "cli" : "api"))}
        onClick={() =>
          api
            .setAgentModel(id, m)
            .then(() => (onSaved(), app.refresh(), app.notify(app.t("common.done"))))
            .catch((e) => app.notify(String(e)))
        }
      >
        {app.t("agents.saveModel")}
      </button>
    </div>
  );
}
