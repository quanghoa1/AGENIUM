// The app's footer: each vendor's subscription usage (5-hour, weekly, monthly windows: % used and the time to the
// reset), amber from 60 % and red from 80 %. A click opens the vendor's details; the refresh reads Codex and OpenCode
// Go now; Claude reports during its own runs, or reads now on the Claude row's button (one small Haiku turn).
import { AlertTriangle, RefreshCw } from "lucide-react";
import { useEffect, useRef, useState } from "react";
import { api, onUsage } from "../lib/api";
import type { Key } from "../lib/i18n";
import { useApp } from "../lib/store";
import type { VendorUsage } from "../lib/types";
import { ago, cn } from "./ui";

const NAMES: Record<string, string> = { claude: "Claude", codex: "Codex", agy: "Antigravity" };

function vendorName(id: string) {
  if (id.startsWith("opencode-go:")) {
    const p = id.slice("opencode-go:".length);
    return p === "opencode-go" ? "OpenCode Go" : `OpenCode Go · ${p}`;
  }
  return NAMES[id] ?? id;
}

const tone = (p: number) => (p >= 80 ? "rose" : p >= 60 ? "amber" : "emerald");
const TEXT = { rose: "text-rose-600 dark:text-rose-400", amber: "text-amber-600 dark:text-amber-400", emerald: "text-fg" };
const BAR = { rose: "bg-rose-500", amber: "bg-amber-500", emerald: "bg-emerald-500" };

/** "2h13m", "3d4h" until `iso`. */
function until(iso?: string | null) {
  if (!iso) return "";
  const m = Math.max(0, Math.round((new Date(iso).getTime() - Date.now()) / 60_000));
  if (m < 60) return `${m}m`;
  if (m < 24 * 60) return `${Math.floor(m / 60)}h${String(m % 60).padStart(2, "0")}m`;
  return `${Math.floor(m / 1440)}d${Math.floor((m % 1440) / 60)}h`;
}

function Bar({ p, className }: { p: number; className?: string }) {
  return (
    <span className={cn("relative inline-block h-1.5 overflow-hidden rounded-full bg-line", className)}>
      <span className={cn("absolute inset-y-0 left-0 rounded-full", BAR[tone(p)])} style={{ width: `${Math.min(100, Math.max(2, p))}%` }} />
    </span>
  );
}

