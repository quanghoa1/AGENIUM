// Messages waiting for the busy Lead (ADR-149): shown above the input until its next turn reads them; each can be
// changed or taken back until then.
import { Clock, Pencil, Trash2 } from "lucide-react";
import { useState } from "react";
import { api } from "../lib/api";
import { friendly } from "../lib/errors";
import { useApp } from "../lib/store";
import type { QueuedMessage } from "../lib/types";

export function LeadQueue({
  project,
  agentName,
  queue,
  reload,
}: {
  project: string;
  agentName: string;
  queue: QueuedMessage[];
  reload: () => void;
}) {
  const app = useApp();
  const [editing, setEditing] = useState<number | null>(null);
  const [draft, setDraft] = useState("");
  if (queue.length === 0) return null;
  const done = (p: Promise<unknown>) =>
    p
      .catch((e) => app.notify(friendly(e, app.t)))
      .finally(() => {
        setEditing(null);
        reload();
      });
  return (
    <div className="border-t border-amber-500/30 bg-amber-500/5 px-4 py-2.5">
      <div className="mb-1.5 flex items-center gap-1.5 text-[11px] font-semibold text-amber-700 dark:text-amber-300">
        <Clock className="h-3.5 w-3.5" />
        {app.t("chat.queueTitle").replace("{name}", agentName).replace("{n}", String(queue.length))}
      </div>
      <div className="space-y-1.5">
        {queue.map((m) =>
          editing === m.id ? (
            <div key={m.id} className="flex items-start gap-2">
              <textarea
                className="input min-h-[40px] flex-1 resize-y text-[13px]"
                rows={2}
                value={draft}
                autoFocus
                onChange={(e) => setDraft(e.target.value)}
                onKeyDown={(e) => {
                  if (e.key === "Escape") setEditing(null);
                  if (e.key === "Enter" && !e.shiftKey && !e.nativeEvent.isComposing) {
                    e.preventDefault();
                    if (draft.trim()) done(api.editLeadMessage(project, m.id, draft.trim()));
                  }
                }}
              />
              <button className="btn-primary" disabled={!draft.trim()} onClick={() => done(api.editLeadMessage(project, m.id, draft.trim()))}>
                {app.t("common.save")}
              </button>
              <button className="btn" onClick={() => setEditing(null)}>
                {app.t("common.cancel")}
              </button>
            </div>
          ) : (
            <div key={m.id} className="flex items-start gap-2 rounded-lg border border-line bg-elevated/60 px-3 py-2">
              <div className="min-w-0 flex-1 whitespace-pre-wrap text-[13px]">{m.text}</div>
              {m.id > 0 && (
                <>
                  <button
                    className="btn"
                    title={app.t("chat.queueEdit")}
                    aria-label={app.t("chat.queueEdit")}
                    onClick={() => (setEditing(m.id), setDraft(m.text))}
                  >
                    <Pencil className="h-3.5 w-3.5" />
                  </button>
                  <button
                    className="btn"
                    title={app.t("chat.queueDrop")}
                    aria-label={app.t("chat.queueDrop")}
                    onClick={() => done(api.dropLeadMessage(project, m.id))}
                  >
                    <Trash2 className="h-3.5 w-3.5" />
                  </button>
                </>
              )}
            </div>
          ),
        )}
      </div>
    </div>
  );
}
