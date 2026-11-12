import { ShieldAlert, ShieldCheck } from "lucide-react";
import { useEffect, useState } from "react";
import { ConfirmModal } from "../components/motion";
import { Section, cn } from "../components/ui";
import { api } from "../lib/api";
import { useApp } from "../lib/store";
import type { CommandPolicy } from "../lib/types";

const MODES = ["sandboxed", "on-request", "never"] as const;

/** What agents' commands do right now: the mode as this computer can carry it out (ADR-64, ADR-117, ADR-160). */
function now(p: CommandPolicy): "sandboxed" | "ask" | "refused" | "unsandboxed" | "off" {
  if (p.approval === "on-request") return "ask";
  if (p.off) return "off";
  if (p.backend) return "sandboxed";
  if (p.unsandboxed) return "unsandboxed";
  return p.approval === "never" ? "refused" : "ask";
}

/** How agent shell commands are approved. Every mode is described at once, and what it does on this computer, so
 * the choice is understood before it is made (a mode's description used to show only once it was chosen). */
export function CommandPolicySection() {
  const app = useApp();
  const [p, setP] = useState<CommandPolicy | null>(null);
  const [ask, setAsk] = useState(false);
  const [askOff, setAskOff] = useState(false);
  useEffect(() => {
    api.commandPolicy().then(setP).catch(() => setP(null));
  }, []);
  const setUnsandboxed = (on: boolean) => api.setUnsandboxed(on).then(setP).catch((e) => app.notify(String(e)));
  const setOff = (off: boolean) => api.setSandboxOff(off).then(setP).catch((e) => app.notify(String(e)));
  const setAuto = (on: boolean) => api.setAutoEscalate(on).then(setP).catch((e) => app.notify(String(e)));
  if (!p) return null;
  const state = now(p);
  return (
    <Section title={app.t("settings.commands")}>
      <div className="text-[11px] text-subtle">{app.t("settings.commands.intro")}</div>
      <div role="radiogroup" aria-label={app.t("settings.commands")} className="mt-2 space-y-1.5">
        {MODES.map((m) => (
          <button
            key={m}
            type="button"
            role="radio"
            aria-checked={p.approval === m}
            className={cn(
              "press flex w-full items-start gap-2.5 rounded-lg border p-2.5 text-left",
              p.approval === m ? "border-accent/50 bg-blue-600/10" : "border-line hover:border-line-strong",
            )}
            onClick={() => p.approval !== m && api.setCommandPolicy(m).then(setP).catch((e) => app.notify(String(e)))}
          >
            <span
              className={cn(
                "mt-0.5 flex h-3.5 w-3.5 shrink-0 items-center justify-center rounded-full border",
                p.approval === m ? "border-accent" : "border-line-strong",
              )}
            >
              {p.approval === m && <span className="h-1.5 w-1.5 rounded-full bg-accent" />}
            </span>
            <span className="min-w-0">
              <span className="block text-xs font-medium">{app.t(`settings.commands.${m}`)}</span>
              <span className="mt-0.5 block text-[11px] text-subtle">{app.t(`settings.commands.${m}.hint`)}</span>
            </span>
          </button>
        ))}
      </div>
      <div className="mt-2 text-[11px] text-subtle">{app.t("settings.commands.sandboxWhat")}</div>
      <div
        className={cn(
          "mt-2 flex items-start gap-1.5 text-[11px]",
          p.backend ? "text-emerald-600 dark:text-emerald-300" : "text-amber-600 dark:text-amber-300",
        )}
      >
        {p.backend ? <ShieldCheck className="mt-px h-3.5 w-3.5 shrink-0" /> : <ShieldAlert className="mt-px h-3.5 w-3.5 shrink-0" />}
        <span>
          {app.t(p.off ? "settings.sandboxTurnedOff" : p.backend ? "settings.sandboxOn" : "settings.sandboxOff")}{" "}
          {app.t(`settings.now.${state}`)}
        </span>
      </div>
      {!p.backend && !p.off && (
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
      {p.backend && p.approval !== "never" && (
        <label className="mt-3 flex items-start gap-2 rounded-lg border border-line p-2.5 text-xs">
          <input
            type="checkbox"
            className="mt-0.5"
            data-testid="auto-escalate"
            checked={p.auto_escalate}
            onChange={(e) => setAuto(e.target.checked)}
          />
          <span>
            <span className="font-medium">{app.t("settings.autoEscalate")}</span>
            <span className="mt-0.5 block text-[11px] text-subtle">{app.t("settings.autoEscalate.hint")}</span>
          </span>
        </label>
      )}
      {(p.os_backend || p.off) && (
        <label className="mt-3 flex items-start gap-2 rounded-lg border border-red-500/40 bg-red-500/5 p-2.5 text-xs">
          <input
            type="checkbox"
            className="mt-0.5"
            data-testid="sandbox-off"
            checked={p.off}
            onChange={(e) => (e.target.checked ? setAskOff(true) : setOff(false))}
          />
          <span>
            <span className="font-medium">{app.t("settings.sandboxOffSwitch")}</span>
            <span className="mt-0.5 block text-[11px] text-subtle">{app.t("settings.sandboxOffSwitch.hint")}</span>
          </span>
        </label>
      )}
      <div className="mt-2 text-[11px] text-subtle">{app.t("settings.commands.always")}</div>
      <ConfirmModal
        open={ask}
        title={app.t("settings.unsandboxed")}
        note={app.t("settings.unsandboxed.warn")}
        confirm={app.t("settings.unsandboxed.ok")}
        icon={<ShieldAlert className="h-3.5 w-3.5" />}
        onClose={() => setAsk(false)}
        onConfirm={() => (setAsk(false), setUnsandboxed(true))}
      />
      <ConfirmModal
        open={askOff}
        title={app.t("settings.sandboxOffSwitch")}
        note={app.t("settings.sandboxOffSwitch.warn")}
        confirm={app.t("settings.sandboxOffSwitch.ok")}
        icon={<ShieldAlert className="h-3.5 w-3.5" />}
        onClose={() => setAskOff(false)}
        onConfirm={() => (setAskOff(false), setOff(true))}
      />
    </Section>
  );
}
