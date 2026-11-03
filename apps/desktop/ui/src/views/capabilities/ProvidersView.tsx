import { AlertTriangle, CheckCircle2, KeyRound, Laptop, ListTree, Pencil, Plus, RefreshCw, Trash2, XCircle } from "lucide-react";
import { Fragment, useEffect, useState } from "react";
import { ConfirmModal } from "../../components/motion";
import { ModelsReload } from "../../components/ModelsReload";
import { Empty, Mono, PageHeader, Section, ago, cn } from "../../components/ui";
import { api } from "../../lib/api";
import type { Key } from "../../lib/i18n";
import { useApp } from "../../lib/store";
import type { Capabilities, ModelInfo } from "../../lib/types";
import { CliVersionCell, useCliUpdates } from "./CliUpdates";
import { PRESETS, Preset, ProviderDraft, ProviderForm, capsDraft } from "./ProviderForm";

/** A provider's models as kept; `at` changes after a reload, and the list is read again. */
function ModelsList({ id, at }: { id: string; at?: string | null }) {
  const app = useApp();
  const [ms, setMs] = useState<ModelInfo[] | null>(null);
  const [error, setError] = useState<string | null>(null);
  useEffect(() => {
    api
      .providerModels(id)
      .then((c) => (setMs(c.value), setError(c.value.length ? null : (c.error ?? null))))
      .catch((e) => setError(String(e)));
  }, [id, at]);
  if (error) return <div className="text-[11px] text-rose-600 dark:text-rose-400">{error}</div>;
  if (!ms) return <div className="text-[11px] text-subtle">{app.t("common.loading")}</div>;
  const flag = (b?: boolean | null) => (b === true ? <CheckCircle2 className="h-3.5 w-3.5 text-emerald-500" /> : b === false ? <XCircle className="h-3.5 w-3.5 text-rose-500" /> : <span className="text-subtle">?</span>);
  return (
    <div className="max-h-64 overflow-y-auto">
      <table className="w-full text-[11px]">
        <thead>
          <tr className="label text-left">
            <th className="py-1">model</th>
            <th>tools</th>
            <th>vision</th>
            <th>context</th>
          </tr>
        </thead>
        <tbody className="divide-y divide-line">
          {ms.map((m) => (
            <tr key={m.id}>
              <td className="py-1 font-mono">{m.id}</td>
              <td>{flag(m.supports_tools)}</td>
              <td>{flag(m.supports_vision)}</td>
              <td className="font-mono text-subtle">{m.context_window ? `${Math.round(m.context_window / 1000)}k` : "–"}</td>
            </tr>
          ))}
        </tbody>
      </table>
      {ms.length === 0 && <Empty>{app.t("common.none")}</Empty>}
    </div>
  );
}

/** The detector's note on a CLI in the interface language (its English sentences are for the logs). */
function cliNote(note: string | null | undefined, t: (k: Key) => string): string {
  if (!note) return "";
  if (note.startsWith("Agenium does not drive")) return t("providers.noteNotDriven");
  if (note.startsWith("cannot read its version")) return t("providers.noteNoVersion");
  const old = note.match(/older than the checked minimum (\S+)/);
  return old ? t("providers.noteOld").replace("{x}", old[1]) : note;
}

/** A CLI's login in words: signed in, not signed in, or not known (the CLI gives no way to ask). */
function LoginState({ login, installed }: { login: unknown; installed: boolean }) {
  const app = useApp();
  if (!installed) return <span className="text-subtle">–</span>;
  const [key, tone] =
    login === "logged_in"
      ? (["providers.loggedIn", "text-emerald-600 dark:text-emerald-300"] as const)
      : login === "not_logged_in"
        ? (["providers.notLoggedIn", "text-rose-600 dark:text-rose-300"] as const)
        : (["providers.loginUnknown", "text-amber-600 dark:text-amber-300"] as const);
  return <span className={tone}>{app.t(key)}</span>;
}

