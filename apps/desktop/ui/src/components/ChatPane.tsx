// The conversation with a project's Lead (or Solo agent): history, live streaming of the turn, the input. Used by
// the chat page and by the Live tab (where the input can be pre-filled and focused from elsewhere).
import { Loader2, Send } from "lucide-react";
import { useEffect, useRef, useState, type ReactNode } from "react";
import { api } from "../lib/api";
import { useApp } from "../lib/store";
import type { ChatLine } from "../lib/types";
import { Markdown } from "./Markdown";
import { ListItem } from "./motion";
import { Avatar, Empty, cn } from "./ui";

export interface Prefill {
  text: string;
  /** Changes on every request so the same text can be asked for twice. */
  nonce: number;
}

export function ChatPane({
  project,
  agentName,
  enabled = true,
  prefill,
  after,
  className,
}: {
  project: string;
  agentName: string;
  enabled?: boolean;
  prefill?: Prefill | null;
  /** Rendered after the conversation (the Lead's delegation cards in the Live tab). */
  after?: ReactNode;
  className?: string;
}) {
  const app = useApp();
  const [lines, setLines] = useState<ChatLine[]>([]);
  const [text, setText] = useState("");
  const [busy, setBusy] = useState(false);
  const end = useRef<HTMLDivElement>(null);
  const input = useRef<HTMLTextAreaElement>(null);
  useEffect(() => {
    setLines([]);
    api.chatHistory(project).then(setLines);
  }, [project]);
  useEffect(() => {
    if (!prefill) return;
    setText(prefill.text);
    // Focus after the pane is on screen, caret at the end.
    window.setTimeout(() => {
      const el = input.current;
      if (!el) return;
      el.focus();
      el.setSelectionRange(prefill.text.length, prefill.text.length);
    }, 60);
  }, [prefill]);
  useEffect(() => {
    // Scroll the conversation only (scrollIntoView would also scroll the page or pane around it).
    const el = end.current?.parentElement;
    el?.scrollTo({ top: el.scrollHeight, behavior: "smooth" });
  }, [lines, busy, app.live]);

  // While a turn runs, show the text of the newest live run that is still going.
  const streaming = busy
    ? Object.values(app.live)
        .filter((l) => !l.done)
        .sort((a, b) => a.at - b.at)
        .map((l) => l.text)
        .filter(Boolean)
        .slice(-1)[0]
    : undefined;

  const send = () => {
    if (!text.trim() || !enabled) return;
    const msg = text.trim();
    setText("");
    setLines((l) => [...l, { role: "user", text: msg }]);
    setBusy(true);
    api
      .chat(project, msg)
      .then((r) => setLines((l) => [...l, { role: "assistant", text: r.text || `(${r.outcome})` }]))
      .catch((e) => setLines((l) => [...l, { role: "system", text: String(e) }]))
      .finally(() => (setBusy(false), app.refresh()));
  };

  return (
    <div className={cn("flex min-h-0 flex-1 flex-col", className)}>
      <div className="min-h-0 flex-1 space-y-4 overflow-y-auto p-5">
        {lines.map((l, i) => (
          <ListItem key={i} id={String(i)} className={cn("flex gap-3", l.role === "user" && "flex-row-reverse")}>
            {l.role === "user" ? (
              <div className="flex h-8 w-8 shrink-0 items-center justify-center rounded-lg bg-elevated font-mono text-[10px] font-bold">{app.t("chat.you")}</div>
            ) : (
              <Avatar name={agentName} />
            )}
            <div
              className={cn(
                "max-w-[80%] whitespace-pre-wrap rounded-xl px-3.5 py-2.5 text-xs leading-relaxed",
                l.role === "user" ? "bg-accent text-white" : l.role === "system" ? "border border-rose-500/40 bg-rose-500/10 text-rose-600 dark:text-rose-300" : "border border-line bg-elevated",
              )}
            >
              {l.role === "assistant" ? <Markdown text={l.text} /> : l.text}
            </div>
          </ListItem>
        ))}
        {busy && (
          <div className="flex gap-3">
            <Avatar name={agentName} />
            <div className="max-w-[80%] whitespace-pre-wrap rounded-xl border border-line bg-elevated px-3.5 py-2.5 text-xs">
              {(streaming && <Markdown text={streaming} />) || (
                <span className="inline-flex items-center gap-2 text-subtle">
                  <Loader2 className="h-3.5 w-3.5 animate-spin" /> {app.t("chat.thinking")}
                </span>
              )}
            </div>
          </div>
        )}
        {lines.length === 0 && !busy && <Empty>{app.t("chat.placeholder")}</Empty>}
        {after}
        <div ref={end} />
      </div>
      <div className="flex gap-2 border-t border-line p-3">
        <textarea
          ref={input}
          className="input min-h-[44px] resize-none"
          rows={2}
          placeholder={app.t("chat.placeholder")}
          value={text}
          disabled={busy || !enabled}
          onChange={(e) => setText(e.target.value)}
          onKeyDown={(e) => {
            if (e.key === "Enter" && !e.shiftKey) {
              e.preventDefault();
              send();
            }
          }}
        />
        <button className="btn-primary self-end" disabled={busy || !text.trim() || !enabled} onClick={send}>
          <Send className="h-3.5 w-3.5" /> {app.t("common.send")}
        </button>
      </div>
    </div>
  );
}
