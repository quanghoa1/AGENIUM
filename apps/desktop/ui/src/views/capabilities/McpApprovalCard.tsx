import { ShieldQuestion } from "lucide-react";
import { cn } from "../../components/ui";
import { api } from "../../lib/api";
import { useApp } from "../../lib/store";
import type { McpApproval, McpApprovalSettings } from "../../lib/types";

const CHOICES = ["destructive", "writes", "never"] as const;

/** Which MCP tool calls wait for the user in Needs you (ADR-143), for every server; a server's row sets its own. Each
 *  choice is described at once, as the command modes are. */
export function McpApprovalCard({ settings, onChange }: { settings: McpApprovalSettings; onChange: (s: McpApprovalSettings) => void }) {
  const app = useApp();
  const pick = (a: McpApproval) =>
    api
      .setMcpApproval(null, a)
      .then(onChange)
      .catch((e) => app.notify(String(e)));
  return (
    <div className="card space-y-3 p-4">
      <div className="flex items-center gap-2 text-xs font-semibold">
        <ShieldQuestion className="h-3.5 w-3.5" /> {app.t("library.approval")}
      </div>
      <div className="text-[11px] text-subtle">{app.t("library.approval.intro")}</div>
      <div role="radiogroup" aria-label={app.t("library.approval")} className="space-y-1.5">
        {CHOICES.map((a) => (
          <button
            key={a}
            type="button"
            role="radio"
            aria-checked={settings.default === a}
            className={cn(
              "press flex w-full items-start gap-2.5 rounded-lg border p-2.5 text-left",
              settings.default === a ? "border-accent/50 bg-blue-600/10" : "border-line hover:border-line-strong",
            )}
            onClick={() => settings.default !== a && pick(a)}
          >
            <span className={cn("mt-0.5 flex h-3.5 w-3.5 shrink-0 items-center justify-center rounded-full border", settings.default === a ? "border-accent" : "border-line-strong")}>
              {settings.default === a && <span className="h-1.5 w-1.5 rounded-full bg-accent" />}
            </span>
            <span className="min-w-0">
              <span className="block text-xs font-medium">{app.t(`library.approval.${a}`)}</span>
              <span className="mt-0.5 block text-[11px] text-subtle">{app.t(`library.approval.${a}.hint`)}</span>
            </span>
          </button>
        ))}
      </div>
    </div>
  );
}
