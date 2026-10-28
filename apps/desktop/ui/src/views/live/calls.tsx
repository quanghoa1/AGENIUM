import { BookOpen, ChevronDown, ChevronRight, CircleCheck, CircleX, GitBranch, Plug, Wrench } from "lucide-react";
import { useState } from "react";
import { Markdown } from "../../components/Markdown";
import type { CallKind } from "../../components/PixelOffice";
import { Mono, cn } from "../../components/ui";
import type { LogEntry } from "../../lib/types";
import { plainArgs, startsSubagent, subagentOf } from "../../lib/subagents";

export const SHELL = new Set(["shell", "Bash", "PowerShell", "run_command"]);

/** Agenium's protocol tools (the team's own moves: tickets, reports, questions, plans). */
const PROTOCOL = new Set([
  "answer_question",
  "approve_permission",
  "ask_question",
  "assign_ticket",
  "cancel_ticket",
  "close_ticket",
  "confirm_link",
  "create_aggregate",
  "create_handoff",
  "create_plan",
  "create_ticket",
  "get_team",
  "post_progress",
  "propose_role",
  "propose_role_change",
  "propose_skill_update",
  "propose_soul_change",
  "propose_spec",
  "request_permission",
  "review_ticket",
  "revise_plan",
  "submit_plan_draft",
  "submit_report",
  "suggest_link",
]);

/** A short view of a tool call's arguments (path, command, pattern…). */
function argPreview(args: any): string {
  if (!args || typeof args !== "object") return "";
  const v = args.command ?? args.cmd ?? args.path ?? args.pattern ?? args.query ?? args.url ?? args.name ?? args.ticket_id ?? Object.values(args)[0];
  const s = typeof v === "string" ? v : JSON.stringify(v ?? "");
  return s.length > 60 ? s.slice(0, 59) + "…" : s;
}

export interface Call {
  kind: CallKind;
  /** Short label: `server :: tool`, the skill name, `bash: <command>`, `read_file: <path>`. */
  label: string;
  name: string;
}

/** Sorts a call into MCP (`mcp__<server>__<tool>`), Skill (`load_skill`), Agenium (protocol) or Tool. */
export function classify(tool: string, args: any): Call {
  const name = tool.replace(/^cli:/, "");
  if (name.startsWith("mcp__")) {
    const [, server, ...rest] = name.split("__");
    return { kind: "mcp", label: `${server} :: ${rest.join("__")}`, name };
  }
  if (name === "load_skill" || name === "Skill") return { kind: "skill", label: String(args?.name ?? args?.skill ?? args?.command ?? "skill"), name };
  if (PROTOCOL.has(name)) return { kind: "agenium", label: `${name}${args?.ticket_id ? ` ${args.ticket_id}` : ""}`, name };
  if (SHELL.has(name)) return { kind: "tool", label: `bash: ${argPreview(args)}`, name };
  return { kind: "tool", label: `${name}: ${argPreview(args)}`, name };
}

const CALL_TONE: Record<CallKind, string> = {
  mcp: "border-purple-500/40 bg-purple-500/10 text-purple-600 dark:text-purple-300",
  skill: "border-teal-500/40 bg-teal-500/10 text-teal-600 dark:text-teal-300",
  tool: "border-line-strong bg-elevated text-muted",
  agenium: "border-blue-500/40 bg-blue-500/10 text-blue-600 dark:text-blue-300",
};

export function CallBadge({ kind }: { kind: CallKind }) {
  const icon = kind === "mcp" ? <Plug className="h-3 w-3" /> : kind === "skill" ? <BookOpen className="h-3 w-3" /> : kind === "agenium" ? <GitBranch className="h-3 w-3" /> : <Wrench className="h-3 w-3" />;
  return (
    <span className={cn("chip shrink-0", CALL_TONE[kind])}>
      {icon}
      {kind === "mcp" ? "MCP" : kind === "skill" ? "Skill" : kind === "agenium" ? "Agenium" : "Tool"}
    </span>
  );
}

/** What the Jev judge said for this run (ADR-75): what it was asked, and its answer when opened. */
export function JudgeRow({ e }: { e: LogEntry }) {
  const [open, setOpen] = useState(false);
  return (
    <div className="rounded-lg border border-fuchsia-500/30">
      <button className="flex w-full items-center gap-2 px-2.5 py-1.5 text-left" onClick={() => setOpen(!open)}>
        {open ? <ChevronDown className="h-3 w-3 shrink-0 text-subtle" /> : <ChevronRight className="h-3 w-3 shrink-0 text-subtle" />}
        <span className="chip shrink-0 border-fuchsia-500/40 bg-fuchsia-500/10 text-fuchsia-600 dark:text-fuchsia-300">Jev</span>
        <span className="truncate font-mono text-[11px]">{e.data.use}</span>
      </button>
      {open && (
        <div className="border-t border-line p-2.5 text-[11px]">
          <Markdown text={String(e.data.text ?? "")} />
        </div>
      )}
    </div>
  );
}

/** One call of the trace: badge, label, duration, ok; arguments and output when opened. */
export function CallRow({ e, open: start = false }: { e: LogEntry; open?: boolean }) {
  const [open, setOpen] = useState(start);
  const c = classify(e.data.tool, plainArgs(e.data.args));
  const ok = e.data.ok !== false;
  // ADR-129: a call a CLI's subagent made is indented and named; the call that starts one says so.
  const sub = subagentOf(e.data.args);
  const starts = startsSubagent(e.data.args);
  return (
    <div className={cn("rounded-lg border", ok ? "border-line" : "border-rose-500/40", sub && "ml-5 border-l-2 border-l-violet-500/60")}>
      <button className="flex w-full items-center gap-2 px-2.5 py-1.5 text-left" onClick={() => setOpen(!open)}>
        {open ? <ChevronDown className="h-3 w-3 shrink-0 text-subtle" /> : <ChevronRight className="h-3 w-3 shrink-0 text-subtle" />}
        <CallBadge kind={c.kind} />
        <span className="truncate font-mono text-[11px]">{c.label}</span>
        {(starts ?? sub) && (
          <span className="truncate rounded border border-violet-500/30 bg-violet-500/10 px-1 text-[10px] text-violet-600 dark:text-violet-300">
            {starts ? `▶ ${starts}` : sub}
          </span>
        )}
        <span className="ml-auto flex shrink-0 items-center gap-1.5">
          {e.data.ms != null && <Mono className="text-subtle">{e.data.ms} ms</Mono>}
          {ok ? <CircleCheck className="h-3.5 w-3.5 text-emerald-500" /> : <CircleX className="h-3.5 w-3.5 text-rose-500" />}
        </span>
      </button>
      {open && (
        <div className="space-y-1.5 border-t border-line p-2.5">
          <div className="label">Args</div>
          <pre className="max-h-40 overflow-auto rounded bg-elevated p-2 font-mono text-[11px]">{JSON.stringify(plainArgs(e.data.args), null, 2)}</pre>
          <div className="label">Out</div>
          <pre className={cn("max-h-56 overflow-auto whitespace-pre-wrap rounded p-2 font-mono text-[11px]", SHELL.has(c.name) ? "bg-[#0a0e14] text-zinc-300" : ok ? "bg-elevated" : "bg-rose-500/10 text-rose-600 dark:text-rose-300")}>
            {String(e.data.result ?? "")}
          </pre>
        </div>
      )}
    </div>
  );
}
