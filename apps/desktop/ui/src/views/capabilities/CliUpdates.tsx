// Updating an agent CLI from Providers & CLIs (ADR-153): the version installed, the newest one and the one Agenium
// verified; the CLI's own updater run on request (asked first: it is the user's install), never while one of its runs
// is in progress; a short question through the CLI after; when that fails, the version it had offered back.
import { AlertTriangle, ArrowDownToLine, CheckCircle2, Loader2, RotateCcw, XCircle } from "lucide-react";
import { useEffect, useState } from "react";
import { ConfirmModal } from "../../components/motion";
import { Mono, cn } from "../../components/ui";
import { api, type CliUpdateInfo, type CliUpdated } from "../../lib/api";
import { friendly } from "../../lib/errors";
import { useApp } from "../../lib/store";

/** Whether version `a` is newer than `b` (dotted numbers, as the CLIs print them). */
function newerThan(a?: string | null, b?: string | null): boolean {
  const parts = (v: string) => (v.match(/\d+(\.\d+)+/)?.[0] ?? "").split(".").map(Number);
  if (!a || !b) return false;
  const [x, y] = [parts(a), parts(b)];
  for (let i = 0; i < Math.max(x.length, y.length); i++) {
    if ((x[i] ?? 0) !== (y[i] ?? 0)) return (x[i] ?? 0) > (y[i] ?? 0);
  }
  return false;
}

/** The update state of every installed CLI, by program name; read again with `reload`. */
export function useCliUpdates(tick: number) {
  const [list, setList] = useState<Record<string, CliUpdateInfo>>({});
  const reload = () =>
    api
      .cliUpdates()
      .then((l) => setList(Object.fromEntries(l.map((x) => [x.program, x]))))
      .catch(() => undefined);
  useEffect(() => {
    reload();
  }, [tick]);
  return { list, reload };
}

export function CliVersionCell({ version, info, onDone }: { version?: string | null; info?: CliUpdateInfo; onDone: () => void }) {
  const app = useApp();
  const [asking, setAsking] = useState<string | null | undefined>(undefined);
  const [busy, setBusy] = useState(false);
  const [result, setResult] = useState<CliUpdated | null>(null);
  const run = (target: string | null) => {
    if (!info) return;
    setBusy(true);
    setResult(null);
    api
      .updateCli(info.program, target)
      .then((r) => {
        setResult(r);
        app.notify(
          r.check_ok
            ? app.t("cli.updated").replace("{cli}", info.program).replace("{v}", r.after ?? "?")
            : app.t("cli.checkFailed").replace("{cli}", info.program),
        );
        onDone();
      })
      .catch((e) => app.notify(friendly(e, app.t)))
      .finally(() => setBusy(false));
  };
  if (!info) return <Mono>{version ?? "–"}</Mono>;
  const target = asking === undefined ? null : asking;
  // Said beside the version it is about: the one installed, or the one offered.
  const installedBeyond = newerThan(info.installed, info.verified);
  const latestBeyond = info.update && newerThan(info.latest, info.verified);
  return (
    <div className="space-y-0.5">
      <div className="flex flex-wrap items-center gap-1.5">
        <Mono>{info.installed ?? version ?? "–"}</Mono>
        {installedBeyond && info.verified && (
          <span className="chip border-amber-500/40 text-amber-600 dark:text-amber-400" title={app.t("cli.beyondVerifiedHint")}>
            <AlertTriangle className="h-3 w-3" /> {app.t("cli.verified").replace("{v}", info.verified)}
          </span>
        )}
      </div>
      <div className="flex flex-wrap items-center gap-1.5 text-[11px]">
        {info.update ? (
          <span className="text-amber-600 dark:text-amber-400">
            {app.t("cli.available").replace("{v}", info.latest ?? "")}
            {latestBeyond && info.verified && (
              <span className="ml-1 text-subtle" title={app.t("cli.beyondVerifiedHint")}>
                ({app.t("cli.unverified").replace("{v}", info.verified)})
              </span>
            )}
          </span>
        ) : info.latest ? (
          <span className="text-subtle">{app.t("cli.upToDate")}</span>
        ) : (
          <span className="text-subtle" title={info.latest_error ?? ""}>
            {app.t("cli.cantCheck")}
          </span>
        )}
        <button
          className={cn("btn h-6 px-1.5 text-[11px]", info.update && "border-amber-500/50")}
          disabled={busy || info.busy > 0}
          title={info.busy > 0 ? app.t("cli.busy").replace("{n}", String(info.busy)) : app.t("cli.updateHint")}
          onClick={() => setAsking(null)}
        >
          {busy ? <Loader2 className="h-3 w-3 animate-spin" /> : <ArrowDownToLine className="h-3 w-3" />} {app.t("cli.update")}
        </button>
      </div>
      {result && (
        <div className={cn("text-[11px]", result.check_ok ? "text-emerald-600 dark:text-emerald-400" : "text-rose-600 dark:text-rose-400")} title={[result.check, result.output].filter(Boolean).join(" | ")}>
          {result.check_ok ? <CheckCircle2 className="mr-1 inline h-3 w-3" /> : <XCircle className="mr-1 inline h-3 w-3" />}
          {result.before ?? "?"} → {result.after ?? "?"} · {result.check_ok ? app.t("cli.checkOk") : `${app.t("cli.checkBad")}: ${result.check}`}
          {result.rollback && (
            <button className="btn ml-1.5 h-6 px-1.5 text-[11px]" disabled={busy} onClick={() => setAsking(result.rollback!)}>
              <RotateCcw className="h-3 w-3" /> {app.t("cli.rollback").replace("{v}", result.rollback)}
            </button>
          )}
        </div>
      )}
      <ConfirmModal
        open={asking !== undefined}
        title={target ? app.t("cli.rollback").replace("{v}", target) : `${app.t("cli.update")} ${info.program}`}
        note={app
          .t(target ? "cli.confirmRollback" : info.beyond_verified || (info.update && info.verified) ? "cli.confirmBeyond" : "cli.confirm")
          .replace("{cli}", info.program)
          .replace("{from}", info.installed ?? "?")
          .replace("{to}", target ?? info.latest ?? app.t("cli.newest"))
          .replace("{verified}", info.verified ?? "?")}
        confirm={target ? app.t("cli.rollback").replace("{v}", target) : app.t("cli.update")}
        onClose={() => setAsking(undefined)}
        onConfirm={() => {
          setAsking(undefined);
          run(target);
        }}
      />
    </div>
  );
}
