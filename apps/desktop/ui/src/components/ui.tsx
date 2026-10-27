// Shared UI primitives in the mockup's visual language: mono micro-labels, rounded cards, semantic colours
// (blue = working, emerald = done/ok, amber = waiting/approval, rose = problem, cyan = team, fuchsia = solo,
// purple = lead/review).
import { clsx } from "clsx";
import { twMerge } from "tailwind-merge";
import {
  AlertTriangle,
  Bell,
  Bot,
  Brain,
  CheckCheck,
  CheckCircle2,
  CircleDashed,
  CircleDot,
  Clock,
  Cpu,
  Loader2,
  MessagesSquare,
  Pause,
  PauseCircle,
  Terminal,
  WifiOff,
  XCircle,
} from "lucide-react";
import type { ReactNode } from "react";
import { isFree, priceOf, usePrices } from "../lib/prices";
import { useApp } from "../lib/store";
import type { AgentStatus, BackendKind, ModelBinding, TicketStatus } from "../lib/types";

export function cn(...v: (string | false | null | undefined)[]) {
  return twMerge(clsx(v));
}

/** The interface language for `ago` (set by the store when the language changes). */
let agoLang: "vi" | "en" = "vi";
export function setAgoLang(l: "vi" | "en") {
  agoLang = l;
}

/** How long ago `iso` was, in the interface language: "11 phút trước" / "11m ago". */
export function ago(iso?: string | null): string {
  if (!iso) return "–";
  const s = Math.max(0, (Date.now() - new Date(iso).getTime()) / 1000);
  const [n, unit] =
    s < 60 ? [Math.round(s), 0] : s < 3600 ? [Math.round(s / 60), 1] : s < 86400 ? [Math.round(s / 3600), 2] : [Math.round(s / 86400), 3];
  if (agoLang === "vi") return s < 10 ? "vừa xong" : `${n} ${["giây", "phút", "giờ", "ngày"][unit]} trước`;
  return s < 10 ? "just now" : `${n}${["s", "m", "h", "d"][unit]} ago`;
}

export function tokens(n: number): string {
  if (n >= 1_000_000) return (n / 1_000_000).toFixed(1) + "M";
  if (n >= 1000) return (n / 1000).toFixed(1) + "k";
  return String(n);
}

const agentTone: Record<AgentStatus, [string, ReactNode]> = {
  Idle: ["border-emerald-500/30 bg-emerald-500/10 text-emerald-600 dark:text-emerald-300", <CircleDot className="h-3 w-3" />],
  Working: ["border-blue-500/30 bg-blue-500/10 text-blue-600 dark:text-blue-300", <Loader2 className="h-3 w-3 animate-spin" />],
  Waiting: ["border-amber-500/30 bg-amber-500/10 text-amber-600 dark:text-amber-300", <PauseCircle className="h-3 w-3" />],
  QuotaExhausted: ["border-rose-500/40 bg-rose-500/10 text-rose-600 dark:text-rose-300", <AlertTriangle className="h-3 w-3" />],
  Offline: ["border-zinc-500/40 bg-zinc-500/10 text-zinc-500 dark:text-zinc-400", <WifiOff className="h-3 w-3" />],
};

export function AgentStatusBadge({ status }: { status: AgentStatus }) {
  const [tone, icon] = agentTone[status] ?? agentTone.Offline;
  return (
    <span className={cn("inline-flex items-center gap-1 rounded-full border px-2 py-0.5 text-[11px] font-medium", tone)}>
      {icon}
      {status === "QuotaExhausted" ? "Quota exhausted" : status}
    </span>
  );
}

