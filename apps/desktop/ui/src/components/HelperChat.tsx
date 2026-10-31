// The chat of a drafting helper (ADR-151: the team designer, the skill writer): the user picks the model it runs on (an
// API provider or a CLI, the model and its effort), writes, attaches documents, and reads its replies; the draft it
// makes shows beside the chat (the caller's `children`). The helper's whole answer goes back with the next message,
// so "make it smaller" edits that draft. Each chat is a session kept in the root (ADR-152): saved after every answer,
// listed under History and reopened where it stopped.
import { Bot, History, Loader2, Plus, Send, User, X } from "lucide-react";
import { type ReactNode, useEffect, useRef, useState } from "react";
import { api, type ChatTurn, type HelperSession, type ModelArg, type PrefKey, type SkillDoc } from "../lib/api";
import { remember, rememberedModel, usable } from "../lib/remembered";
import { friendly } from "../lib/errors";
import { useApp } from "../lib/store";
import type { Capabilities } from "../lib/types";
import { ModelPicker, emptyModel } from "../views/organize/ModelPicker";
import { AttachmentBar, DocChip, useAttachments } from "./Attachments";
import { HelperHistory } from "./HelperHistory";
import { Markdown } from "./Markdown";
import { Overlay } from "./motion";
import { cn } from "./ui";

/** What a helper answered to one turn. */
export interface HelperAnswer {
  reply: string;
  questions: string[];
  raw: string;
  model: string;
  draft: unknown;
}

type Shown = { user: boolean; text: string; questions?: string[]; model?: string; docs?: { name: string; size: number }[] };

