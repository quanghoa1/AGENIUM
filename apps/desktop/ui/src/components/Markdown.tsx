// A small Markdown renderer for agent text (chat replies, reports, comments). It builds React elements — never HTML
// strings — so agent output cannot inject markup. Supported: headings, paragraphs, bullet/numbered lists, fenced
// code, inline code, bold, italic, strikethrough, links (shown, not navigated), horizontal rules, tables.
import { Fragment, createContext, useContext, type ReactNode } from "react";
import { cn } from "./ui";

/** One project's ticket keys made into links (the chat with its Lead): `prefix` like "AGN", `open` shows the ticket. */
export const TicketLinks = createContext<{ prefix: string; open: (key: string) => void } | null>(null);

/** Plain text, its ticket keys linked when a `TicketLinks` is around. */
function Linked({ text }: { text: string }) {
  const l = useContext(TicketLinks);
  if (!l) return <>{text}</>;
  const re = new RegExp(`\\b${l.prefix.replace(/[^A-Za-z0-9]/g, "")}-\\d+\\b`, "g");
  const out: ReactNode[] = [];
  let last = 0;
  for (const m of text.matchAll(re)) {
    out.push(text.slice(last, m.index));
    out.push(
      <button key={m.index} className="font-mono text-cyan-600 underline decoration-dotted hover:decoration-solid dark:text-cyan-400" onClick={() => l.open(m[0])}>
        {m[0]}
      </button>,
    );
    last = m.index + m[0].length;
  }
  out.push(text.slice(last));
  return <>{out}</>;
}

function inline(text: string, key = 0): ReactNode[] {
  const out: ReactNode[] = [];
  // `code`, **bold**, *italic* / _italic_, [label](url), ~~struck~~. An underscore inside a word is a letter, not
  // emphasis (CommonMark): test_calc.py and __main__ stay as written.
  const re = /(`[^`]+`)|(\*\*[^*]+\*\*)|(\*[^*\s][^*]*\*)|((?<![\p{L}\p{N}_])_[^_\s](?:[^_]*[^_\s])?_(?![\p{L}\p{N}_]))|(\[[^\]]+\]\([^)\s]+\))|(~~[^~]+~~)/gu;
  let last = 0;
  let m: RegExpExecArray | null;
  let i = 0;
  while ((m = re.exec(text))) {
    const k = `${key}-${i++}`;
    if (m.index > last) out.push(<Linked key={`${k}t`} text={text.slice(last, m.index)} />);
    const t = m[0];
    if (m[1]) out.push(<code key={k} className="rounded bg-background px-1 py-0.5 font-mono text-[0.92em]">{t.slice(1, -1)}</code>);
    else if (m[2]) out.push(<strong key={k}>{inline(t.slice(2, -2), i)}</strong>);
    else if (m[3] || m[4]) out.push(<em key={k}>{inline(t.slice(1, -1), i)}</em>);
    else if (m[6]) out.push(<del key={k} className="opacity-70">{inline(t.slice(2, -2), i)}</del>);
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
  if (last < text.length) out.push(<Linked key={`${key}-end`} text={text.slice(last)} />);
  return out;
}

/** The cells of a table row: split on `|`, not inside `code` and not when escaped (`\|`). */
export function cells(row: string): string[] {
  const t = row.trim().replace(/^\|/, "").replace(/\|$/, "");
  const out: string[] = [];
  let cur = "";
  let code = false;
  for (let k = 0; k < t.length; k++) {
    const c = t[k];
    if (c === "\\" && t[k + 1] === "|") {
      cur += "|";
      k++;
    } else if (c === "`") {
      code = !code;
      cur += c;
    } else if (c === "|" && !code) {
      out.push(cur.trim());
      cur = "";
    } else cur += c;
  }
  out.push(cur.trim());
  return out;
}

/** A table's separator row: `|---|:--:|`. */
const SEPARATOR = /^\s*\|?\s*:?-{3,}:?\s*(\|\s*:?-{3,}:?\s*)*\|?\s*$/;

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
    if (/^\s*\|/.test(l) && i + 1 < lines.length && SEPARATOR.test(lines[i + 1])) {
      const head = cells(l);
      const rows: string[][] = [];
      i += 2;
      while (i < lines.length && /^\s*\|/.test(lines[i])) rows.push(cells(lines[i++]));
      blocks.push(
        <div key={n++} className="overflow-x-auto">
          <table className="w-full border-collapse text-left text-[12px]">
            <thead>
              <tr>
                {head.map((c, k) => (
                  <th key={k} className="border border-line bg-background px-2 py-1 font-semibold">
                    {inline(c, k)}
                  </th>
                ))}
              </tr>
            </thead>
            <tbody>
              {rows.map((r, k) => (
                <tr key={k}>
                  {head.map((_, j) => (
                    <td key={j} className="border border-line px-2 py-1 align-top">
                      {inline(r[j] ?? "", j)}
                    </td>
                  ))}
                </tr>
              ))}
            </tbody>
          </table>
        </div>,
      );
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