/** What the picked vendor is, when it needs saying: OpenCode Go and Zen take the same key and bill differently. */
function PresetNote({ draft, vendors }: { draft: ProviderDraft; vendors: Preset[] }) {
  const app = useApp();
  const same = (p: Preset) => !p.local && !draft.local && (p.base_url ?? "") === draft.base_url && p.kind === draft.kind;
  const p = PRESETS.find((x) => same(x) && x.note) ?? vendors.find((x) => same(x) && x.hint);
  if (!p) return null;
  return <div className="text-[11px] text-subtle">{p.note ? app.t(p.note) : p.hint}</div>;
}

export function ProvidersView() {
  const app = useApp();
  const [c, setC] = useState<Capabilities | null>(null);
  const [draft, setDraft] = useState<ProviderDraft | null>(null);
  const [models, setModels] = useState<string | null>(null);
  const [del, setDel] = useState<string | null>(null);
  const [scanning, setScanning] = useState(false);
  const updates = useCliUpdates(app.tick);
  const [vendors, setVendors] = useState<Preset[]>([]);
  const load = (refresh = false) => api.capabilities(refresh).then(setC);
  useEffect(() => {
    // The configured providers show at once; the agent CLIs (found by running each one) fill in after.
    api.capabilities(false, false).then((x) => setC((cur) => cur ?? x));
    load();
  }, [app.tick]);
  useEffect(() => {
    api
      .providerPresets()
      .then((v) => setVendors((v ?? []).map((p) => ({ id: p.id, label: p.name, kind: p.kind, base_url: p.base_url ?? undefined, key: p.key_env !== "", local: p.key_env === "", hint: p.note || undefined }))))
      .catch(() => setVendors([]));
  }, []);
  if (!c)
    return (
      <div className="space-y-4">
        <PageHeader title={app.t("nav.providers")} sub={app.t("providers.sub")} />
        <div className="card flex items-center gap-2 p-4 text-xs text-muted">
          <RefreshCw className="h-3.5 w-3.5 animate-spin" /> {app.t("providers.detecting")}
        </div>
      </div>
    );
  const fromPreset = (p: Preset): ProviderDraft => ({
    editing: false,
    id: c.providers.some((x) => x.id === p.id) ? "" : p.id,
    kind: p.kind,
    base_url: p.base_url ?? "",
    api_key: "",
    clear_key: false,
    max_concurrency: p.local ? 1 : 4,
    models: "",
    local: !!p.local,
    has_key: false,
    headers: [],
    caps: [],
    body: [],
    effort_field: "",
  });
  return (
    <div className="space-y-4">
      <PageHeader title={app.t("nav.providers")} sub={app.t("providers.sub")} />
      <Section
        title={app.t("providers.api")}
        right={
          !draft && (
            <button className="btn" onClick={() => setDraft(fromPreset(PRESETS.find((p) => p.id === "custom")!))}>
              <Plus className="h-3.5 w-3.5" /> {app.t("providers.add")}
            </button>
          )
        }
      >
        {draft && (
          <div className="mb-4 space-y-3">
            {!draft.editing && (
              <div className="flex flex-wrap gap-2">
                {vendors.length > 0 && (
                  <select
                    className="input w-auto text-xs"
                    value=""
                    onChange={(e) => {
                      const p = vendors.find((v) => v.id === e.target.value);
                      if (p) setDraft(fromPreset(p));
                    }}
                  >
                    <option value="">{app.t("providers.more")} ({vendors.length})</option>
                    {vendors.map((v) => (
                      <option key={v.id} value={v.id}>
                        {v.label}
                      </option>
                    ))}
                  </select>
                )}
                {PRESETS.map((p) => (
                  <button
                    key={p.id}
                    onClick={() => setDraft(fromPreset(p))}
                    className={cn(
                      "press rounded-lg border px-3 py-2 text-left text-xs",
                      (draft.local ? p.local : !p.local && draft.base_url === (p.base_url ?? "") && draft.kind === p.kind) ? "border-accent bg-blue-600/10" : "border-line hover:border-line-strong",
                    )}
                  >
                    <div className="flex items-center gap-1.5 font-semibold">
                      {p.local ? <Laptop className="h-3.5 w-3.5" /> : <KeyRound className="h-3.5 w-3.5" />}
                      {p.local ? app.t("providers.local") : p.label}
                    </div>
                  </button>
                ))}
              </div>
            )}
            {!draft.editing && <PresetNote draft={draft} vendors={vendors} />}
            <ProviderForm key={`${draft.id}-${draft.kind}-${draft.base_url}-${draft.local}`} draft={draft} onDone={() => (setDraft(null), load())} />
          </div>
        )}
        <table className="w-full text-xs">
          <thead>
            <tr className="label border-b border-line text-left">
              <th className="py-2">id</th>
              <th>kind</th>
              <th>base URL</th>
              <th>key</th>
              <th>{app.t("providers.concurrency")}</th>
              <th />
            </tr>
          </thead>
          <tbody className="divide-y divide-line">
            {c.providers.map((p) => (
              <Fragment key={p.id}>
                <tr>
                  <td className="py-2 font-medium">{p.id}</td>
                  <td className="text-muted">{p.kind}</td>
                  <td className="font-mono text-[11px] text-subtle">{p.base_url ?? "default"}</td>
                  <td>{p.has_key ? <CheckCircle2 className="h-4 w-4 text-emerald-500" /> : <span className="text-subtle">–</span>}</td>
                  <td className="font-mono">{p.max_concurrency}</td>
                  <td className="py-1.5">
                    <div className="flex items-center justify-end gap-1.5">
                      <ModelsReload
                        at={c.models_at?.[`api:${p.id}`]?.at}
                        error={c.models_at?.[`api:${p.id}`]?.error}
                        onReload={() => api.providerModels(p.id, true).then(() => load())}
                      />
                      <button className="btn" onClick={() => setModels(models === p.id ? null : p.id)}>
                        <ListTree className="h-3.5 w-3.5" /> {app.t("providers.models")}
                      </button>
                      <button
                        className="btn"
                        title={app.t("common.edit")}
                        aria-label={app.t("common.edit")}
                        onClick={() =>
                          setDraft({
                            editing: true,
                            id: p.id,
                            kind: p.kind,
                            base_url: p.base_url ?? "",
                            api_key: "",
                            clear_key: false,
                            max_concurrency: p.max_concurrency,
                            models: p.models.join(", "),
                            local: !p.has_key && !!p.base_url && /127\.0\.0\.1|localhost/.test(p.base_url),
                            has_key: p.has_key,
                            headers: (p.headers ?? []).map((h) => ({ name: h.name, value: h.value, secret: h.secret, stored: h.secret })),
                            caps: capsDraft(p.model_caps),
                            body: (p.body ?? []).map((f) => ({ path: f.path, value: f.value, model: f.model ?? "" })),
                            effort_field: p.effort_field ?? "",
                          })
                        }
                      >
                        <Pencil className="h-3.5 w-3.5" />
                      </button>
                      <button className="btn" title={app.t("providers.remove")} aria-label={app.t("providers.remove")} onClick={() => setDel(p.id)}>
                        <Trash2 className="h-3.5 w-3.5" />
                      </button>
                    </div>
                  </td>
                </tr>
                {models === p.id && (
                  <tr key={p.id + "-models"}>
                    <td colSpan={6} className="pb-3">
                      <ModelsList id={p.id} at={c.models_at?.[`api:${p.id}`]?.at} />
                    </td>
                  </tr>
                )}
              </Fragment>
            ))}
          </tbody>
        </table>
        {c.providers.length === 0 && !draft && <Empty>{app.t("providers.empty")}</Empty>}
      </Section>
      <Section
        title={app.t("providers.cli")}
        right={
          <button
            className="btn"
            disabled={scanning}
            onClick={() => {
              setScanning(true);
              load(true).finally(() => setScanning(false));
            }}
          >
            <RefreshCw className={cn("h-3.5 w-3.5", scanning && "animate-spin")} /> {app.t("providers.detect")}
          </button>
        }
      >
        <div className="mb-3 text-[11px] text-subtle">
          {app.t("providers.cliNote")} {c.clis_at && `${app.t("providers.clisFound")} ${ago(c.clis_at)}.`} {app.t("models.weekly")}
        </div>
        <table className="w-full text-xs">
          <thead>
            <tr className="label border-b border-line text-left">
              <th className="py-2">CLI</th>
              <th>{app.t("providers.version")}</th>
              <th>{app.t("providers.login")}</th>
              <th>{app.t("providers.usable")}</th>
              <th>{app.t("providers.models")}</th>
              <th>{app.t("providers.note")}</th>
            </tr>
          </thead>
          <tbody className="divide-y divide-line">
            {c.clis_pending && (
              <tr>
                <td colSpan={6} className="py-3 text-muted">
                  <RefreshCw className="mr-2 inline h-3.5 w-3.5 animate-spin" />
                  {app.t("providers.detecting")}
                </td>
              </tr>
            )}
            {c.clis.map((x) => (
              <tr key={x.kind}>
                <td className="py-2 font-medium">{x.kind.toLowerCase()}</td>
                <td>
                  <CliVersionCell version={x.version} info={updates.list[x.kind.toLowerCase()]} onDone={() => (updates.reload(), load(true))} />
                </td>
                <td>
                  <LoginState login={x.login} installed={!!x.path} />
                </td>
                <td>
                  {/* Usable, but a login not confirmed is a warning, not a tick. */}
                  {!x.compatible ? (
                    <XCircle className="h-4 w-4 text-subtle" />
                  ) : x.login === "logged_in" ? (
                    <CheckCircle2 className="h-4 w-4 text-emerald-500" />
                  ) : (
                    <span title={app.t("providers.loginUnsure")}>
                      <AlertTriangle className="h-4 w-4 text-amber-500" />
                    </span>
                  )}
                </td>
                <td>
                  {x.compatible ? (
                    <span className="inline-flex items-center gap-2">
                      <Mono className="text-muted">{c.cli_models?.[x.kind.toLowerCase()]?.length ?? 0}</Mono>
                      <ModelsReload
                        at={c.models_at?.[`cli:${x.kind.toLowerCase()}`]?.at}
                        error={c.models_at?.[`cli:${x.kind.toLowerCase()}`]?.error}
                        onReload={() => api.reloadCliModels(x.kind.toLowerCase()).then(() => load())}
                      />
                    </span>
                  ) : (
                    <span className="text-subtle">–</span>
                  )}
                </td>
                <td className="text-[11px] text-subtle">{x.path ? cliNote(x.note, app.t) : app.t("providers.notInstalled")}</td>
              </tr>
            ))}
          </tbody>
        </table>
        <div className="mt-3 text-[11px] text-subtle">
          {app.t("providers.fallback")}: {c.cli_fallback.provider ? <Mono>{c.cli_fallback.provider}/{c.cli_fallback.model}</Mono> : app.t("providers.cliBrain")}
        </div>
      </Section>
      <ConfirmModal
        open={!!del}
        title={`${app.t("providers.remove")} ${del ?? ""}`}
        note={app.t("providers.removeNote")}
        confirm={app.t("common.delete")}
        onClose={() => setDel(null)}
        onConfirm={() =>
          api
            .removeProvider(del!)
            .then(() => (setDel(null), load(), app.notify(app.t("common.done"))))
            .catch((e) => (setDel(null), app.notify(String(e))))
        }
      />
    </div>
  );
}
