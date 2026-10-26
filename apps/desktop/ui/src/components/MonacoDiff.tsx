// Side-by-side diff of one file (Monaco, bundled with the app: no CDN). Loaded on demand by `DiffView`, so the
// editor (a few MB) is fetched only when a file is opened side by side.
import * as monaco from "monaco-editor/editor/editor.api";
import "monaco-editor/basic-languages/monaco.contribution";
import EditorWorker from "monaco-editor/editor/editor.worker?worker";
import { useEffect, useRef } from "react";

// Syntax colouring only (no language services): the plain editor worker serves every label.
(self as unknown as { MonacoEnvironment: monaco.Environment }).MonacoEnvironment = { getWorker: () => new EditorWorker() };

const EXT: Record<string, string> = {
  rs: "rust", ts: "typescript", tsx: "typescript", js: "javascript", jsx: "javascript", mjs: "javascript", py: "python",
  go: "go", java: "java", kt: "kotlin", c: "c", h: "c", cpp: "cpp", hpp: "cpp", cs: "csharp", rb: "ruby", php: "php",
  swift: "swift", sh: "shell", ps1: "powershell", sql: "sql", md: "markdown", json: "json", toml: "ini", yaml: "yaml",
  yml: "yaml", html: "html", css: "css", scss: "scss", xml: "xml", dockerfile: "dockerfile",
};

function language(path: string): string {
  const name = path.split("/").pop()?.toLowerCase() ?? "";
  if (name === "dockerfile") return "dockerfile";
  return EXT[name.split(".").pop() ?? ""] ?? "plaintext";
}

function dark(): boolean {
  return document.documentElement.classList.contains("dark");
}

export default function MonacoDiff({ path, original, modified }: { path: string; original: string; modified: string }) {
  const host = useRef<HTMLDivElement>(null);
  useEffect(() => {
    if (!host.current) return;
    const lang = language(path);
    const a = monaco.editor.createModel(original, lang);
    const b = monaco.editor.createModel(modified, lang);
    const ed = monaco.editor.createDiffEditor(host.current, {
      readOnly: true,
      originalEditable: false,
      automaticLayout: true,
      renderSideBySide: true,
      minimap: { enabled: false },
      scrollBeyondLastLine: false,
      fontSize: 12,
      fontFamily: '"JetBrains Mono Variable", "Cascadia Mono", Consolas, monospace',
      theme: dark() ? "vs-dark" : "vs",
    });
    ed.setModel({ original: a, modified: b });
    // Follow the app's light/dark switch.
    const mo = new MutationObserver(() => monaco.editor.setTheme(dark() ? "vs-dark" : "vs"));
    mo.observe(document.documentElement, { attributes: true, attributeFilter: ["class"] });
    return () => {
      mo.disconnect();
      ed.dispose();
      a.dispose();
      b.dispose();
    };
  }, [path, original, modified]);
  return <div ref={host} className="h-[60vh] min-h-[320px] w-full overflow-hidden rounded-lg border border-line" />;
}
