// The earlier chats of one helper (ADR-152): each a session kept in the root, reopened where it stopped. A session is
// called by its draft (the skill's or the team's name) once it has one, else by its first message; the user can name it,
// and that name stays.
import { Check, History, Loader2, Pencil, Trash2, X } from "lucide-react";
import { useEffect, useState } from "react";
import { api, type HelperSession } from "../lib/api";
import { friendly } from "../lib/errors";
import { useApp } from "../lib/store";
import { ago, cn } from "./ui";

/** A name edited in place: Enter keeps it, Escape leaves it; an empty name gives back the session's own title. */
export function RenameInline({ value, onSave, className }: { value: string; onSave: (title: string) => Promise<unknown>; className?: string }) {
  const app = useApp();
  const [editing, setEditing] = useState(false);
  const [text, setText] = useState(value);
  const [busy, setBusy] = useState(false);
  const save = () => {
    setBusy(true);
    onSave(text.trim())
      .then(() => setEditing(false))
      .catch((e) => app.notify(friendly(e, app.t)))
      .finally(() => setBusy(false));
  };
  if (!editing)
    return (
      <span className={cn("group/name inline-flex min-w-0 items-center gap-1", className)}>
        <span className="truncate">{value || app.t("helper.untitled")}</span>
        <button
          className="shrink-0 text-subtle opacity-0 hover:text-accent group-hover/name:opacity-100 focus:opacity-100"
          title={app.t("helper.rename")}
          onClick={(e) => {
            e.stopPropagation();
            setText(value);
            setEditing(true);
          }}
        >
          <Pencil className="h-3 w-3" />
        </button>
      </span>
    );
  return (
    <span className={cn("inline-flex min-w-0 items-center gap-1", className)} onClick={(e) => e.stopPropagation()}>
      <input
        autoFocus
        className="input h-6 min-w-0 flex-1 px-1.5 text-xs"
        value={text}
        maxLength={120}
        placeholder={app.t("helper.renameHint")}
        onChange={(e) => setText(e.target.value)}
        onKeyDown={(e) => {
          if (e.key === "Enter" && !e.nativeEvent.isComposing) save();
          if (e.key === "Escape") {
            e.stopPropagation();
            setEditing(false);
          }
        }}
      />
      <button className="text-emerald-600 dark:text-emerald-400" disabled={busy} title={app.t("common.save")} onClick={save}>
        {busy ? <Loader2 className="h-3.5 w-3.5 animate-spin" /> : <Check className="h-3.5 w-3.5" />}
      </button>
      <button className="text-subtle" title={app.t("common.cancel")} onClick={() => setEditing(false)}>
        <X className="h-3.5 w-3.5" />
      </button>
    </span>
  );
}

export function HelperHistory({
  kind,
  current,
  onOpen,
  onRenamed,
}: {
  kind: "design" | "skill";
  current: string | null;
  onOpen: (s: HelperSession) => void;
  /** The open session was renamed. */
  onRenamed?: (s: HelperSession) => void;
}) {
  const app = useApp();
  const [list, setList] = useState<HelperSession[] | null>(null);
  const [cost, setCost] = useState<Record<string, number>>({});
  const load = () =>
    Promise.all([api.sessions(kind), api.helperCalls(2000).catch(() => [])])
      .then(([s, calls]) => {
        setList(s);
        const c: Record<string, number> = {};
        for (const h of calls as { session?: string | null; usage: { estimated_cost_usd: number } }[]) {
          if (h.session) c[h.session] = (c[h.session] ?? 0) + h.usage.estimated_cost_usd;
        }
        setCost(c);
      })
      .catch((e) => app.notify(friendly(e, app.t)));
  useEffect(() => {
    load();
  }, [kind]);
  const rename = (s: HelperSession, title: string) =>
    api.renameSession(s.id, title).then((r) => {
      setList((l) => (l ?? []).map((x) => (x.id === r.id ? { ...x, title: r.title, named: r.named } : x)));
      if (r.id === current) onRenamed?.(r);
    });
  if (!list) return <Loader2 className="m-4 h-4 w-4 animate-spin text-subtle" />;
  if (!list.length) return <div className="p-3 text-xs text-subtle">{app.t("helper.noHistory")}</div>;
  return (
    <div className="space-y-1.5">
      {list.map((s) => (
        <div key={s.id} className={cn("group flex items-center gap-2 rounded-lg border border-line p-2 text-xs", s.id === current && "border-accent bg-accent/5")}>
          <History className="h-3.5 w-3.5 shrink-0 text-subtle" />
          <div className="min-w-0 flex-1 cursor-pointer text-left" onClick={() => api.session(s.id).then(onOpen, (e) => app.notify(friendly(e, app.t)))}>
            <RenameInline value={s.title} onSave={(t) => rename(s, t)} className="max-w-full font-medium" />
            <div className="text-[11px] text-subtle">
              {ago(s.updated_at)} · {Math.ceil((s.turns?.length ?? 0) / 2)} {app.t("helper.turns")}
              {s.model?.model && ` · ${s.model.model}`}
              {cost[s.id] ? ` · $${cost[s.id].toFixed(4)}` : ""}
            </div>
          </div>
          <span className={cn("chip", s.status === "saved" ? "border-emerald-500/40 text-emerald-600 dark:text-emerald-400" : "border-line-strong text-muted")}>
            {s.status === "saved" ? `${app.t("helper.saved")}${s.result ? `: ${s.result}` : ""}` : app.t("helper.open")}
          </span>
          {s.id !== current && (
            <button className="hidden text-subtle hover:text-rose-500 group-hover:block" title={app.t("common.delete")} onClick={() => api.removeSession(s.id).then(load)}>
              <Trash2 className="h-3.5 w-3.5" />
            </button>
          )}
        </div>
      ))}
    </div>
  );
}
