// `@name` in a chat message names one of the documents of the conversation: those attached to the message being
// written and those sent earlier in it (the agent reads them in the history, each under its name). Typing `@` opens
// the list; the letters after it narrow it; ↑/↓ move, Enter or Tab puts `@name` in, Esc closes. With ten files
// attached, `@br` + Enter is enough instead of typing a name again.
import { FileText } from "lucide-react";
import { useState, type KeyboardEvent, type RefObject } from "react";
import { cn } from "./ui";

/** The `@…` being typed just before the caret, and where it starts; `null` when there is none. */
function typing(text: string, caret: number): { at: number; query: string } | null {
  const m = /(^|\s)@([^\s@"]*)$/.exec(text.slice(0, caret));
  return m ? { at: caret - m[2].length - 1, query: m[2] } : null;
}

/** How a name is written after `@`: as it is, or quoted when it has spaces. */
export function mention(name: string): string {
  return /\s/.test(name) ? `@"${name}"` : `@${name}`;
}

export function useMentions(
  names: string[],
  text: string,
  setText: (t: string) => void,
  input: RefObject<HTMLTextAreaElement | null>,
) {
  const [caret, setCaret] = useState(0);
  const [pick, setPick] = useState(0);
  const [closed, setClosed] = useState<number | null>(null);
  const at = typing(text, caret);
  const unique = [...new Set(names)];
  const found = at ? unique.filter((n) => n.toLowerCase().includes(at.query.toLowerCase())) : [];
  const open = !!at && found.length > 0 && closed !== at.at;
  const insert = (name: string) => {
    if (!at) return;
    const put = `${mention(name)} `;
    const next = text.slice(0, at.at) + put + text.slice(caret);
    setText(next);
    const pos = at.at + put.length;
    window.setTimeout(() => {
      const el = input.current;
      if (!el) return;
      el.focus();
      el.setSelectionRange(pos, pos);
      setCaret(pos);
    });
  };
  /** Where the caret is, after each change or move (the list follows it). */
  const track = (el: HTMLTextAreaElement) => {
    setCaret(el.selectionStart ?? el.value.length);
    setPick(0);
  };
  /** The list's keys; `true` when the key was the list's (the input does nothing more with it). */
  const onKey = (e: KeyboardEvent<HTMLTextAreaElement>): boolean => {
    if (!open || !at) return false;
    if (e.key === "ArrowDown" || e.key === "ArrowUp") {
      e.preventDefault();
      setPick((p) => (p + (e.key === "ArrowDown" ? 1 : found.length - 1)) % found.length);
      return true;
    }
    if (e.key === "Enter" || e.key === "Tab") {
      e.preventDefault();
      insert(found[Math.min(pick, found.length - 1)]);
      return true;
    }
    if (e.key === "Escape") {
      e.preventDefault();
      setClosed(at.at);
      return true;
    }
    return false;
  };
  const list = open ? (
    <div className="absolute bottom-full left-0 z-50 mb-1 max-h-64 w-[26rem] max-w-full overflow-y-auto rounded-lg border border-line bg-surface p-1 shadow-lg">
      {found.map((n, i) => (
        <button
          type="button"
          key={n}
          className={cn("flex w-full items-center gap-2 rounded-md px-2.5 py-1.5 text-left text-xs", i === pick ? "bg-blue-600/15 text-blue-600 dark:text-blue-300" : "hover:bg-elevated")}
          onMouseDown={(e) => e.preventDefault()}
          onClick={() => insert(n)}
        >
          <FileText className="h-3.5 w-3.5 shrink-0" />
          <span className="truncate font-mono">{n}</span>
        </button>
      ))}
    </div>
  ) : null;
  return { list, onKey, track };
}
