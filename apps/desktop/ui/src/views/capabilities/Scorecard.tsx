// A skill on the 12 quality criteria (ADR-152), checked locally with no model call, and its effect on real tickets.
import { AlertTriangle, CheckCircle2, Info, Loader2 } from "lucide-react";
import { useEffect, useState } from "react";
import { cn } from "../../components/ui";
import { api, type Scorecard, type SkillEffect } from "../../lib/api";
import type { Key } from "../../lib/i18n";
import { useApp } from "../../lib/store";

/** The scorecard of a skill as written, fetched again (debounced) when it changes; null while it loads. */
export function useScorecard(s: { name: string; description: string; body: string; origin: string; skip?: string | null } | null): Scorecard | null {
  const [card, setCard] = useState<Scorecard | null>(null);
  const key = s ? JSON.stringify(s) : "";
  useEffect(() => {
    if (!s) return setCard(null);
    let stale = false;
    const t = setTimeout(
      () =>
        api
          .skillScorecard(s.name, s.description, s.body, s.origin, s.skip)
          .then((c) => !stale && setCard(c))
          .catch(() => !stale && setCard(null)),
      300,
    );
    return () => {
      stale = true;
      clearTimeout(t);
    };
  }, [key]);
  return card;
}

export function ScorecardView({ card, className }: { card: Scorecard | null; className?: string }) {
  const app = useApp();
  if (!card) return <Loader2 className="h-4 w-4 animate-spin text-subtle" />;
  return (
    <div className={cn("space-y-1.5", className)}>
      <div className="flex items-center gap-2">
        <span className="label">{app.t("quality.title")}</span>
        <span className={cn("text-[11px]", card.warn ? "text-amber-600 dark:text-amber-400" : "text-emerald-600 dark:text-emerald-400")}>
          {app.t("quality.summary").replace("{ok}", String(card.ok)).replace("{warn}", String(card.warn))}
        </span>
      </div>
      <div className="grid gap-x-4 gap-y-1 text-[11px] md:grid-cols-2">
        {card.criteria.map((c, i) => (
          <div key={c.id} className="flex items-start gap-1.5" title={c.note}>
            {c.status === "ok" ? (
              <CheckCircle2 className="mt-0.5 h-3 w-3 shrink-0 text-emerald-500" />
            ) : c.status === "warn" ? (
              <AlertTriangle className="mt-0.5 h-3 w-3 shrink-0 text-amber-500" />
            ) : (
              <Info className="mt-0.5 h-3 w-3 shrink-0 text-subtle" />
            )}
            <span className="min-w-0">
              <span className={cn(c.status === "warn" && "text-amber-600 dark:text-amber-400")}>
                {i + 1}. {app.t(`quality.c.${c.id}` as Key)}
              </span>
              {c.note && c.status !== "ok" && <span className="text-subtle"> · {c.note}</span>}
            </span>
          </div>
        ))}
      </div>
    </div>
  );
}

/** A library skill's effect on the closed tickets it was loaded in (criterion 12). */
export function EffectBadge({ e }: { e?: SkillEffect }) {
  const app = useApp();
  if (!e) return null;
  const text = app.t("quality.effect").replace("{n}", String(e.tickets)).replace("{r}", String(e.reopened));
  return (
    <span
      className={cn("chip", e.worse ? "border-amber-500/50 text-amber-600 dark:text-amber-400" : "border-line-strong text-subtle")}
      title={e.worse ? app.t("quality.worse") : e.enough ? `${e.others_reopened}/${e.others} ${app.t("quality.without")}` : app.t("quality.notEnough")}
    >
      {e.worse && <AlertTriangle className="h-3 w-3" />} {text}
    </span>
  );
}
