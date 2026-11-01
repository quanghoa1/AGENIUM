// Messages waiting for the Lead (ADR-149): shown above the input until its next turn reads them; each can be changed
// or taken back until then. They wait while it works, or after its turn failed (they are put back): then they go with
// the next message, or now with "Send again".
import { Clock, Pencil, RotateCw, Trash2 } from "lucide-react";
import { useState } from "react";
import { api } from "../lib/api";
import { friendly } from "../lib/errors";
import { useApp } from "../lib/store";
import type { QueuedMessage } from "../lib/types";
import { DocChip, splitMaterial } from "./Attachments";

export function LeadQueue({
  project,
  agentName,
  queue,
  working,
  reload,
  answered,
}: {
  project: string;
  agentName: string;
  queue: QueuedMessage[];
  /** The Lead is running now (else the messages are held after a failed turn). */
  working: boolean;
  reload: () => void;
  /** After "Send again": the Lead's turn, to show its answer or why it failed. */
  answered: (r: Promise<{ kind: string }>) => void;
}) {
  const app = useApp();
  const [editing, setEditing] = useState<number | null>(null);
  const [draft, setDraft] = useState("");
  const [sending, setSending] = useState(false);
  if (queue.length === 0) return null;
  const done = (p: Promise<unknown>) =>
    p
      .catch((e) => app.notify(friendly(e, app.t)))
      .finally(() => {
        setEditing(null);
        reload();
      });
  // Only the typed part is edited; the attached documents stay with the message.
  const save = (m: QueuedMessage) => {
    const at = m.text.indexOf("<attachments note=");
    const kept = at >= 0 ? `\n\n${m.text.slice(at)}` : "";
    done(api.editLeadMessage(project, m.id, draft.trim() + kept));
  };
  return (
    <div className="border-t border-amber-500/30 bg-amber-500/5 px-4 py-2.5">
      <div className="mb-1.5 flex items-center gap-1.5 text-[11px] font-semibold text-amber-700 dark:text-amber-300">
        <Clock className="h-3.5 w-3.5 shrink-0" />
        <span className="flex-1">
          {app
            .t(working ? "chat.queueTitle" : "chat.queueHeld")
            .replaceAll("{name}", agentName)
            .replace("{n}", String(queue.length))}
        </span>
        {!working && (
          <button
            className="btn"
            disabled={sending}
            onClick={() => {
              setSending(true);
              answered(api.deliverLeadQueue(project).finally(() => (setSending(false), reload())));
            }}
          >
            <RotateCw className={sending ? "h-3.5 w-3.5 animate-spin" : "h-3.5 w-3.5"} /> {app.t("chat.queueSend")}
          </button>
        )}
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
                    if (draft.trim()) save(m);
                  }
                }}
              />
              <button className="btn-primary" disabled={!draft.trim()} onClick={() => save(m)}>
                {app.t("common.save")}
              </button>
              <button className="btn" onClick={() => setEditing(null)}>
                {app.t("common.cancel")}
              </button>
            </div>
          ) : (
            <div key={m.id} className="flex items-start gap-2 rounded-lg border border-line bg-elevated/60 px-3 py-2">
              <Queued text={m.text} />
              {m.id > 0 && (
                <>
                  <button
                    className="btn"
                    title={app.t("chat.queueEdit")}
                    aria-label={app.t("chat.queueEdit")}
                    onClick={() => (setEditing(m.id), setDraft(splitMaterial(m.text).text))}
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

/** A waiting message as the conversation shows it: the typed text, its documents as chips. */
function Queued({ text }: { text: string }) {
  const m = splitMaterial(text);
  return (
    <div className="min-w-0 flex-1 text-[13px]">
      <div className="whitespace-pre-wrap">{m.text}</div>
      {m.docs.length > 0 && (
        <div className="mt-1.5 flex flex-wrap gap-1">
          {m.docs.map((d, i) => (
            <DocChip key={i} name={d.name} size={d.size} />
          ))}
        </div>
      )}
    </div>
  );
}
