// A read-only terminal (xterm.js) showing a run's shell session: each command with its output, colours kept.
// Loaded on demand; it follows the text as steps are added.
import { FitAddon } from "@xterm/addon-fit";
import { Terminal as XTerm } from "@xterm/xterm";
import "@xterm/xterm/css/xterm.css";
import { useEffect, useRef } from "react";

const THEME = { background: "#0b0e14", foreground: "#d4d4d8", cursor: "#0b0e14", selectionBackground: "#3b82f655" };

export default function Terminal({ text }: { text: string }) {
  const host = useRef<HTMLDivElement>(null);
  const term = useRef<XTerm | null>(null);
  const shown = useRef("");
  useEffect(() => {
    if (!host.current) return;
    const t = new XTerm({
      disableStdin: true,
      convertEol: true,
      cursorBlink: false,
      fontFamily: '"JetBrains Mono Variable", "Cascadia Mono", Consolas, monospace',
      fontSize: 12,
      scrollback: 20_000,
      theme: THEME,
    });
    const fit = new FitAddon();
    t.loadAddon(fit);
    t.open(host.current);
    fit.fit();
    const ro = new ResizeObserver(() => fit.fit());
    ro.observe(host.current);
    term.current = t;
    shown.current = "";
    return () => {
      ro.disconnect();
      t.dispose();
      term.current = null;
    };
  }, []);
  useEffect(() => {
    const t = term.current;
    if (!t) return;
    // New steps extend the text: write only what is new; anything else redraws.
    if (text.startsWith(shown.current)) t.write(text.slice(shown.current.length));
    else {
      t.reset();
      t.write(text);
    }
    shown.current = text;
  }, [text]);
  return <div ref={host} className="h-[60vh] min-h-[320px] w-full overflow-hidden rounded-lg border border-zinc-800 bg-[#0b0e14] p-1.5" />;
}