const ticketTone: Record<TicketStatus, string> = {
  New: "border-zinc-500/40 bg-zinc-500/10 text-zinc-600 dark:text-zinc-300",
  Assigned: "border-cyan-500/30 bg-cyan-500/10 text-cyan-600 dark:text-cyan-300",
  InProgress: "border-blue-500/30 bg-blue-500/10 text-blue-600 dark:text-blue-300",
  Pending: "border-amber-500/30 bg-amber-500/10 text-amber-600 dark:text-amber-300",
  Resolved: "border-purple-500/30 bg-purple-500/10 text-purple-600 dark:text-purple-300",
  Reopened: "border-rose-500/30 bg-rose-500/10 text-rose-600 dark:text-rose-300",
  Closed: "border-emerald-500/30 bg-emerald-500/10 text-emerald-600 dark:text-emerald-300",
  Cancelled: "border-zinc-500/30 bg-zinc-500/5 text-zinc-500",
};

export type RunState = "Agreeing" | "Running" | "Paused" | "Done";

/** Colour of a project's run state, like a ticket's status: running glows green, paused amber, done blue. */
export const runTone: Record<RunState, string> = {
  Agreeing: "border-cyan-500/30 bg-cyan-500/10 text-cyan-600 dark:text-cyan-300",
  Running: "border-emerald-500/40 bg-emerald-500/10 text-emerald-600 dark:text-emerald-300",
  Paused: "border-amber-500/40 bg-amber-500/10 text-amber-600 dark:text-amber-300",
  Done: "border-blue-500/40 bg-blue-500/10 text-blue-600 dark:text-blue-300",
};

/** The glow around a project's card: running, or waiting for the user. */
export function projectGlow(state: RunState | undefined, needsYou: number): string {
  if (needsYou > 0) return "border-amber-500/50 shadow-[0_0_22px_-6px_rgba(245,158,11,0.55)]";
  if (state === "Running") return "border-emerald-500/50 shadow-[0_0_22px_-6px_rgba(16,185,129,0.55)]";
  return "";
}

export function RunStateIcon({ state }: { state: RunState }) {
  if (state === "Running")
    return (
      <span className="relative inline-flex h-2 w-2">
        <span className="absolute inset-0 animate-ping rounded-full bg-emerald-500/60" />
        <span className="relative h-2 w-2 rounded-full bg-emerald-500" />
      </span>
    );
  if (state === "Paused") return <Pause className="h-3.5 w-3.5" />;
  if (state === "Done") return <CheckCheck className="h-3.5 w-3.5" />;
  return <MessagesSquare className="h-3.5 w-3.5" />;
}

/** A project's state at a glance: its run state, and how many things wait for the user. */
export function ProjectStateChips({ state, needsYou, cancelled }: { state?: RunState; needsYou: number; cancelled?: boolean }) {
  const app = useApp();
  return (
    <span className="flex shrink-0 items-center gap-1.5">
      {needsYou > 0 && (
        <span className="chip border-amber-500/50 bg-amber-500/15 font-semibold text-amber-600 dark:text-amber-300" title={app.t("nav.needs")}>
          <Bell className="h-3 w-3 animate-pulse" /> {needsYou} {app.t("needs.waiting")}
        </span>
      )}
      {cancelled ? (
        <span className="chip border-zinc-500/30 bg-zinc-500/5 text-zinc-500">{app.t("projects.status.Cancelled")}</span>
      ) : state ? (
        <span className={cn("chip font-semibold", runTone[state])} title={app.t(`run.${state}.hint`)}>
          <RunStateIcon state={state} /> {app.t(`run.${state}`)}
        </span>
      ) : null}
    </span>
  );
}

export function TicketStatusBadge({ status }: { status: TicketStatus }) {
  return (
    <span className={cn("chip", ticketTone[status])}>
      <span className="h-1.5 w-1.5 rounded-full bg-current" />
      {status === "InProgress" ? "IN PROGRESS" : status.toUpperCase()}
    </span>
  );
}

export function PriorityBadge({ p }: { p: number }) {
  const tone = [
    "border-rose-500/40 bg-rose-500/10 text-rose-600 dark:text-rose-300",
    "border-amber-500/40 bg-amber-500/10 text-amber-600 dark:text-amber-300",
    "border-blue-500/30 bg-blue-500/10 text-blue-600 dark:text-blue-300",
    "border-zinc-500/30 bg-zinc-500/10 text-zinc-500",
  ][Math.min(3, Math.max(0, p))];
  return <span className={cn("chip", tone)}>P{p}</span>;
}

