// Documents in Settings (ADR-152): what reads them for the agents and the chats. Agenium reads PDF, Word, PowerPoint,
// Excel, EPUB and more itself; Microsoft's markitdown reads Office files and PDF tables better, and old Office files and
// Outlook messages, in the sandbox. Agenium installs its own copy on request (pinned, self-tested), looks for a newer
// version once a week and only says so; an update is self-tested and the copy in use stays when it fails.
import { CheckCircle2, Download, Loader2, RefreshCw, ShieldCheck, Trash2, XCircle } from "lucide-react";
import { useEffect, useState } from "react";
import { ConfirmModal } from "../components/motion";
import { Mono, Section, ago } from "../components/ui";
import { api, onMarkitdown, type MarkitdownStatus } from "../lib/api";
import { friendly } from "../lib/errors";
import { useApp } from "../lib/store";

export function MarkitdownSection() {
  const app = useApp();
  const [st, setSt] = useState<MarkitdownStatus | null>(null);
  const [busy, setBusy] = useState<string | null>(null);
  const [step, setStep] = useState("");
  const [removing, setRemoving] = useState(false);
  const load = () => api.markitdownStatus().then(setSt).catch(() => undefined);
  useEffect(() => {
    load();
    const off = onMarkitdown((e) => {
      if (e.step) setStep(e.step);
      if (e.update) load();
    });
    return () => {
      off.then((f) => f());
    };
  }, []);
  const run = (what: string, p: Promise<unknown>, done: string) => {
    setBusy(what);
    setStep("");
    p.then(() => app.notify(done))
      .catch((e) => app.notify(friendly(e, app.t)))
      .finally(() => (setBusy(null), setStep(""), load()));
  };
  if (!st) return null;
  const own = st.source === "agenium";
  return (
    <Section title={app.t("md.title")}>
      <div className="text-[11px] leading-relaxed text-subtle">{app.t("md.note")}</div>
      <div className="mt-2 space-y-1 text-xs">
        {st.source === "none" && <div className="text-muted">{app.t("md.none")}</div>}
        {st.source !== "none" && (
          <div className="flex flex-wrap items-center gap-2">
            <CheckCircle2 className="h-3.5 w-3.5 text-emerald-500" />
            <span>{app.t(own ? "md.agenium" : "md.path").replace("{v}", st.version ?? "?")}</span>
            {own && st.method && <span className="chip border-line-strong text-subtle">{st.method}</span>}
            {own && <span className="text-[11px] text-subtle">{st.size_mb} MB</span>}
          </div>
        )}
        {st.program && <Mono className="block truncate text-[11px] text-subtle">{st.program}</Mono>}
        {own && (
          <div className="text-[11px] text-subtle">
            {st.update ? (
              <span className="text-amber-600 dark:text-amber-400">{app.t("md.updateAvailable").replace("{v}", st.latest ?? "")}</span>
            ) : st.latest ? (
              app.t("md.upToDate")
            ) : null}
            {st.checked_at && ` · ${app.t("md.checked").replace("{t}", ago(st.checked_at))}`}
          </div>
        )}
        {st.tests.length > 0 && (
          <div className="flex flex-wrap items-center gap-1.5 text-[11px]">
            <span className="text-subtle">{app.t("md.selfTest")}:</span>
            {st.tests.map(([f, r]) => (
              <span key={f} className={r === "ok" ? "chip border-emerald-500/40 text-emerald-600 dark:text-emerald-400" : "chip border-rose-500/40 text-rose-600 dark:text-rose-400"} title={r}>
                {r === "ok" ? <CheckCircle2 className="h-3 w-3" /> : <XCircle className="h-3 w-3" />} {f}
              </span>
            ))}
            {st.tested_at && <span className="text-subtle">{ago(st.tested_at)}</span>}
          </div>
        )}
        {st.blocked && <div className="text-[11px] text-amber-600 dark:text-amber-400">{app.t("md.blocked")}</div>}
      </div>
      {busy && (
        <div className="mt-2 flex items-start gap-2 text-[11px] text-subtle">
          <Loader2 className="mt-0.5 h-3.5 w-3.5 shrink-0 animate-spin" /> <span className="min-w-0 break-words">{step || app.t("md.working")}</span>
        </div>
      )}
      <div className="mt-2 flex flex-wrap gap-2">
        {!own && (
          <button className="btn-primary" disabled={!!busy || st.busy} onClick={() => run("install", api.installMarkitdown(), app.t("md.installed"))}>
            <Download className="h-3.5 w-3.5" /> {app.t("md.install").replace("{v}", st.pinned)}
          </button>
        )}
        {own && st.update && (
          <button className="btn-primary" disabled={!!busy || st.busy} onClick={() => run("update", api.updateMarkitdown(), app.t("md.updated"))}>
            <Download className="h-3.5 w-3.5" /> {app.t("md.update").replace("{v}", st.latest ?? "")}
          </button>
        )}
        {own && (
          <button className="btn" disabled={!!busy || st.busy} onClick={() => run("check", api.checkMarkitdownUpdate(), app.t("md.checkedNow"))}>
            <RefreshCw className="h-3.5 w-3.5" /> {app.t("md.check")}
          </button>
        )}
        {st.source !== "none" && (
          <button className="btn" disabled={!!busy || st.busy} onClick={() => run("test", api.testMarkitdown(), app.t("md.tested"))}>
            <ShieldCheck className="h-3.5 w-3.5" /> {app.t("md.selfTest")}
          </button>
        )}
        {own && (
          <button className="btn-danger" disabled={!!busy || st.busy} onClick={() => setRemoving(true)}>
            <Trash2 className="h-3.5 w-3.5" /> {app.t("md.remove")}
          </button>
        )}
      </div>
      <ConfirmModal
        open={removing}
        title={app.t("md.remove")}
        note={app.t("md.removeNote").replace("{mb}", String(st.size_mb))}
        confirm={app.t("md.remove")}
        onClose={() => setRemoving(false)}
        onConfirm={() => (setRemoving(false), run("remove", api.removeMarkitdown(), app.t("md.removed")))}
      />
    </Section>
  );
}