export function HelperChat<A extends HelperAnswer>({
  kind,
  prefKey,
  title,
  intro,
  placeholder,
  first,
  initialDocs,
  source,
  open,
  queued,
  ask,
  onAnswer,
  onSession,
  onRestore,
  children,
  onClose,
}: {
  /** The helper: its sessions are listed apart. */
  kind: "design" | "skill";
  /** Where this helper's model is kept (each helper remembers its own, ADR-152). */
  prefKey: PrefKey;
  title: string;
  intro: string;
  placeholder: string;
  /** A first message, written in the box for the user to send (the skill a team draft proposed). */
  first?: string;
  /** Documents attached from the start (an agent's proposal and its evidence). */
  initialDocs?: SkillDoc[];
  /** What the chat starts from, kept with its session (`{ seed }`, `{ proposal }`). */
  source?: unknown;
  /** A session to reopen. */
  open?: string;
  /** A message to send now (a button beside the draft asks for a change); a new `n` sends it again. */
  queued?: { text: string; n: number } | null;
  ask: (model: ModelArg, turns: ChatTurn[], docs: SkillDoc[], session: string) => Promise<A>;
  onAnswer: (a: A) => void;
  /** The session this chat is kept in, once it has one. */
  onSession?: (id: string) => void;
  /** A reopened session's last draft. */
  onRestore?: (draft: unknown) => void;
  /** The draft, beside the chat. */
  children: ReactNode;
  onClose: () => void;
}) {
  const app = useApp();
  const [caps, setCaps] = useState<Capabilities | null>(null);
  const [model, setModel] = useState<ModelArg>(emptyModel);
  const [gone, setGone] = useState<string | null>(null);
  const [turns, setTurns] = useState<ChatTurn[]>([]);
  const [shown, setShown] = useState<Shown[]>([]);
  const [text, setText] = useState(first ?? "");
  const [busy, setBusy] = useState(false);
  const [session, setSession] = useState<string | null>(null);
  const [history, setHistory] = useState(false);
  const att = useAttachments();
  useEffect(() => {
    if (initialDocs?.length) att.setDocs(initialDocs);
  }, []);
  // Documents already sent go with every later turn (the helper reads them each time); new ones show on the next message.
  const [sentDocs, setSentDocs] = useState<string[]>([]);
  const end = useRef<HTMLDivElement>(null);
  useEffect(() => {
    Promise.all([api.capabilities(false, true), api.preference(prefKey).catch(() => null)])
      .then(([c, kept]) => {
        setCaps(c);
        const r = rememberedModel(kept, c);
        setGone(r.gone);
        if (r.model) setModel(r.model);
      })
      .catch(() => undefined);
  }, [prefKey]);
  // Reopened once the models this computer has are known (the session's model is used if it still runs here).
  useEffect(() => {
    if (open && caps) api.session(open).then(restore, (e) => app.notify(friendly(e, app.t)));
  }, [open, !!caps]);
  useEffect(() => {
    end.current?.scrollIntoView({ block: "end" });
  }, [shown.length, busy]);
  const ready = !!model.provider && !!model.model;

  const restore = (s: HelperSession) => {
    setSession(s.id);
    onSession?.(s.id);
    setTurns(s.turns ?? []);
    setShown(s.transcript ?? []);
    att.setDocs(s.attachments ?? []);
    setSentDocs((s.attachments ?? []).map((d) => d.name));
    if (s.model && caps && usable(s.model, caps)) setModel(s.model);
    else if (s.model?.model) setGone(s.model.model);
    setText("");
    setHistory(false);
    onRestore?.(s.draft ?? null);
  };
  const fresh = () => {
    setSession(null);
    setTurns([]);
    setShown([]);
    att.setDocs(initialDocs ?? []);
    setSentDocs([]);
    setText(first ?? "");
    setHistory(false);
    onRestore?.(null);
  };

  const send = async (raw?: string) => {
    const msg = (raw ?? text).trim();
    if (!msg || !ready || busy || att.reading) return;
    remember(prefKey, model);
    const docs = att.docs;
    const fresh = docs.filter((d) => !sentDocs.includes(d.name)).map((d) => ({ name: d.name, size: d.text.length }));
    const next: ChatTurn[] = [...turns, { role: "user", text: msg }];
    const before = shown;
    const mine: Shown = { user: true, text: msg, docs: fresh.length ? fresh : undefined };
    setShown([...before, mine]);
    if (raw === undefined) setText("");
    setBusy(true);
    try {
      const id = session ?? (await api.newSession(kind, source ?? null, model)).id;
      if (!session) {
        setSession(id);
        onSession?.(id);
      }
      const a = await ask(model, next, docs, id);
      const turnsAfter: ChatTurn[] = [...next, { role: "assistant", text: a.raw }];
      const shownAfter: Shown[] = [...before, mine, { user: false, text: a.reply, questions: a.questions, model: a.model }];
      setTurns(turnsAfter);
      setShown(shownAfter);
      setSentDocs(docs.map((d) => d.name));
      onAnswer(a);
      api
        .saveSession(id, { model, turns: turnsAfter, transcript: shownAfter, attachments: docs, draft: a.draft ?? undefined })
        .catch((e) => app.notify(friendly(e, app.t)));
    } catch (e) {
      // The message stays in the box to send again (another model, after a login...).
      setShown(before);
      if (raw === undefined) setText(msg);
      app.notify(friendly(e, app.t));
    } finally {
      setBusy(false);
    }
  };
  useEffect(() => {
    if (queued) send(queued.text);
  }, [queued?.n]);

  return (
    <Overlay open onClose={() => !busy && onClose()} kind="dialog" top="pt-[3vh]">
      <div role="dialog" aria-modal="true" aria-label={title} className="card flex h-[92vh] w-[min(1500px,96vw)] flex-col gap-3 p-4" onKeyDown={(e) => e.key === "Escape" && !busy && onClose()}>
        <div className="flex items-center gap-2">
          <Bot className="h-4 w-4 text-accent" />
          <span className="text-sm font-semibold">{title}</span>
          <button className={cn("btn ml-auto h-7 px-2 text-[11px]", history && "border-accent text-accent")} disabled={busy} onClick={() => setHistory((h) => !h)}>
            <History className="h-3.5 w-3.5" /> {app.t("helper.history")}
          </button>
          <button className="btn h-7 px-2 text-[11px]" disabled={busy || (!session && !shown.length)} onClick={fresh}>
            <Plus className="h-3.5 w-3.5" /> {app.t("helper.new")}
          </button>
          <button className="btn px-2" disabled={busy} onClick={onClose} title={app.t("common.cancel")}>
            <X className="h-3.5 w-3.5" />
          </button>
        </div>
        <div className="flex min-h-0 flex-1 gap-4">
          <div className="flex w-[min(680px,46%)] shrink-0 flex-col gap-2">
            <div className="rounded-lg border border-line bg-elevated/30 p-2.5">
              <div className="label mb-1.5">{app.t("helper.model")}</div>
              <ModelPicker value={model} onChange={setModel} caps={caps} />
              {gone && <div className="mt-1 text-[11px] text-amber-600 dark:text-amber-400">{app.t("models.keptGone").replace("{m}", gone)}</div>}
            </div>
            <div className="min-h-0 flex-1 space-y-3 overflow-y-auto rounded-lg border border-line p-3">
              {history ? (
                <HelperHistory kind={kind} current={session} onOpen={restore} />
              ) : (
                <>
                  {shown.length === 0 && <div className="text-xs leading-relaxed text-subtle">{intro}</div>}
                  {shown.map((m, i) => (
                    <div key={i} className={cn("flex gap-2", m.user && "flex-row-reverse")}>
                      <span className={cn("mt-0.5 flex h-6 w-6 shrink-0 items-center justify-center rounded-full", m.user ? "bg-accent/15 text-accent" : "bg-cyan-500/15 text-cyan-600 dark:text-cyan-300")}>
                        {m.user ? <User className="h-3.5 w-3.5" /> : <Bot className="h-3.5 w-3.5" />}
                      </span>
                      <div className={cn("min-w-0 max-w-[85%] rounded-lg px-3 py-2 text-xs", m.user ? "bg-accent/10" : "bg-elevated")}>
                        {m.user ? <div className="whitespace-pre-wrap">{m.text}</div> : <Markdown text={m.text || "…"} />}
                        {!!m.docs?.length && (
                          <div className="mt-1.5 flex flex-wrap gap-1">
                            {m.docs.map((d) => (
                              <DocChip key={d.name} name={d.name} size={d.size} />
                            ))}
                          </div>
                        )}
                        {!!m.questions?.length && (
                          <ol className="mt-2 list-decimal space-y-1 pl-4">
                            {m.questions.map((q, j) => (
                              <li key={j}>{q}</li>
                            ))}
                          </ol>
                        )}
                        {m.model && <div className="mt-1.5 font-mono text-[10px] text-subtle">{m.model}</div>}
                      </div>
                    </div>
                  ))}
                  {busy && (
                    <div className="flex items-center gap-2 text-xs text-subtle">
                      <Loader2 className="h-3.5 w-3.5 animate-spin" /> {app.t("helper.thinking")}
                    </div>
                  )}
                  <div ref={end} />
                </>
              )}
            </div>
            <AttachmentBar a={att} disabled={busy} />
            <div className="flex items-end gap-2">
              <textarea
                className="input h-20 min-w-0 flex-1 resize-none"
                value={text}
                placeholder={placeholder}
                disabled={busy}
                onPaste={att.onPaste}
                onChange={(e) => setText(e.target.value)}
                onKeyDown={(e) => {
                  if (e.key === "Enter" && !e.shiftKey && !e.nativeEvent.isComposing) {
                    e.preventDefault();
                    send();
                  }
                }}
              />
              <button className="btn-primary" disabled={busy || !ready || !text.trim() || att.reading} onClick={() => send()} title={ready ? app.t("helper.send") : app.t("helper.pickModel")}>
                {busy ? <Loader2 className="h-3.5 w-3.5 animate-spin" /> : <Send className="h-3.5 w-3.5" />} {app.t("helper.send")}
              </button>
            </div>
            {!ready && <div className="text-[11px] text-amber-600 dark:text-amber-400">{app.t("helper.pickModel")}</div>}
          </div>
          <div className="min-w-0 flex-1 overflow-y-auto rounded-lg border border-line bg-elevated/20 p-3">{children}</div>
        </div>
      </div>
    </Overlay>
  );
}