/** A model whose catalogue price is 0 for input and output. */
export function FreeTag({ className }: { className?: string }) {
  return <span className={cn("chip border-emerald-500/40 bg-emerald-500/15 px-1.5 py-0 text-[11px] font-bold text-emerald-600 dark:text-emerald-300", className)}>FREE</span>;
}

export function RuntimeBadge({ backend, model }: { backend: BackendKind; model: ModelBinding }) {
  const cli = backend === "Cli";
  const prices = usePrices(useApp().tick);
  return (
    <span className="chip border-line-strong bg-elevated text-muted">
      {cli ? <Terminal className="h-3 w-3" /> : <Cpu className="h-3 w-3" />}
      {cli ? `CLI:${model.provider}` : model.provider} · {model.model || useApp().t("model.default")}
      <span className="text-subtle">· {model.effort}</span>
      {!cli && isFree(priceOf(prices, model.provider, model.model)) && <FreeTag />}
    </span>
  );
}

export function OutcomeBadge({ outcome }: { outcome?: string | null }) {
  if (!outcome)
    return (
      <span className="chip border-blue-500/30 bg-blue-500/10 text-blue-600 dark:text-blue-300">
        <Loader2 className="h-3 w-3 animate-spin" /> RUNNING
      </span>
    );
  const o = outcome.split(":")[0];
  const [tone, icon] =
    o === "Completed"
      ? ["border-emerald-500/30 bg-emerald-500/10 text-emerald-600 dark:text-emerald-300", <CheckCircle2 className="h-3 w-3" />]
      : o === "Parked" || o === "Checkpoint"
        ? ["border-amber-500/30 bg-amber-500/10 text-amber-600 dark:text-amber-300", <Clock className="h-3 w-3" />]
        : o === "Cancelled"
          ? ["border-zinc-500/30 bg-zinc-500/10 text-zinc-500", <CircleDashed className="h-3 w-3" />]
          : o === "Memory"
            ? ["border-violet-500/30 bg-violet-500/10 text-violet-600 dark:text-violet-300", <Brain className="h-3 w-3" />]
            : ["border-rose-500/40 bg-rose-500/10 text-rose-600 dark:text-rose-300", <XCircle className="h-3 w-3" />];
  return (
    <span className={cn("chip", tone)} title={outcome}>
      {icon}
      {o.toUpperCase()}
    </span>
  );
}

export function RoleChip({ lead, planner, solo }: { lead?: boolean; planner?: boolean; solo?: boolean }) {
  if (solo) return <span className="chip border-fuchsia-500/30 bg-fuchsia-500/10 text-fuchsia-600 dark:text-fuchsia-300">SOLO</span>;
  if (lead) return <span className="chip border-purple-500/30 bg-purple-500/10 text-purple-600 dark:text-purple-300">LEAD</span>;
  if (planner) return <span className="chip border-cyan-500/30 bg-cyan-500/10 text-cyan-600 dark:text-cyan-300">PLANNER</span>;
  return <span className="chip border-line-strong bg-elevated text-muted">WORKER</span>;
}

/** Two or three letters for an agent's name: `sc-backend` -> BA, `lead-1` -> L1 (a numbered name keeps its number). */
export function initials(name: string): string {
  const parts = name.split(/[-_\s.]+/).filter(Boolean);
  if (parts.length === 0) return "";
  const last = parts[parts.length - 1];
  if (parts.length > 1 && /^\d+$/.test(last)) return (parts[0][0] + last).slice(0, 3).toUpperCase();
  return last.slice(0, 2).toUpperCase();
}

