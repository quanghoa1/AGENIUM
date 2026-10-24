// A small Markdown renderer for agent text (chat replies, reports, comments). It builds React elements — never HTML
// strings — so agent output cannot inject markup. Supported: headings, paragraphs, bullet/numbered lists, fenced
// code, inline code, bold, italic, links (shown, not navigated), horizontal rules.
import { Fragment, type ReactNode } from "react";
import { cn } from "./ui";

function inline(text: string, key = 0): ReactNode[] {
  const out: ReactNode[] = [];
  // `code`, **bold**, *italic* / _italic_, [label](url)
  const re = /(`[^`]+`)|(\*\*[^*]+\*\*)|(\*[^*\s][^*]*\*)|(_[^_\s][^_]*_)|(\[[^\]]+\]\([^)\s]+\))/g;
  let last = 0;
  let m: RegExpExecArray | null;
  let i = 0;
  while ((m = re.exec(text))) {
    if (m.index > last) out.push(text.slice(last, m.index));
    const t = m[0];
    const k = `${key}-${i++}`;
    if (m[1]) out.push(<code key={k} className="rounded bg-background px-1 py-0.5 font-mono text-[0.92em]">{t.slice(1, -1)}</code>);
    else if (m[2]) out.push(<strong key={k}>{inline(t.slice(2, -2), i)}</strong>);
    else if (m[3] || m[4]) out.push(<em key={k}>{inline(t.slice(1, -1), i)}</em>);
    else if (m[5]) {
      const label = t.slice(1, t.indexOf("]"));
      const url = t.slice(t.indexOf("(") + 1, -1);
      out.push(
        <span key={k} className="text-accent underline decoration-dotted" title={url}>
          {label}
        </span>,
      );
    }
    last = m.index + t.length;
  }
  if (last < text.length) out.push(text.slice(last));
  return out;
}

export function Markdown({ text, className }: { text: string; className?: string }) {
  const lines = (text ?? "").replace(/\r\n/g, "\n").split("\n");
  const blocks: ReactNode[] = [];
  let i = 0;
  let n = 0;
  while (i < lines.length) {
    const l = lines[i];
    if (/^```/.test(l)) {
      const body: string[] = [];
      i++;
      while (i < lines.length && !/^```/.test(lines[i])) body.push(lines[i++]);
      i++;
      blocks.push(
        <pre key={n++} className="overflow-x-auto rounded-lg border border-line bg-background p-2.5 font-mono text-[11px] leading-relaxed">
          {body.join("\n")}
        </pre>,
      );
      continue;
    }
    const h = /^(#{1,4})\s+(.*)$/.exec(l);
    if (h) {
      const size = ["text-base", "text-sm", "text-[13px]", "text-xs"][h[1].length - 1];
      blocks.push(
        <div key={n++} className={cn("font-semibold", size)}>
          {inline(h[2])}
        </div>,
      );
      i++;
      continue;
    }
    if (/^\s*(?:-\s*){3,}$|^\s*(?:\*\s*){3,}$|^\s*(?:_\s*){3,}$/.test(l)) {
      blocks.push(<hr key={n++} className="border-line" />);
      i++;
      continue;
    }
    if (/^\s*([-*+]|\d+[.)])\s+/.test(l)) {
      const ordered = /^\s*\d+[.)]/.test(l);
      const items: { depth: number; text: string }[] = [];
      while (i < lines.length && /^\s*([-*+]|\d+[.)])\s+/.test(lines[i])) {
        const mm = /^(\s*)([-*+]|\d+[.)])\s+(.*)$/.exec(lines[i])!;
        items.push({ depth: Math.min(3, Math.floor(mm[1].length / 2)), text: mm[3] });
        i++;
      }
      const List = ordered ? "ol" : "ul";
      blocks.push(
        <List key={n++} className={cn("space-y-0.5 pl-5", ordered ? "list-decimal" : "list-disc")}>
          {items.map((it, k) => (
            <li key={k} style={{ marginLeft: it.depth * 14 }}>
              {inline(it.text, k)}
            </li>
          ))}
        </List>,
      );
      continue;
    }
    if (!l.trim()) {
      i++;
      continue;
    }
    const para: string[] = [];
    while (i < lines.length && lines[i].trim() && !/^(```|#{1,4}\s|\s*([-*+]|\d+[.)])\s+)/.test(lines[i])) para.push(lines[i++]);
    blocks.push(
      <p key={n++}>
        {para.map((p, k) => (
          <Fragment key={k}>
            {k > 0 && <br />}
            {inline(p, k)}
          </Fragment>
        ))}
      </p>,
    );
  }
  return <div className={cn("space-y-2 break-words leading-relaxed", className)}>{blocks}</div>;
}
