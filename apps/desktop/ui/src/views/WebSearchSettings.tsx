// Web search in Settings (ADR-163): the order searches go in (the user's keys, the strongest first, then Exa's free
// search last; one out of its limit hands the search to the next), a key added (its API told by the key) or one
// API's key removed. The keys go to the secret store and never come back here.
import { CheckCircle2, Circle, Search, Trash2 } from "lucide-react";
import { useEffect, useState } from "react";
import { Section } from "../components/ui";
import { api, type WebSearchStatus } from "../lib/api";
import { friendly } from "../lib/errors";
import { useApp } from "../lib/store";

/** The API a key belongs to, as Agenium tells it (Tavily `tvly-`, Brave `BSA`, Exa otherwise). */
const apiOf = (key: string) => (key.startsWith("tvly-") ? "Tavily" : key.startsWith("BSA") ? "Brave Search" : "Exa");

export function WebSearchSection() {
  const app = useApp();
  const [st, setSt] = useState<WebSearchStatus | null>(null);
  const [key, setKey] = useState("");
  const [busy, setBusy] = useState(false);
  useEffect(() => {
    api.webSearchStatus().then(setSt).catch(() => undefined);
  }, []);
  const run = (p: Promise<WebSearchStatus>, done: string) => {
    setBusy(true);
    p.then((s) => (setSt(s), setKey(""), app.notify(done)))
      .catch((e) => app.notify(friendly(e, app.t)))
      .finally(() => setBusy(false));
  };
  if (!st) return null;
  return (
    <Section title={app.t("ws.title")}>
      <div className="text-[11px] leading-relaxed text-subtle">{app.t("ws.note")}</div>
      <div className="mt-2 text-xs" data-testid="web-search">
        <div className="label mb-1">{app.t("ws.order")}</div>
        <ol className="space-y-1">
          {st.order.map((s, i) => (
            <li key={`${s.api}-${i}`} className="flex flex-wrap items-center gap-2" data-api={s.api}>
              <span className="w-4 text-right text-subtle">{i + 1}.</span>
              {s.key ? <CheckCircle2 className="h-3.5 w-3.5 text-emerald-500" /> : <Circle className="h-3.5 w-3.5 text-subtle" />}
              <span className="font-medium">{s.api}</span>
              <span className="text-subtle">{s.key ? app.t("ws.yourKey") : app.t("ws.free")}</span>
              {s.key && (
                <button className="btn" disabled={busy} onClick={() => run(api.removeWebSearchKey(s.api), app.t("ws.removed"))}>
                  <Trash2 className="h-3.5 w-3.5" /> {app.t("ws.remove")}
                </button>
              )}
            </li>
          ))}
        </ol>
      </div>
      <form
        className="mt-2 flex flex-wrap items-center gap-2"
        onSubmit={(e) => {
          e.preventDefault();
          const k = key.trim();
          if (k) run(api.setWebSearchKey(k), app.t("ws.saved").replace("{api}", apiOf(k)));
        }}
      >
        <input
          className="input min-w-[18rem] flex-1 text-xs"
          type="password"
          autoComplete="off"
          aria-label={app.t("ws.placeholder")}
          placeholder={app.t("ws.placeholder")}
          value={key}
          onChange={(e) => setKey(e.target.value)}
        />
        <button className="btn" type="submit" disabled={busy || !key.trim()}>
          <Search className="h-3.5 w-3.5" /> {app.t("ws.save")}
        </button>
      </form>
    </Section>
  );
}
