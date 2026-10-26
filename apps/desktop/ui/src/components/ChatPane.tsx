// The conversation with a project's Lead (or Solo agent), like a CLI chat: history, the turn in progress (its
// reasoning folded in an expandable "thinking" block, the tool being called, the text as it streams), the input.
// Used by the chat page and by the Live tab (where the input can be pre-filled and focused from elsewhere).
import { Brain, ChevronDown, ChevronRight, Loader2, Send, Wrench } from "lucide-react";
import { useEffect, useRef, useState, type ReactNode } from "react";
import { api } from "../lib/api";
import { useApp, useLive, type LiveRun } from "../lib/store";
import type { ChatLine } from "../lib/types";
import { Markdown } from "./Markdown";
import { ListItem } from "./motion";
import { Avatar, Empty, cn } from "./ui";

export interface Prefill {
  text: string;
  /** Changes on every request so the same text can be asked for twice. */
  nonce: number;
}

/** The turn in progress: thinking (expandable, open while the model reasons), the tool being called, the text. */
export function LiveTurn({ name, stream }: { name: string; stream?: LiveRun }) {
  const app = useApp();
  const [open, setOpen] = useState(true);
  const writing = !!stream?.text;
  useEffect(() => {
    // Fold the reasoning once the answer starts.
    if (writing) setOpen(false);
  }, [writing]);
  const thinking = stream?.thinking.trim() ?? "";
  const reasoning = !stream || stream.phase === "thinking";
  return (
    <div className="flex gap-3">
      <Avatar name={name} />
      <div className="min-w-0 max-w-[85%] flex-1 space-y-1.5">
        {thinking ? (
          <div className="rounded-lg border border-blue-500/30 bg-blue-500/5">
            <button className="flex w-full items-center gap-1.5 px-2.5 py-1.5 text-left text-[11px] text-blue-600 dark:text-blue-300" onClick={() => setOpen(!open)}>
              {open ? <ChevronDown className="h-3 w-3" /> : <ChevronRight className="h-3 w-3" />}
              <Brain className={cn("h-3.5 w-3.5", reasoning && "animate-pulse")} />
              {reasoning ? `${app.t("live.thinking")}…` : app.t("chat.thought")}
            </button>
            {open && <div className="max-h-56 overflow-y-auto whitespace-pre-wrap px-3 pb-2 font-mono text-[10.5px] leading-relaxed text-muted">{thinking.slice(-6000)}</div>}
          </div>
        ) : (
          reasoning && (
            <div className="inline-flex items-center gap-2 rounded-lg border border-line bg-elevated px-2.5 py-1.5 text-[11px] text-subtle">
              <Loader2 className="h-3.5 w-3.5 animate-spin" /> {app.t("live.thinking")}… <span className="text-[10px]">{stream ? app.t("chat.noReasoning") : ""}</span>
            </div>
          )
        )}
        {stream?.phase === "tool" && stream.tool && (
          <div className="inline-flex items-center gap-1.5 rounded-lg border border-purple-500/40 bg-purple-500/5 px-2.5 py-1 font-mono text-[10.5px] text-purple-600 dark:text-purple-300">
            <Wrench className="h-3 w-3" /> {stream.tool.replace(/^cli:/, "")}
          </div>
        )}
        {writing && (
          <div className="whitespace-pre-wrap rounded-xl border border-line bg-elevated px-3.5 py-2.5 text-xs">
            <Markdown text={stream!.text} />
          </div>
        )}
      </div>
    </div>
  );
}

export function ChatPane({
  project,
  agentName,
  agentId,
  enabled = true,
  prefill,
  after,
  stream,
  className,
}: {
  project: string;
  agentName: string;
  /** The agent's id: on the chat page its newest live run is the turn in progress (never another agent's). */
  agentId?: string | null;
  enabled?: boolean;
  prefill?: Prefill | null;
  /** Rendered after the conversation (the Lead's delegation cards in the Live tab). */
  after?: ReactNode;
  /** The agent's run in progress (Live tab); shown as the turn in progress even when another window started it. */
  stream?: LiveRun;
  className?: string;
}) {
  const app = useApp();
  const live = useLive();
  const [lines, setLines] = useState<ChatLine[]>([]);
  const [text, setText] = useState("");
  const [busy, setBusy] = useState(false);
  const end = useRef<HTMLDivElement>(null);
  const input = useRef<HTMLTextAreaElement>(null);
  // The turn in progress: the agent's run when known, else (chat page) this agent's newest live run while we wait.
  const current =
    stream && !stream.done
      ? stream
      : busy && agentId
        ? Object.values(live)
            .filter((l) => !l.done && l.agent === agentId)
            .sort((a, b) => a.at - b.at)
            .slice(-1)[0]
        : undefined;
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
    // Scroll the conversation only (scrollIntoView would also scroll the page or pane around it), and only when
    // the user is at the bottom already: reading an earlier message is not interrupted by the streaming turn.
    const el = end.current?.parentElement;
    if (!el) return;
    if (el.scrollHeight - el.scrollTop - el.clientHeight < 160) el.scrollTo({ top: el.scrollHeight, behavior: "smooth" });
  }, [lines, busy, current?.text.length, current?.thinking.length]);
  useEffect(() => {
    // A message the user sends always brings the conversation to its end.
    const el = end.current?.parentElement;
    el?.scrollTo({ top: el.scrollHeight });
  }, [lines.length]);

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
        {(busy || current) && <LiveTurn name={agentName} stream={current} />}
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
