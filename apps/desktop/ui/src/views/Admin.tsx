import { Copy, Loader2, ShieldAlert, ShieldCheck } from "lucide-react";
import { ProjectPicker } from "../components/ProjectPicker";
import { useEffect, useState } from "react";
import { Logo } from "../components/Logo";
import { ConfirmModal } from "../components/motion";
import { Empty, Mono, PageHeader, Section, cn } from "../components/ui";
import { api } from "../lib/api";
import { useApp } from "../lib/store";
import type { Key } from "../lib/i18n";
import type { AuditEntry, CommandPolicy } from "../lib/types";
import { JudgeSection } from "./JudgeSettings";

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
        <ProjectPicker />
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
                    <button className="font-mono text-cyan-600 hover:underline dark:text-cyan-400" onClick={() => app.openTicket(e.ticket!)}>
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

export function SettingsView() {
  const app = useApp();
  return (
    <div className="space-y-4">
      <PageHeader title={app.t("nav.settings")} />
      <div className="grid gap-4 md:grid-cols-2">
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
