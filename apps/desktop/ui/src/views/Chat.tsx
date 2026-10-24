import { Loader2, Send } from "lucide-react";
import { useEffect, useRef, useState } from "react";
import { Markdown } from "../components/Markdown";
import { Avatar, Empty, PageHeader, cn } from "../components/ui";
import { api } from "../lib/api";
import { useApp } from "../lib/store";
import type { ChatLine } from "../lib/types";

/** Talk to the project's Lead (or Solo agent) in your language: discovery, spec, plan, steering. */
export function ChatView() {
  const app = useApp();
  const p = app.projects.find((x) => x.id === app.project);
  const [lines, setLines] = useState<ChatLine[]>([]);
  const [text, setText] = useState("");
  const [busy, setBusy] = useState(false);
  const end = useRef<HTMLDivElement>(null);
  useEffect(() => {
    if (app.project) api.chatHistory(app.project).then(setLines);
  }, [app.project]);
  useEffect(() => {
    // Recent Chromium returns a Promise here: never let it become the effect cleanup.
    void end.current?.scrollIntoView({ behavior: "smooth" });
  }, [lines, busy, app.live]);

  // While a turn runs, show the text of the newest live run that started after the message was sent.
  const streaming = busy
    ? Object.entries(app.live)
        .filter(([, l]) => !l.done)
        .map(([, l]) => l.text)
        .filter(Boolean)
        .slice(-1)[0]
    : undefined;

  const send = () => {
    if (!app.project || !text.trim()) return;
    const msg = text.trim();
    setText("");
    setLines((l) => [...l, { role: "user", text: msg }]);
    setBusy(true);
    api
      .chat(app.project, msg)
      .then((r) => setLines((l) => [...l, { role: "assistant", text: r.text || `(${r.outcome})` }]))
      .catch((e) => setLines((l) => [...l, { role: "system", text: String(e) }]))
      .finally(() => (setBusy(false), app.refresh()));
  };

  if (!p) return <Empty>{app.t("common.none")}</Empty>;
  const lead = p.team ? (p.team.solo ? "Solo" : "Lead") : "–";
  return (
    <div className="flex h-full flex-col">
      <PageHeader title={app.t("chat.title")} sub={`${p.key_prefix} — ${p.name} · ${p.team?.name ?? "no team"} (${lead})`} />
      <div className="card flex min-h-0 flex-1 flex-col">
        <div className="min-h-0 flex-1 space-y-4 overflow-y-auto p-5">
          {lines.map((l, i) => (
            <div key={i} className={cn("flex gap-3", l.role === "user" && "flex-row-reverse")}>
              {l.role === "user" ? <div className="flex h-8 w-8 shrink-0 items-center justify-center rounded-lg bg-elevated font-mono text-[10px] font-bold">{app.t("chat.you")}</div> : <Avatar name={p.team?.name ?? "lead"} />}
              <div
                className={cn(
                  "max-w-[75%] whitespace-pre-wrap rounded-xl px-3.5 py-2.5 text-xs leading-relaxed",
                  l.role === "user" ? "bg-accent text-white" : l.role === "system" ? "border border-rose-500/40 bg-rose-500/10 text-rose-600 dark:text-rose-300" : "border border-line bg-elevated",
                )}
              >
                {l.role === "assistant" ? <Markdown text={l.text} /> : l.text}
              </div>
            </div>
          ))}
          {busy && (
            <div className="flex gap-3">
              <Avatar name={p.team?.name ?? "lead"} />
              <div className="max-w-[75%] whitespace-pre-wrap rounded-xl border border-line bg-elevated px-3.5 py-2.5 text-xs">
                {(streaming && <Markdown text={streaming} />) || (
                  <span className="inline-flex items-center gap-2 text-subtle">
                    <Loader2 className="h-3.5 w-3.5 animate-spin" /> {app.t("chat.thinking")}
                  </span>
                )}
              </div>
            </div>
          )}
          {lines.length === 0 && !busy && <Empty>{app.t("chat.placeholder")}</Empty>}
          <div ref={end} />
        </div>
        <div className="flex gap-2 border-t border-line p-3">
          <textarea
            className="input min-h-[44px] resize-none"
            rows={2}
            placeholder={app.t("chat.placeholder")}
            value={text}
            disabled={busy || !p.team}
            onChange={(e) => setText(e.target.value)}
            onKeyDown={(e) => {
              if (e.key === "Enter" && !e.shiftKey) {
                e.preventDefault();
                send();
              }
            }}
          />
          <button className="btn-primary self-end" disabled={busy || !text.trim() || !p.team} onClick={send}>
            <Send className="h-3.5 w-3.5" /> {app.t("common.send")}
          </button>
        </div>
      </div>
    </div>
  );
}