function Segment({ u, open, onOpen, onRefresh, onReadNow, busy }: { u: VendorUsage; open: boolean; onOpen: () => void; onRefresh: () => void; onReadNow?: () => void; busy: boolean }) {
  const app = useApp();
  const t = app.t;
  const top = u.windows.reduce((m, w) => Math.max(m, w.used_percent), 0);
  const label = (w: string) => t(`footer.window.${w}` as Key);
  return (
    <div className="relative">
      <button className={cn("flex h-7 items-center gap-1.5 rounded px-2 text-[11px] hover:bg-elevated", open && "bg-elevated")} onClick={onOpen}>
        <span className="font-semibold">{vendorName(u.vendor)}</span>
        {u.windows.length > 0 ? (
          <>
            <Bar p={top} className="w-12" />
            {u.windows.map((w) => (
              <span key={w.window} className={cn("font-mono", TEXT[tone(w.used_percent)])}>
                <span className="text-subtle">{label(w.window)}</span> {Math.round(w.used_percent)}%
                {w.window === "5h" && w.resets_at ? <span className="text-subtle"> · {until(w.resets_at)}</span> : null}
              </span>
            ))}
            {u.error && <AlertTriangle className="h-3 w-3 text-amber-500" />}
          </>
        ) : u.error ? (
          <span className="flex items-center gap-1 text-amber-600 dark:text-amber-400">
            <AlertTriangle className="h-3 w-3" /> {u.error === "not signed in" ? t("footer.signedOut") : t("footer.unavailable")}
          </span>
        ) : (
          <span className="font-mono text-subtle">--</span>
        )}
      </button>
      {open && (
        <div className="absolute bottom-full right-0 z-30 mb-1.5 w-72 rounded-xl border border-line-strong bg-surface p-3 text-xs shadow-xl">
          <div className="flex items-center gap-2">
            <span className="font-semibold">{vendorName(u.vendor)}</span>
            {u.plan && <span className="chip border-line-strong px-1.5 py-0 text-[10px] uppercase text-muted">{u.plan}</span>}
            {!onReadNow && (
              <button className="ml-auto rounded p-1 text-subtle hover:bg-elevated hover:text-fg" title={t("footer.refresh")} onClick={onRefresh} disabled={busy}>
                <RefreshCw className={cn("h-3.5 w-3.5", busy && "animate-spin")} />
              </button>
            )}
          </div>
          <div className="mt-0.5 text-[10px] text-subtle">{u.updated_at ? `${t("footer.updated")} ${ago(u.updated_at)}` : t("footer.never")}</div>
          <div className="mt-2.5 space-y-2">
            {u.windows.map((w) => (
              <div key={w.window}>
                <div className="flex items-center justify-between">
                  <span className="text-muted">{t(`footer.windowLong.${w.window}` as Key)}</span>
                  <span className={cn("font-mono", TEXT[tone(w.used_percent)])}>{Math.round(w.used_percent)}%</span>
                </div>
                <Bar p={w.used_percent} className="mt-1 w-full" />
                {w.resets_at && (
                  <div className="mt-0.5 text-[10px] text-subtle">
                    {t("footer.resetsIn")} {until(w.resets_at)} · {new Date(w.resets_at).toLocaleString()}
                  </div>
                )}
              </div>
            ))}
          </div>
          {u.note && <div className="mt-2.5 rounded-lg bg-elevated px-2 py-1.5 text-[11px] text-muted">{u.note}</div>}
          {onReadNow && (
            <button className="btn mt-2.5 w-full justify-center" onClick={onReadNow} disabled={busy}>
              <RefreshCw className={cn("h-3.5 w-3.5", busy && "animate-spin")} /> {t("footer.readClaude")}
            </button>
          )}
          {u.error && (
            <div className="mt-2 flex items-start gap-1.5 text-[11px] text-amber-600 dark:text-amber-400">
              <AlertTriangle className="mt-0.5 h-3 w-3 shrink-0" /> {u.windows.length > 0 ? `${t("footer.stale")} ` : ""}
              {u.error}
            </div>
          )}
        </div>
      )}
    </div>
  );
}

export function UsageFooter() {
  const app = useApp();
  const [rows, setRows] = useState<VendorUsage[]>([]);
  const [open, setOpen] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const [, setNow] = useState(0);
  const ref = useRef<HTMLDivElement>(null);
  useEffect(() => {
    const load = () => api.vendorUsage().then(setRows).catch(() => undefined);
    load();
    // Readings change every 15 minutes or after a Claude run; the countdowns move every half minute.
    const a = setInterval(load, 60_000);
    const b = setInterval(() => setNow((n) => n + 1), 30_000);
    const off = onUsage(load);
    return () => (clearInterval(a), clearInterval(b), void off.then((f) => f()));
  }, []);
  useEffect(() => {
    if (!open) return;
    const close = (e: MouseEvent) => !ref.current?.contains(e.target as Node) && setOpen(null);
    window.addEventListener("mousedown", close);
    return () => window.removeEventListener("mousedown", close);
  }, [open]);
  const refresh = (read = api.refreshVendorUsage) => {
    setBusy(true);
    read()
      .then(setRows)
      .catch((e) => app.notify(String(e)))
      .finally(() => setBusy(false));
  };
  if (rows.length === 0) return null;
  return (
    <footer ref={ref} className="flex h-8 shrink-0 items-center justify-end gap-1 border-t border-line bg-surface px-3">
      <span className="label mr-1">{app.t("footer.usage")}</span>
      {rows.map((u) => (
        <Segment
          key={u.vendor}
          u={u}
          open={open === u.vendor}
          onOpen={() => setOpen(open === u.vendor ? null : u.vendor)}
          onRefresh={() => refresh()}
          onReadNow={u.vendor === "claude" ? () => refresh(api.readClaudeUsage) : undefined}
          busy={busy}
        />
      ))}
      <button className="ml-1 rounded p-1 text-subtle hover:bg-elevated hover:text-fg" title={app.t("footer.refresh")} onClick={() => refresh()} disabled={busy}>
        <RefreshCw className={cn("h-3.5 w-3.5", busy && "animate-spin")} />
      </button>
    </footer>
  );
}
