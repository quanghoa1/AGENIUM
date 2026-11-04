// The conversation with a project's Lead (or Solo agent), like a CLI chat: history, the turn in progress (its
// reasoning folded in an expandable "thinking" block, the tool being called, the text as it streams), the input.
// Used by the chat page and by the Live tab (where the input can be pre-filled and focused from elsewhere).
import { Brain, ChevronDown, ChevronRight, Loader2, MessageSquarePlus, Send, Wrench, X } from "lucide-react";
import { useEffect, useRef, useState } from "react";
import { api } from "../lib/api";
import { useApp, useLive, type LiveRun } from "../lib/store";
import type { ChatLine, QueuedMessage } from "../lib/types";
import { AttachmentBar, DocChip, material, splitMaterial, useAttachments } from "./Attachments";
import { LeadQueue } from "./LeadQueue";
import { useMentions } from "./Mentions";
import { Markdown, TicketLinks } from "./Markdown";
import { ListItem } from "./motion";
import { Avatar, Empty, cn } from "./ui";
import { CallBadge, classify } from "../views/live/calls";

// A long paste shows as one label in the input ("[Pasted #1 · 120 lines · 8.4 KB]") and is put back on send.
const PASTE_CHARS = 1000;
const PASTE_LINES = 15;
const PASTE_TOKEN = /\[(?:Pasted|Đã dán) #(\d+)[^\]]*\]/g;
const lineCount = (t: string) => t.split("\n").length;
const size = (t: string) => {
  const b = new Blob([t]).size;
  return b < 1024 ? `${b} B` : `${(b / 1024).toFixed(1)} KB`;
};

/** A message of the user's: its text (folded when long) and the documents attached to it as chips (ADR-152). */
function UserText({ text }: { text: string }) {
  const m = splitMaterial(text);
  return (
    <>
      {m.text && <Folded text={m.text} />}
      {m.docs.length > 0 && (
        <span className="mt-1.5 flex flex-wrap gap-1">
          {m.docs.map((d) => (
            <DocChip key={d.name} name={d.name} size={d.size} className="border-white/40 text-white/90" />
          ))}
        </span>
      )}
    </>
  );
}

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

/** The chat as turns: each user message alone, and the agent's lines after it (memory, calls, text) as one turn. */
function turnsOf(lines: ChatLine[]): { at: number; user?: ChatLine; steps: ChatLine[] }[] {
  const out: { at: number; user?: ChatLine; steps: ChatLine[] }[] = [];
  lines.forEach((l, i) => {
    const last = out[out.length - 1];
    if (l.role === "user") out.push({ at: i, user: l, steps: [] });
    else if (last && !last.user) last.steps.push(l);
    else out.push({ at: i, steps: [l] });
  });
  return out;
}

/** One call the agent made in its turn: its kind (Agenium, Tool, MCP, Skill) and what it was, red when it failed. */
function Step({ line }: { line: ChatLine }) {
  const c = classify(line.text, line.args);
  return (
    <div className={cn("flex min-w-0 items-center gap-1.5 font-mono text-[11.5px]", line.ok === false ? "text-rose-600 dark:text-rose-400" : "text-muted")}>
      <span className={cn("h-1.5 w-1.5 shrink-0 rounded-full", line.ok === false ? "bg-rose-500" : "bg-emerald-500")} />
      <CallBadge kind={c.kind} />
      <span className="truncate" title={c.label}>
        {c.label}
      </span>
    </div>
  );
}

/** What the agent recalled from its memory for the user's message (ADR-155): its own, folded, first in its turn. */
function Recalled({ text }: { text: string }) {
  const app = useApp();
  const [open, setOpen] = useState(false);
  const items = text.split("\n").filter((l) => l.trim());
  return (
    <div>
      <div className="inline-block max-w-full rounded-lg border border-teal-500/30 bg-teal-500/5 text-[11px] text-teal-700 dark:text-teal-300">
        <button className="flex items-center gap-1.5 px-2.5 py-1 text-left" onClick={() => setOpen(!open)}>
          {open ? <ChevronDown className="h-3 w-3" /> : <ChevronRight className="h-3 w-3" />}
          <Brain className="h-3 w-3" /> {app.t("chat.recalled").replace("{n}", String(items.length))}
        </button>
        {open && (
          <ul className="space-y-0.5 px-3 pb-2 text-muted">
            {items.map((l, k) => (
              <li key={k}>{l.replace(/^- /, "")}</li>
            ))}
          </ul>
        )}
      </div>
    </div>
  );
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
        {/* Every call of the turn, one per line in the order made, the newest last and lit while it runs: an API model's tool runs in milliseconds, so
            showing only the current one let it flash by. */}
        {!!stream?.tools.length && (
          <div className="flex flex-col items-start gap-1">
            {stream.tools.slice(-8).map((c, k, all) => (
              <span
                key={k}
                className={cn(
                  "inline-flex max-w-full items-center gap-1.5 truncate rounded-lg border px-2 py-0.5 font-mono text-[11px]",
                  k === all.length - 1 && stream.phase === "tool" ? "border-purple-500/40 bg-purple-500/5 text-purple-600 dark:text-purple-300" : "border-line text-subtle",
                )}
              >
                <Wrench className="h-3 w-3 shrink-0" /> {classify(c.name, c.args).label}
              </span>
            ))}
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
  const att = useAttachments(project);
  const [note, setNote] = useState<string | null>(null);
  // This pane's own turns in flight (a message sent while one runs waits in the Lead's queue: ADR-149).
  const [turns, setTurns] = useState(0);
  const busy = turns > 0;
  const [queue, setQueue] = useState<QueuedMessage[]>([]);
  const end = useRef<HTMLDivElement>(null);
  const input = useRef<HTMLTextAreaElement>(null);
  const pastes = useRef(new Map<number, string>());
  // `@name` for a document of the conversation: attached now or sent earlier in it (the agent has those in the history).
  const named = [
    ...att.docs.map((d) => d.name),
    ...lines.filter((l) => l.role === "user").flatMap((l) => splitMaterial(l.text).docs.map((d) => d.name)),
  ];
  const mentions = useMentions(named, text, setText, input);
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
  // Any run of the Lead going on (a chat turn from another window, a review, an inbox run): a message waits for it.
  const leadWorking = busy || (!!agentId && Object.values(live).some((l) => !l.done && l.agent === agentId));
  const reloadQueue = () => api.leadQueue(project).then(setQueue).catch(() => {});
  useEffect(() => {
    setLines([]);
    setQueue([]);
    api.chatHistory(project).then(setLines);
    reloadQueue();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [project]);
  // A run of the agent that ends (the turn that read the queue among them) may have added to the conversation, and a
  // project that becomes done gets its Lead's summary in it. Not when a run starts: the message a turn takes is in
  // the conversation only once the turn ends, and a reload then would drop the one this pane just showed.
  // By the newest end, not by how many ended: the live store drops old finished runs, and a count that falls made the
  // pane reload mid-turn and lose the message it had just shown.
  const lastEnd = agentId ? Math.max(0, ...Object.values(live).filter((l) => l.done && l.agent === agentId).map((l) => l.at)) : 0;
  const seenEnd = useRef(0);
  useEffect(() => {
    if (lastEnd <= seenEnd.current) return;
    seenEnd.current = lastEnd;
    api.chatHistory(project).then(setLines);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [lastEnd]);
  const status = app.projects.find((p) => p.id === project)?.status;
  useEffect(() => {
    if (status === "Completed") api.chatHistory(project).then(setLines);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [status]);
  useEffect(() => {
    // While messages wait, the queue is read again; once the Lead's turn takes them, the conversation shows them.
    if (queue.length === 0) return;
    const t = window.setInterval(() => {
      api
        .leadQueue(project)
        .then((q) => {
          if (q.length < queue.length) api.chatHistory(project).then(setLines);
          setQueue(q);
        })
        .catch(() => {});
    }, 2500);
    return () => window.clearInterval(t);
  }, [project, queue.length]);
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
    // Files, or a file's path, are attached (read as Markdown) instead of typed.
    att.onPaste(e);
    if (e.defaultPrevented) return;
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
    const typed = text.trim().replace(PASTE_TOKEN, (label, id) => pastes.current.get(Number(id)) ?? label);
    // The attached documents go with the message, in the block the helpers use: material to read, not instructions.
    const msg = att.docs.length ? `${typed}\n\n${material(att.docs)}` : typed;
    pastes.current.clear();
    att.clear();
    setText("");
    setNote(null);
    // While the Lead works, the message joins its queue; otherwise it opens a turn now. The app decides; the pane
    // shows where it expects it and corrects itself from the reply.
    const waits = leadWorking;
    if (waits) setQueue((q) => [...q, { id: -Date.now(), text: msg, created_at: "" }]);
    else setLines((l) => [...l, { role: "user", text: msg }]);
    setTurns((n) => n + 1);
    api
      .chat(project, msg)
      .then((r) => {
        if (r.kind === "queued") {
          if (!waits) setLines((l) => l.filter((x, i) => !(i === l.length - 1 && x.role === "user" && x.text === msg)));
        } else {
          setLines((l) => [...l, { role: "assistant", text: r.text || `(${r.outcome})` }]);
          if (waits) api.chatHistory(project).then(setLines);
        }
      })
      .catch((e) => setLines((l) => [...l, { role: "system", text: String(e) }]))
      .finally(() => {
        setTurns((n) => n - 1);
        reloadQueue();
        app.refresh();
      });
  };

  const prefix = app.projects.find((p) => p.id === project)?.key_prefix;
  return (
    <TicketLinks.Provider value={prefix ? { prefix, open: (k) => app.openTicket(k, project) } : null}>
    <div className={cn("flex min-h-0 flex-1 flex-col", className)}>
      <div className="min-h-0 flex-1 space-y-4 overflow-y-auto p-5">
        {turnsOf(lines).map(({ at, user, steps }) =>
          user ? (
            <ListItem key={at} id={String(at)} className="flex flex-row-reverse gap-3">
              <div className="flex h-8 w-8 shrink-0 items-center justify-center rounded-lg bg-elevated font-mono text-[11px] font-bold">{app.t("chat.you")}</div>
              <div className="max-w-[80%] whitespace-pre-wrap rounded-xl bg-accent px-3.5 py-2.5 text-[13px] leading-relaxed text-white">
                <UserText text={user.text} />
              </div>
            </ListItem>
          ) : (
            // The agent's turn as a CLI shows it: what it recalled, each call it made, its answer, in that order.
            <ListItem key={at} id={String(at)} className="flex gap-3">
              <Avatar name={agentName} />
              <div className="min-w-0 max-w-[80%] flex-1 space-y-1.5">
                {steps.map((l, k) =>
                  l.role === "memory" ? (
                    <Recalled key={k} text={l.text} />
                  ) : l.role === "tool" ? (
                    <Step key={k} line={l} />
                  ) : (
                    <div
                      key={k}
                      className={cn(
                        "whitespace-pre-wrap rounded-xl px-3.5 py-2.5 text-[13px] leading-relaxed",
                        l.role === "system" ? "border border-rose-500/40 bg-rose-500/10 text-rose-600 dark:text-rose-300" : "border border-line bg-elevated",
                      )}
                    >
                      {l.role === "assistant" ? <Markdown text={l.text} /> : l.text}
                    </div>
                  ),
                )}
              </div>
            </ListItem>
          ),
        )}
        {(busy || current) && <LiveTurn name={agentName} stream={current} />}
        {lines.length === 0 && !busy && <Empty>{app.t("chat.intro")}</Empty>}
        <div ref={end} />
      </div>
      <LeadQueue
        project={project}
        agentName={agentName}
        queue={queue}
        working={leadWorking}
        reload={reloadQueue}
        answered={(p) => {
          setTurns((n) => n + 1);
          p.then((r) => {
            if (r.kind !== "queued") api.chatHistory(project).then(setLines);
          })
            .catch((e) => setLines((l) => [...l, { role: "system", text: String(e) }]))
            .finally(() => (setTurns((n) => n - 1), app.refresh()));
        }}
      />
      {note && (
        <div className="flex items-start gap-2 border-t border-cyan-500/30 bg-cyan-500/10 px-4 py-2 text-xs text-cyan-700 dark:text-cyan-300">
          <MessageSquarePlus className="mt-0.5 h-3.5 w-3.5 shrink-0" />
          <span className="flex-1">{note}</span>
          <button className="shrink-0 opacity-70 hover:opacity-100" title={app.t("common.close")} onClick={() => (setNote(null), setText(""))}>
            <X className="h-3.5 w-3.5" />
          </button>
        </div>
      )}
      <div className="border-t border-line px-3 pt-2">
        <AttachmentBar a={att} disabled={!enabled} />
      </div>
      <div className="relative flex gap-2 p-3 pt-2">
        {mentions.list}
        <textarea
          ref={input}
          className="input input-lg min-h-[48px] resize-none"
          rows={2}
          placeholder={app.t("chat.placeholder")}
          value={text}
          disabled={!enabled}
          onChange={(e) => (setText(e.target.value), mentions.track(e.target))}
          onSelect={(e) => mentions.track(e.currentTarget)}
          onPaste={onPaste}
          onKeyDown={(e) => {
            if (mentions.onKey(e)) return;
            if (e.key === "Backspace" && dropLabel(e)) return;
            // Not while an IME composes (the Vietnamese keyboard): that Enter ends the word, not the message (audit L16).
            if (e.key === "Enter" && !e.shiftKey && !e.nativeEvent.isComposing) {
              e.preventDefault();
              send();
            }
          }}
        />
        <button className="btn-primary btn-lg self-end" disabled={!text.trim() || !enabled || att.reading} onClick={send}>
          <Send className="h-3.5 w-3.5" /> {app.t("common.send")}
        </button>
      </div>
    </div>
    </TicketLinks.Provider>
  );
}
