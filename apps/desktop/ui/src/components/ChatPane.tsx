// The conversation with a project's Lead (or Solo agent), like a CLI chat: history, the turn in progress (its
// reasoning folded in an expandable "thinking" block, the tool being called, the text as it streams), the input.
// Used by the chat page and by the Live tab (where the input can be pre-filled and focused from elsewhere).
import { Brain, ChevronDown, ChevronRight, Loader2, MessageSquarePlus, Send, Wrench, X } from "lucide-react";
import { useEffect, useRef, useState } from "react";
import { api } from "../lib/api";
import { useApp, useLive, type LiveRun } from "../lib/store";
import type { ChatLine } from "../lib/types";
import { Markdown, TicketLinks } from "./Markdown";
import { ListItem } from "./motion";
import { Avatar, Empty, cn } from "./ui";

// A long paste shows as one label in the input ("[Pasted #1 · 120 lines · 8.4 KB]") and is put back on send.
const PASTE_CHARS = 1000;
const PASTE_LINES = 15;
const PASTE_TOKEN = /\[(?:Pasted|Đã dán) #(\d+)[^\]]*\]/g;
const lineCount = (t: string) => t.split("\n").length;
const size = (t: string) => {
  const b = new Blob([t]).size;
  return b < 1024 ? `${b} B` : `${(b / 1024).toFixed(1)} KB`;
};

/** A long message of the user's, folded to its first lines until asked for. */
function Folded({ text }: { text: string }) {
  const app = useApp();
  const [open, setOpen] = useState(false);
  const n = lineCount(text);
  if (n <= PASTE_LINES && text.length <= PASTE_CHARS * 1.5) return <>{text}</>;
  return (
    <>
      {open ? text : text.split("\n").slice(0, 8).join("\n").slice(0, 900) + "…"}
      <button className="mt-1.5 block text-[11px] font-semibold underline underline-offset-2 opacity-90 hover:opacity-100" onClick={() => setOpen(!open)}>
        {open ? app.t("chat.showLess") : app.t("chat.showMore").replace("{n}", String(n))}
      </button>
    </>
  );
}

export interface Prefill {
  text: string;
  /** What the drafted message is for, shown above the input until it is sent or dropped (a follow-up on a ticket). */
  note?: string;
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
            {open && <div className="max-h-56 overflow-y-auto whitespace-pre-wrap px-3 pb-2 font-mono text-[11.5px] leading-relaxed text-muted">{thinking.slice(-6000)}</div>}
          </div>
        ) : (
          reasoning && (
            <div className="inline-flex items-center gap-2 rounded-lg border border-line bg-elevated px-2.5 py-1.5 text-[11px] text-subtle">
              <Loader2 className="h-3.5 w-3.5 animate-spin" /> {app.t("live.thinking")}… <span className="text-[11px]">{stream ? app.t("chat.noReasoning") : ""}</span>
            </div>
          )
        )}
        {stream?.phase === "tool" && stream.tool && (
          <div className="inline-flex items-center gap-1.5 rounded-lg border border-purple-500/40 bg-purple-500/5 px-2.5 py-1 font-mono text-[11.5px] text-purple-600 dark:text-purple-300">
            <Wrench className="h-3 w-3" /> {stream.tool.replace(/^cli:/, "")}
          </div>
        )}
        {writing && (
          <div className="whitespace-pre-wrap rounded-xl border border-line bg-elevated px-3.5 py-2.5 text-[13px] leading-relaxed">
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
  stream,
  className,
}: {
  project: string;
  agentName: string;
  /** The agent's id: on the chat page its newest live run is the turn in progress (never another agent's). */
  agentId?: string | null;
  enabled?: boolean;
  prefill?: Prefill | null;
  /** The agent's run in progress (Live tab); shown as the turn in progress even when another window started it. */
  stream?: LiveRun;
  className?: string;
}) {
  const app = useApp();
  const live = useLive();
  const [lines, setLines] = useState<ChatLine[]>([]);
  const [text, setText] = useState("");
  const [note, setNote] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const end = useRef<HTMLDivElement>(null);
  const input = useRef<HTMLTextAreaElement>(null);
  const pastes = useRef(new Map<number, string>());
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
    setNote(prefill.note ?? null);
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
  useEffect(() => {
    // The pane can shrink after the conversation is shown (the Lead's delegations above it arrive later): a
    // conversation read at its end stays at its end; one scrolled back stays where it is.
    const el = end.current?.parentElement;
    if (!el) return;
    let atEnd = true;
    const onScroll = () => {
      atEnd = el.scrollHeight - el.scrollTop - el.clientHeight < 40;
    };
    const ro = new ResizeObserver(() => {
      if (atEnd) el.scrollTo({ top: el.scrollHeight });
    });
    el.addEventListener("scroll", onScroll, { passive: true });
    ro.observe(el);
    return () => {
      el.removeEventListener("scroll", onScroll);
      ro.disconnect();
    };
  }, []);

  const onPaste = (e: React.ClipboardEvent<HTMLTextAreaElement>) => {
    const pasted = e.clipboardData.getData("text");
    if (pasted.length <= PASTE_CHARS && lineCount(pasted) <= PASTE_LINES) return;
    e.preventDefault();
    const id = pastes.current.size + 1;
    pastes.current.set(id, pasted);
    const label = `[${app.t("chat.pasted")} #${id} · ${lineCount(pasted)} ${app.t("chat.lines")} · ${size(pasted)}]`;
    const el = e.currentTarget;
    const [a, b] = [el.selectionStart, el.selectionEnd];
    const next = text.slice(0, a) + label + text.slice(b);
    setText(next);
    window.setTimeout(() => el.setSelectionRange(a + label.length, a + label.length));
  };
  // Backspace right after a paste label removes the whole label (and the paste with it).
  const dropLabel = (e: React.KeyboardEvent<HTMLTextAreaElement>) => {
    const el = e.currentTarget;
    if (el.selectionStart !== el.selectionEnd) return false;
    const before = text.slice(0, el.selectionStart);
    const m = before.match(/\[(?:Pasted|Đã dán) #\d+[^\]]*\]$/);
    if (!m) return false;
    e.preventDefault();
    const at = before.length - m[0].length;
    setText(text.slice(0, at) + text.slice(el.selectionStart));
    window.setTimeout(() => el.setSelectionRange(at, at));
    return true;
  };

  const send = () => {
    if (!text.trim() || !enabled) return;
    const msg = text.trim().replace(PASTE_TOKEN, (label, id) => pastes.current.get(Number(id)) ?? label);
    pastes.current.clear();
    setText("");
    setNote(null);
    setLines((l) => [...l, { role: "user", text: msg }]);
    setBusy(true);
    api
      .chat(project, msg)
      .then((r) => setLines((l) => [...l, { role: "assistant", text: r.text || `(${r.outcome})` }]))
      .catch((e) => setLines((l) => [...l, { role: "system", text: String(e) }]))
      .finally(() => (setBusy(false), app.refresh()));
  };

  const prefix = app.projects.find((p) => p.id === project)?.key_prefix;
  return (
    <TicketLinks.Provider value={prefix ? { prefix, open: (k) => app.openTicket(k, project) } : null}>
    <div className={cn("flex min-h-0 flex-1 flex-col", className)}>
      <div className="min-h-0 flex-1 space-y-4 overflow-y-auto p-5">
        {lines.map((l, i) => (
          <ListItem key={i} id={String(i)} className={cn("flex gap-3", l.role === "user" && "flex-row-reverse")}>
            {l.role === "user" ? (
              <div className="flex h-8 w-8 shrink-0 items-center justify-center rounded-lg bg-elevated font-mono text-[11px] font-bold">{app.t("chat.you")}</div>
            ) : (
              <Avatar name={agentName} />
            )}
            <div
              className={cn(
                "max-w-[80%] whitespace-pre-wrap rounded-xl px-3.5 py-2.5 text-[13px] leading-relaxed",
                l.role === "user" ? "bg-accent text-white" : l.role === "system" ? "border border-rose-500/40 bg-rose-500/10 text-rose-600 dark:text-rose-300" : "border border-line bg-elevated",
              )}
            >
              {l.role === "assistant" ? <Markdown text={l.text} /> : l.role === "user" ? <Folded text={l.text} /> : l.text}
            </div>
          </ListItem>
        ))}
        {(busy || current) && <LiveTurn name={agentName} stream={current} />}
        {lines.length === 0 && !busy && <Empty>{app.t("chat.intro")}</Empty>}
        <div ref={end} />
      </div>
      {note && (
        <div className="flex items-start gap-2 border-t border-cyan-500/30 bg-cyan-500/10 px-4 py-2 text-xs text-cyan-700 dark:text-cyan-300">
          <MessageSquarePlus className="mt-0.5 h-3.5 w-3.5 shrink-0" />
          <span className="flex-1">{note}</span>
          <button className="shrink-0 opacity-70 hover:opacity-100" title={app.t("common.close")} onClick={() => (setNote(null), setText(""))}>
            <X className="h-3.5 w-3.5" />
          </button>
        </div>
      )}
      <div className="flex gap-2 border-t border-line p-3">
        <textarea
          ref={input}
          className="input input-lg min-h-[48px] resize-none"
          rows={2}
          placeholder={app.t("chat.placeholder")}
          value={text}
          disabled={busy || !enabled}
          onChange={(e) => setText(e.target.value)}
          onPaste={onPaste}
          onKeyDown={(e) => {
            if (e.key === "Backspace" && dropLabel(e)) return;
            if (e.key === "Enter" && !e.shiftKey) {
              e.preventDefault();
              send();
            }
          }}
        />
        <button className="btn-primary btn-lg self-end" disabled={busy || !text.trim() || !enabled} onClick={send}>
          <Send className="h-3.5 w-3.5" /> {app.t("common.send")}
        </button>
      </div>
    </div>
    </TicketLinks.Provider>
  );
}
