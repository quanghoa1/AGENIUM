import { ShieldCheck } from "lucide-react";
import { useEffect, useState } from "react";
import { api } from "../lib/api";
import { useApp } from "../lib/store";
import type { CommandPolicy } from "../lib/types";

/** "Approve commands the sandbox blocks by myself" (ADR-161), on the page those commands wait on (Needs you): on, a
 *  blocked command runs outside the sandbox at once instead of waiting there. Shown only where it applies (a working
 *  sandbox and a mode that may ask). */
export function AutoApprove() {
  const app = useApp();
  const [p, setP] = useState<CommandPolicy | null>(null);
  useEffect(() => {
    api
      .commandPolicy()
      .then(setP)
      .catch(() => setP(null));
  }, []);
  if (!p || !p.backend || p.approval === "never") return null;
  return (
    <label className="card flex items-start gap-2 p-3 text-xs">
      <input
        type="checkbox"
        className="mt-0.5"
        data-testid="auto-escalate"
        checked={p.auto_escalate}
        onChange={(e) =>
          api
            .setAutoEscalate(e.target.checked)
            .then(setP)
            .catch((err) => app.notify(String(err)))
        }
      />
      <ShieldCheck className="h-4 w-4 shrink-0 text-amber-500" />
      <span>
        <span className="font-medium">{app.t("settings.autoEscalate")}</span>
        <span className="mt-0.5 block text-[11px] text-subtle">
          {app.t("settings.autoEscalate.hint")}
        </span>
      </span>
    </label>
  );
}