/** `CLI:codex/gpt-6` or `deepseek/v4`; a CLI left on its own default model says so instead of ending in a slash. */
export function modelLabel(backend: string, model: { provider: string; model: string }, defaultWord: string): string {
  const who = backend === "Cli" || backend === "cli" ? `CLI:${model.provider}` : model.provider;
  return model.model ? `${who}/${model.model}` : `${who} (${defaultWord})`;
}

export function Avatar({ name, className }: { name: string; className?: string }) {
  const hues = ["from-blue-600 to-cyan-500", "from-purple-600 to-fuchsia-500", "from-emerald-600 to-teal-500", "from-amber-500 to-orange-500", "from-rose-600 to-pink-500"];
  const h = hues[[...name].reduce((a, c) => a + c.charCodeAt(0), 0) % hues.length];
  return (
    <div className={cn("flex h-8 w-8 shrink-0 items-center justify-center rounded-lg bg-gradient-to-br font-mono text-xs font-bold text-white", h, className)}>
      {initials(name) || <Bot className="h-4 w-4" />}
    </div>
  );
}

export function PageHeader({ title, sub, children }: { title: string; sub?: string; children?: ReactNode }) {
  return (
    <div className="mb-5 flex flex-wrap items-end justify-between gap-3">
      <div>
        <h1 className="text-xl font-semibold tracking-tight">{title}</h1>
        {sub && <p className="mt-1 text-[13px] text-muted">{sub}</p>}
      </div>
      <div className="flex flex-wrap items-center gap-2">{children}</div>
    </div>
  );
}

export function Section({ title, right, children, className }: { title: ReactNode; right?: ReactNode; children: ReactNode; className?: string }) {
  return (
    <section className={cn("card flex min-h-0 flex-col", className)}>
      <div className="flex items-center justify-between border-b border-line px-4 py-2.5">
        <div className="label">{title}</div>
        {right}
      </div>
      <div className="min-h-0 flex-1 p-4">{children}</div>
    </section>
  );
}

export function Metric({ label, value, sub, tone = "neutral", onClick }: { label: string; value: ReactNode; sub?: ReactNode; tone?: "neutral" | "blue" | "amber" | "rose" | "emerald"; onClick?: () => void }) {
  const toneCls = {
    neutral: "",
    blue: "text-blue-600 dark:text-blue-400",
    amber: "text-amber-600 dark:text-amber-400",
    rose: "text-rose-600 dark:text-rose-400",
    emerald: "text-emerald-600 dark:text-emerald-400",
  }[tone];
  return (
    <button onClick={onClick} disabled={!onClick} className="card flex flex-col justify-start p-4 text-left transition-colors enabled:hover:border-accent/40">
      <div className="label">{label}</div>
      <div className={cn("mt-2 font-mono text-2xl font-semibold", toneCls)}>{value}</div>
      {sub && <div className="mt-1 text-[11px] text-muted">{sub}</div>}
    </button>
  );
}

export function Empty({ children }: { children: ReactNode }) {
  return <div className="rounded-lg border border-dashed border-line-strong p-6 text-center text-xs text-subtle">{children}</div>;
}

export function Mono({ children, className }: { children: ReactNode; className?: string }) {
  return <span className={cn("font-mono text-[11px]", className)}>{children}</span>;
}


/** Prompt-cache hit of an API run or agent: the share of input tokens the provider served from its cache (ADR-113).
 *  Not shown for vendor CLIs, which cache their own prompt. */
export function CacheHit({ value, title }: { value?: number | null; title?: string }) {
  if (value === undefined || value === null) return null;
  const pct = Math.round(value * 100);
  return (
    <span
      title={title ?? "Prompt cache: share of input tokens served from the provider's cache"}
      className={cn(
        "chip font-mono",
        pct >= 80 ? "border-emerald-500/30 text-emerald-600 dark:text-emerald-400" : pct >= 50 ? "border-amber-500/30 text-amber-600 dark:text-amber-400" : "border-rose-500/30 text-rose-600 dark:text-rose-400",
      )}
    >
      cache {pct}%
    </span>
  );
}
