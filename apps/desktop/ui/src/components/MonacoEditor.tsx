// A single editable Monaco editor (bundled with the app: no CDN). Loaded on demand (a few MB), used for authoring a
// SKILL.md in the app. Mirrors MonacoDiff's worker/theme setup.
import * as monaco from "monaco-editor/editor/editor.api";
import "monaco-editor/basic-languages/monaco.contribution";
import EditorWorker from "monaco-editor/editor/editor.worker?worker";
import { useEffect, useRef } from "react";

(self as unknown as { MonacoEnvironment: monaco.Environment }).MonacoEnvironment = { getWorker: () => new EditorWorker() };

function dark(): boolean {
  return document.documentElement.classList.contains("dark");
}

export default function MonacoEditor({ value, language = "markdown", onChange, className }: { value: string; language?: string; onChange: (v: string) => void; className?: string }) {
  const host = useRef<HTMLDivElement>(null);
  // Keep the latest onChange without re-creating the editor on every keystroke.
  const cb = useRef(onChange);
  cb.current = onChange;
  useEffect(() => {
    if (!host.current) return;
    const model = monaco.editor.createModel(value, language);
    const ed = monaco.editor.create(host.current, {
      model,
      automaticLayout: true,
      minimap: { enabled: false },
      scrollBeyondLastLine: false,
      wordWrap: "on",
      fontSize: 12,
      fontFamily: '"JetBrains Mono Variable", "Cascadia Mono", Consolas, monospace',
      theme: dark() ? "vs-dark" : "vs",
    });
    const sub = model.onDidChangeContent(() => cb.current(model.getValue()));
    const mo = new MutationObserver(() => monaco.editor.setTheme(dark() ? "vs-dark" : "vs"));
    mo.observe(document.documentElement, { attributes: true, attributeFilter: ["class"] });
    return () => {
      sub.dispose();
      mo.disconnect();
      ed.dispose();
      model.dispose();
    };
    // The editor is created once; `value` seeds it and is then owned by the model (controlled through onChange).
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [language]);
  // A given height (a flex child that fills its column) or the default one.
  return <div ref={host} className={`${className ?? "h-[56vh] min-h-[320px]"} w-full overflow-hidden rounded-lg border border-line`} />;
}
