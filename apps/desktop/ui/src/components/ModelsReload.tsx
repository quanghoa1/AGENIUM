import { RefreshCw } from "lucide-react";
import { useState } from "react";
import { useApp } from "../lib/store";
import { ago, cn } from "./ui";

/** When a source's models were last listed (kept, listed again on their own once a week), whether the last try failed,
 *  and a button that lists them again now. `compact`: the button alone, the age in its tooltip. */
export function ModelsReload({ at, error, onReload, compact }: { at?: string | null; error?: string | null; onReload: () => Promise<unknown>; compact?: boolean }) {
  const app = useApp();
  const [busy, setBusy] = useState(false);
  const age = at ? `${app.t("models.listed")} ${ago(at)}` : app.t("models.never");
  const run = () => {
    setBusy(true);
    onReload()
      .catch((e) => app.notify(String(e)))
      .finally(() => setBusy(false));
  };
  return (
    <span className="inline-flex items-center gap-1.5 text-[11px] text-subtle">
      {!compact && <span title={at ?? ""}>{age}</span>}
      {error && (
        <span className="text-amber-600 dark:text-amber-400" title={error}>
          {app.t("models.lastFailed")}
        </span>
      )}
      <button type="button" className="btn px-2" disabled={busy} title={`${app.t("models.reload")} · ${age}`} aria-label={app.t("models.reload")} onClick={run}>
        <RefreshCw className={cn("h-3.5 w-3.5", busy && "animate-spin")} />
      </button>
    </span>
  );
}
