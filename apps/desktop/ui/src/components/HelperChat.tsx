// The chat of a drafting helper (ADR-151: the team designer, the skill writer): the user picks the model it runs on (an
// API provider or a CLI, the model and its effort), writes, and reads its replies; the draft it makes shows beside the
// chat (the caller's `children`). The helper's whole answer goes back with the next message, so "make it smaller"
// edits that draft.
import { Bot, Loader2, Send, User, X } from "lucide-react";
import { type ReactNode, useEffect, useRef, useState } from "react";
import { api, type ChatTurn, type ModelArg } from "../lib/api";
import { friendly } from "../lib/errors";
import { useApp } from "../lib/store";
import type { Capabilities } from "../lib/types";
import { ModelPicker, emptyModel } from "../views/organize/ModelPicker";
import { Markdown } from "./Markdown";
import { Overlay } from "./motion";
import { cn } from "./ui";

/** What a helper answered to one turn. */
export interface HelperAnswer {
  reply: string;
  questions: string[];
  raw: string;
  model: string;
}

const MODEL_KEY = "agenium-helper-model";

function savedModel(): ModelArg {
  try {
    const m = JSON.parse(localStorage.getItem(MODEL_KEY) ?? "null");
    if (m && typeof m.provider === "string") return { ...emptyModel, ...m };
  } catch {
    /* none kept */
  }
  return emptyModel;
}

export function HelperChat<A extends HelperAnswer>({
  title,
  intro,
  placeholder,
  first,
  ask,
  onAnswer,
  extra,
  children,
  onClose,
}: {
  title: string;
  intro: string;
  placeholder: string;
  /** A first message, written in the box for the user to send (the skill a team draft proposed). */
  first?: string;
  ask: (model: ModelArg, turns: ChatTurn[]) => Promise<A>;
  onAnswer: (a: A) => void;
  /** Shown above the message box (the skill writer's documents). */
  extra?: ReactNode;
  /** The draft, beside the chat. */
  children: ReactNode;
  onClose: () => void;
}) {
  const app = useApp();
  const [caps, setCaps] = useState<Capabilities | null>(null);
  const [model, setModel] = useState<ModelArg>(savedModel);
  const [turns, setTurns] = useState<ChatTurn[]>([]);
  const [shown, setShown] = useState<{ user: boolean; text: string; questions?: string[]; model?: string }[]>([]);
  const [text, setText] = useState(first ?? "");
  const [busy, setBusy] = useState(false);
  const end = useRef<HTMLDivElement>(null);
  useEffect(() => {
    api.capabilities(false, true).then(setCaps).catch(() => undefined);
  }, []);
  useEffect(() => {
    end.current?.scrollIntoView({ block: "end" });
  }, [shown.length, busy]);
  const ready = !!model.provider && !!model.model;
  const send = () => {
    const msg = text.trim();
    if (!msg || !ready || busy) return;
    try {
      localStorage.setItem(MODEL_KEY, JSON.stringify(model));
    } catch {
      /* the choice is not kept */
    }
    const next: ChatTurn[] = [...turns, { role: "user", text: msg }];
    setShown((s) => [...s, { user: true, text: msg }]);
    setText("");
    setBusy(true);
    ask(model, next)
      .then((a) => {
        setTurns([...next, { role: "assistant", text: a.raw }]);
        setShown((s) => [...s, { user: false, text: a.reply, questions: a.questions, model: a.model }]);
        onAnswer(a);
      })
      .catch((e) => {
        // The message stays in the box to send again (another model, after a login...).
        setShown((s) => s.slice(0, -1));
        setText(msg);
        app.notify(friendly(e, app.t));
      })
      .finally(() => setBusy(false));
  };
  return (
    <Overlay open onClose={() => !busy && onClose()} kind="dialog" top="pt-[3vh]">
      <div role="dialog" aria-modal="true" aria-label={title} className="card flex h-[92vh] w-[min(1500px,96vw)] flex-col gap-3 p-4" onKeyDown={(e) => e.key === "Escape" && !busy && onClose()}>
        <div className="flex items-center gap-2">
          <Bot className="h-4 w-4 text-accent" />
          <span className="text-sm font-semibold">{title}</span>
          <button className="btn ml-auto px-2" disabled={busy} onClick={onClose} title={app.t("common.cancel")}>
            <X className="h-3.5 w-3.5" />
          </button>
        </div>
        <div className="flex min-h-0 flex-1 gap-4">
          <div className="flex w-[min(680px,46%)] shrink-0 flex-col gap-2">
            <div className="rounded-lg border border-line bg-elevated/30 p-2.5">
              <div className="label mb-1.5">{app.t("helper.model")}</div>
              <ModelPicker value={model} onChange={setModel} caps={caps} />
            </div>
            <div className="min-h-0 flex-1 space-y-3 overflow-y-auto rounded-lg border border-line p-3">
              {shown.length === 0 && <div className="text-xs leading-relaxed text-subtle">{intro}</div>}
              {shown.map((m, i) => (
                <div key={i} className={cn("flex gap-2", m.user && "flex-row-reverse")}>
                  <span className={cn("mt-0.5 flex h-6 w-6 shrink-0 items-center justify-center rounded-full", m.user ? "bg-accent/15 text-accent" : "bg-cyan-500/15 text-cyan-600 dark:text-cyan-300")}>
                    {m.user ? <User className="h-3.5 w-3.5" /> : <Bot className="h-3.5 w-3.5" />}
                  </span>
                  <div className={cn("min-w-0 max-w-[85%] rounded-lg px-3 py-2 text-xs", m.user ? "bg-accent/10" : "bg-elevated")}>
                    {m.user ? <div className="whitespace-pre-wrap">{m.text}</div> : <Markdown text={m.text || "…"} />}
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
            </div>
            {extra}
            <div className="flex items-end gap-2">
              <textarea
                className="input h-20 min-w-0 flex-1 resize-none"
                value={text}
                placeholder={placeholder}
                disabled={busy}
                onChange={(e) => setText(e.target.value)}
                onKeyDown={(e) => {
                  if (e.key === "Enter" && !e.shiftKey && !e.nativeEvent.isComposing) {
                    e.preventDefault();
                    send();
                  }
                }}
              />
              <button className="btn-primary" disabled={busy || !ready || !text.trim()} onClick={send} title={ready ? app.t("helper.send") : app.t("helper.pickModel")}>
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
