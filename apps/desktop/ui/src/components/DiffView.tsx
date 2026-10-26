// Unified diff viewer (git diff output): one collapsible section per file with +/- counts and coloured lines. With
// `load`, a file can also be opened side by side in Monaco (loaded on demand).
import { ChevronDown, ChevronRight, Columns2, FileDiff, Loader2 } from "lucide-react";
import { Suspense, lazy, useMemo, useState } from "react";
import { cn } from "./ui";

const MonacoDiff = lazy(() => import("./MonacoDiff"));

/** Both sides of one file; `null` = absent on that side. */
export interface FileSides {
  original: string | null;
  modified: string | null;
  binary: boolean;
  too_big: boolean;
}

interface FileDiff_ {
  path: string;
  lines: string[];
  add: number;
  del: number;
}

function parse(diff: string): FileDiff_[] {
  const files: FileDiff_[] = [];
  let cur: FileDiff_ | null = null;
  for (const l of diff.replace(/\r\n/g, "\n").split("\n")) {
    if (l.startsWith("diff --git ")) {
      const m = / b\/(.+)$/.exec(l);
      cur = { path: m ? m[1] : l.slice(11), lines: [], add: 0, del: 0 };
      files.push(cur);
      continue;
    }
    if (!cur) continue;
    if (/^(index |--- |\+\+\+ |new file mode|deleted file mode|similarity |rename )/.test(l)) continue;
    cur.lines.push(l);
    if (l.startsWith("+")) cur.add++;
    else if (l.startsWith("-")) cur.del++;
  }
  return files;
}

export function DiffView({ diff, load }: { diff: string; load?: (path: string) => Promise<FileSides> }) {
  const files = useMemo(() => parse(diff), [diff]);
  const [closed, setClosed] = useState<Record<string, boolean>>({});
  /** Files shown side by side, with their two sides once loaded (or the reason they cannot be). */
  const [sides, setSides] = useState<Record<string, FileSides | "loading" | string>>({});
  const toggleSides = (path: string) => {
    if (sides[path]) {
      const { [path]: _, ...rest } = sides;
      setSides(rest);
      return;
    }
    setSides((s) => ({ ...s, [path]: "loading" }));
    load!(path)
      .then((r) => setSides((s) => ({ ...s, [path]: r.binary ? "binary file" : r.too_big ? "file too large to show" : r })))
      .catch((e) => setSides((s) => ({ ...s, [path]: String(e) })));
  };
  if (!files.length) return <div className="rounded-lg border border-dashed border-line-strong p-6 text-center text-xs text-subtle">No changes.</div>;
  return (
    <div className="space-y-3">
      <div className="text-[11px] text-subtle">
        {files.length} file(s) · <span className="text-emerald-600 dark:text-emerald-400">+{files.reduce((s, f) => s + f.add, 0)}</span>{" "}
        <span className="text-rose-600 dark:text-rose-400">−{files.reduce((s, f) => s + f.del, 0)}</span>
      </div>
      {files.map((f) => (
        <div key={f.path} className="card overflow-hidden">
          <button className="flex w-full items-center gap-2 border-b border-line bg-elevated/60 px-3 py-2 text-left" onClick={() => setClosed({ ...closed, [f.path]: !closed[f.path] })}>
            {closed[f.path] ? <ChevronRight className="h-3.5 w-3.5" /> : <ChevronDown className="h-3.5 w-3.5" />}
            <FileDiff className="h-3.5 w-3.5 text-subtle" />
            <span className="flex-1 truncate font-mono text-[11px]">{f.path}</span>
            <span className="font-mono text-[11px] text-emerald-600 dark:text-emerald-400">+{f.add}</span>
            <span className="font-mono text-[11px] text-rose-600 dark:text-rose-400">−{f.del}</span>
            {load && (
              <span
                role="button"
                tabIndex={0}
                title="Side by side"
                aria-label="Side by side"
                className={cn("rounded p-0.5 text-subtle hover:bg-elevated hover:text-fg", sides[f.path] && "text-accent")}
                onClick={(e) => (e.stopPropagation(), toggleSides(f.path))}
                onKeyDown={(e) => e.key === "Enter" && (e.stopPropagation(), toggleSides(f.path))}
              >
                <Columns2 className="h-3.5 w-3.5" />
              </span>
            )}
          </button>
          {!closed[f.path] && sides[f.path] && (
            <div className="p-2">
              {sides[f.path] === "loading" ? (
                <Loader2 className="h-4 w-4 animate-spin text-subtle" />
              ) : typeof sides[f.path] === "string" ? (
                <div className="text-[11px] text-subtle">{sides[f.path] as string}</div>
              ) : (
                <Suspense fallback={<Loader2 className="h-4 w-4 animate-spin text-subtle" />}>
                  <MonacoDiff path={f.path} original={(sides[f.path] as FileSides).original ?? ""} modified={(sides[f.path] as FileSides).modified ?? ""} />
                </Suspense>
              )}
            </div>
          )}
          {!closed[f.path] && !sides[f.path] && (
            <div className="overflow-x-auto">
              <pre className="min-w-full font-mono text-[11px] leading-[1.55]">
                {f.lines.map((l, i) => (
                  <div
                    key={i}
                    className={cn(
                      "px-3",
                      l.startsWith("+") && "bg-emerald-500/10 text-emerald-700 dark:text-emerald-300",
                      l.startsWith("-") && "bg-rose-500/10 text-rose-700 dark:text-rose-300",
                      l.startsWith("@@") && "bg-blue-500/10 text-blue-600 dark:text-blue-300",
                      l.startsWith("\\") && "text-subtle",
                    )}
                  >
                    {l || " "}
                  </div>
                ))}
              </pre>
            </div>
          )}
        </div>
      ))}
    </div>
  );
}
