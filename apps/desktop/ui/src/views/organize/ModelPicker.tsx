import { AlertTriangle, ChevronDown } from "lucide-react";
import { useEffect, useRef, useState } from "react";
import { ConfirmModal } from "../../components/motion";
import { ModelsReload } from "../../components/ModelsReload";
import { DeprecatedTag, FreeTag, cn, tokens } from "../../components/ui";
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
  // The model list opens under the field (a click on it or its arrow); typing filters it, and a model the list does
  // not have can still be typed in.
  const [open, setOpen] = useState(false);
  const [query, setQuery] = useState<string | null>(null);
  const box = useRef<HTMLDivElement>(null);
  useEffect(() => {
    if (!open) return;
    const away = (e: MouseEvent) => {
      if (box.current && !box.current.contains(e.target as Node)) setOpen(false);
    };
    document.addEventListener("mousedown", away);
    return () => document.removeEventListener("mousedown", away);
  }, [open]);
  const shown = models.filter((m) => !query || m.id.toLowerCase().includes(query.toLowerCase()));
  const tags = (m: ModelInfo) => {
    const p = price(m.id);
    return [
      old(m.id) ? app.t("model.deprecatedTag").toUpperCase() : "",
      isFree(p) ? app.t("model.freeTag").toUpperCase() : p ? app.t("model.perMillion").replace("{p}", priceLabel(p)) : ocBill(m.id),
      m.context_window ? `${tokens(m.context_window)} ctx` : "",
      m.supports_tools === false ? app.t("model.noTools") : "",
    ]
      .filter(Boolean)
      .join(" · ");
  };
  return (
    <div className="grid grid-cols-[4.75rem_minmax(0,1fr)_minmax(0,2.2fr)_minmax(0,1fr)] gap-2">
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
        <div ref={box} className="relative min-w-0 flex-1">
          <input
            className={cn("input w-full pr-8", isFree(price(value.model)) && "pr-24", picked && "pr-36")}
            placeholder={app.t("model.pick")}
            title={value.model}
            value={value.model}
            onFocus={() => (setQuery(null), setOpen(true))}
            onChange={(e) => (onChange({ ...value, model: e.target.value }), setQuery(e.target.value), setOpen(true))}
          />
          {picked ? (
            <DeprecatedTag instead={picked.instead} className="absolute right-8 top-1/2 -translate-y-1/2" />
          ) : (
            isFree(price(value.model)) && <FreeTag className="absolute right-8 top-1/2 -translate-y-1/2" />
          )}
          <button
            type="button"
            className="absolute right-1.5 top-1/2 -translate-y-1/2 rounded p-0.5 text-subtle hover:text-fg"
            aria-label={app.t("model.pick")}
            onMouseDown={(e) => e.preventDefault()}
            onClick={() => (setQuery(null), setOpen(!open))}
          >
            <ChevronDown className={cn("h-4 w-4 transition-transform", open && "rotate-180")} />
          </button>
          {open && models.length > 0 && (
            <div className="absolute left-0 top-full z-50 mt-1 max-h-80 w-[36rem] max-w-[80vw] overflow-y-auto rounded-lg border border-line bg-surface p-1 shadow-lg">
              {shown.map((m) => (
                <button
                  type="button"
                  key={m.id}
                  title={[m.id, tags(m)].filter(Boolean).join(" · ")}
                  className={cn(
                    "flex w-full items-center justify-between gap-3 rounded-md px-2.5 py-1.5 text-left text-xs hover:bg-elevated",
                    m.id === value.model && "bg-blue-600/10 text-blue-600 dark:text-blue-300",
                  )}
                  onMouseDown={(e) => e.preventDefault()}
                  onClick={() => (onChange({ ...value, model: m.id }), setQuery(null), setOpen(false))}
                >
                  <span className="min-w-0 break-all font-mono">{m.id}</span>
                  <span className="shrink-0 text-[11px] text-subtle">{tags(m)}</span>
                </button>
              ))}
              {shown.length === 0 && <div className="px-2.5 py-2 text-xs text-subtle">{app.t("model.noMatch")}</div>}
            </div>
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
      {!cli && value.model && levels === null && (
        <div className="col-span-4 text-[11px] text-subtle">
          {app.t("model.effortUnknown")}{" "}
          <button type="button" className="underline underline-offset-2 hover:text-fg" onClick={() => app.go("providers")}>
            {app.t("model.effortWhere")}
          </button>
        </div>
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
  // ADR-155: a model whose window the agent's context outgrows is said before the change, and asked about.
  const [fit, setFit] = useState<{ used: number; window: number; over: boolean } | null>(null);
  const [asking, setAsking] = useState(false);
  useEffect(() => {
    api.capabilities().then(setCaps);
  }, []);
  const same = m.provider === model.provider && m.model === model.model && m.backend === (backend === "Cli" ? "cli" : "api");
  useEffect(() => {
    setFit(null);
    if (!m.provider || !m.model || same) return;
    const t = window.setTimeout(() => api.contextCheck(id, m).then(setFit).catch(() => undefined), 300);
    return () => window.clearTimeout(t);
  }, [id, m.provider, m.model, m.backend, same]);
  const save = () =>
    api
      .setAgentModel(id, m)
      .then(() => (onSaved(), app.refresh(), app.notify(app.t("common.done"))))
      .catch((e) => app.notify(String(e)));
  const warning = fit?.over
    ? app.t("agents.contextWarn").replace("{used}", tokens(fit.used)).replace("{window}", tokens(fit.window))
    : null;
  return (
    <div className="space-y-2">
      <ModelPicker value={m} onChange={setM} caps={caps} />
      <div className="text-[11px] text-subtle">{app.t("agents.keep")}</div>
      {warning && (
        <div className="flex items-start gap-2 rounded-lg border border-amber-500/40 bg-amber-500/10 p-2.5 text-[11px] text-amber-700 dark:text-amber-300">
          <AlertTriangle className="mt-0.5 h-3.5 w-3.5 shrink-0" />
          <span>{warning}</span>
        </div>
      )}
      <button
        className="btn-primary"
        disabled={!m.provider || (same && m.effort === model.effort)}
        onClick={() => (warning ? setAsking(true) : save())}
      >
        {app.t("agents.saveModel")}
      </button>
      <ConfirmModal
        open={asking}
        title={app.t("agents.contextWarnTitle")}
        note={warning ?? ""}
        confirm={app.t("agents.saveModel")}
        onClose={() => setAsking(false)}
        onConfirm={() => {
          setAsking(false);
          save();
        }}
      />
    </div>
  );
}
